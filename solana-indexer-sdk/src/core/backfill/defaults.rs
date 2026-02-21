use crate::config::BackfillConfig;
use crate::core::execution::fetcher::Fetcher;
use crate::storage::StorageBackend;
use crate::types::backfill_traits::{
    BackfillContext, BackfillProgress, BackfillRange, BackfillStrategy, BackfillTrigger,
    FinalizedBlockTracker, ReorgEvent, ReorgHandler,
};
use crate::utils::error::Result;
use async_trait::async_trait;

/// Default backfill strategy: backfill from earliest to latest.
pub struct DefaultBackfillStrategy {
    pub start_slot: Option<u64>,
    pub end_slot: Option<u64>,
    pub batch_size: usize,
    pub concurrency: usize,
}

impl Default for DefaultBackfillStrategy {
    fn default() -> Self {
        Self {
            start_slot: None,
            end_slot: None,
            batch_size: 100,
            concurrency: 50,
        }
    }
}

#[async_trait]
impl BackfillStrategy for DefaultBackfillStrategy {
    async fn get_slot_range(
        &self,
        _storage: &dyn StorageBackend,
    ) -> Result<(Option<u64>, Option<u64>)> {
        Ok((self.start_slot, self.end_slot))
    }

    fn batch_size(&self) -> usize {
        self.batch_size
    }

    fn concurrency(&self) -> usize {
        self.concurrency
    }
}

/// Default reorg handler using block hash comparison.
pub struct DefaultReorgHandler;

#[async_trait]
impl ReorgHandler for DefaultReorgHandler {
    async fn detect_reorg(
        &self,
        slot: u64,
        storage: &dyn StorageBackend,
        fetcher: &Fetcher,
    ) -> Result<Option<ReorgEvent>> {
        // Get the stored block hash for the slot
        let stored_hash = match storage.get_block_hash(slot).await? {
            Some(h) => h,
            None => return Ok(None), // We haven't processed this slot, so no reorg to detect for us
        };

        // Fetch the current block hash from the canonical chain.
        // We use "finalized" commitment to ensure we are comparing against the immutable truth.
        // If the stored hash differs from the finalized hash, a reorg occurred.
        let current_block = fetcher.fetch_block(slot).await?;
        let current_hash = current_block.blockhash;

        if stored_hash != current_hash {
            Ok(Some(ReorgEvent {
                slot,
                previous_hash: stored_hash,
                new_hash: current_hash,
            }))
        } else {
            Ok(None)
        }
    }

    async fn handle_reorg(&self, event: ReorgEvent, storage: &dyn StorageBackend) -> Result<()> {
        tracing::warn!(
            "Reorg detected at slot {}: {} -> {}",
            event.slot,
            event.previous_hash,
            event.new_hash
        );

        // Rollback the slot in storage
        storage.rollback_slot(event.slot).await?;

        // Note: The indexer loop should naturally pick this up again if we reset the cursor
        // or if we simply delete the tentative data.

        Ok(())
    }
}

/// Default finalized block tracker using commitment levels.
pub struct DefaultFinalizedBlockTracker;

#[async_trait]
impl FinalizedBlockTracker for DefaultFinalizedBlockTracker {
    async fn is_finalized(&self, slot: u64, fetcher: &Fetcher) -> Result<bool> {
        // Fetch detailed block info to check finalization status logic or rely on get_slot
        // A simpler way: get the latest finalized slot and compare
        let finalized_slot = fetcher.get_latest_finalized_slot().await?;
        Ok(slot <= finalized_slot)
    }

    async fn get_latest_finalized_slot(&self, fetcher: &Fetcher) -> Result<u64> {
        fetcher.get_latest_finalized_slot().await
    }

    async fn mark_finalized(
        &self,
        slot: u64,
        block_hash: &str,
        storage: &dyn StorageBackend,
    ) -> Result<()> {
        tracing::debug!(
            "Marking slot {} as finalized with hash {}",
            slot,
            block_hash
        );
        storage.mark_finalized(slot, block_hash).await
    }
}

/// Default progress tracker using storage.
pub struct DefaultBackfillProgress;

#[async_trait]
impl BackfillProgress for DefaultBackfillProgress {
    async fn save_progress(&self, slot: u64, storage: &dyn StorageBackend) -> Result<()> {
        storage.save_backfill_progress(slot).await
    }

    async fn load_progress(&self, storage: &dyn StorageBackend) -> Result<Option<u64>> {
        storage.load_backfill_progress().await
    }

    async fn mark_complete(&self, storage: &dyn StorageBackend) -> Result<()> {
        storage.mark_backfill_complete().await
    }
}

/// Default backfill trigger that uses configuration-based logic.
///
/// This trigger implements a simple policy:
/// - Backfills if lag exceeds `desired_lag_slots` threshold
/// - Respects `max_depth` limit
/// - Uses `start_slot` and `end_slot` from config if provided
pub struct DefaultBackfillTrigger {
    config: BackfillConfig,
}

impl DefaultBackfillTrigger {
    /// Creates a new default backfill trigger.
    pub fn new(config: BackfillConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl BackfillTrigger for DefaultBackfillTrigger {
    async fn next_range(
        &self,
        ctx: &BackfillContext,
        _storage: &dyn StorageBackend,
    ) -> Result<Option<BackfillRange>> {
        // Check if we should backfill based on lag threshold
        if !ctx.should_backfill_by_lag() {
            return Ok(None);
        }

        // Determine start slot
        let start_slot = if let Some(config_start) = self.config.start_slot {
            // Use config start, but don't go before last backfilled slot
            if let Some(last) = ctx.last_backfilled_slot {
                config_start.min(last + 1)
            } else {
                config_start
            }
        } else if let Some(last) = ctx.last_backfilled_slot {
            // Resume from last backfilled slot
            last + 1
        } else {
            // Start from genesis (slot 0)
            0
        };

        // Determine end slot
        let mut end_slot = if let Some(config_end) = self.config.end_slot {
            config_end
        } else {
            // Use latest finalized, but respect max_depth
            let latest = ctx.latest_finalized_slot;
            if let Some(max_depth) = ctx.max_depth {
                latest.saturating_sub(max_depth).max(start_slot)
            } else {
                latest
            }
        };

        // Ensure end >= start
        if end_slot < start_slot {
            return Ok(None);
        }

        // Apply max_depth constraint if set
        if let Some(max_depth) = ctx.max_depth {
            let max_allowed_start = ctx.latest_finalized_slot.saturating_sub(max_depth);
            if start_slot < max_allowed_start {
                // Adjust range to respect max_depth
                end_slot = end_slot.min(ctx.latest_finalized_slot);
                if end_slot < max_allowed_start {
                    return Ok(None);
                }
            }
        }

        // Limit range size to avoid processing too much at once
        // Process in chunks of up to 10k slots
        let chunk_size = 10_000;
        if end_slot - start_slot > chunk_size {
            end_slot = start_slot + chunk_size;
        }

        Ok(Some(BackfillRange::new(start_slot, end_slot)))
    }
}
