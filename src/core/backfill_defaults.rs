use crate::core::fetcher::Fetcher;
use crate::storage::StorageBackend;
use crate::types::backfill_traits::{
    BackfillProgress, BackfillStrategy, FinalizedBlockTracker, ReorgEvent, ReorgHandler,
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

        // Fetch the current block hash from the chain (finalized)
        // We use finalized commitment to be sure
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
        log::warn!(
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
        log::debug!(
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
