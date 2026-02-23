//! Backfill Manager for continuous historical data indexing.
//!
//! The `BackfillManager` runs as a background task alongside the live indexer,
//! continuously checking for gaps in historical data and scheduling backfill ranges
//! based on developer-defined triggers.

use crate::config::SolanaIndexerConfig;
use crate::core::backfill::engine::BackfillEngine;
use crate::core::decoding::Decoder;
use crate::core::execution::fetcher::Fetcher;
use crate::storage::StorageBackend;
use crate::types::backfill_traits::{
    BackfillContext, BackfillHandlerRegistry, BackfillProgress, BackfillRange, BackfillStrategy,
    BackfillTrigger, FinalizedBlockTracker, ReorgHandler,
};
use crate::utils::error::Result;
use crate::utils::logging::{log, log_error, LogLevel};
use std::sync::Arc;
use tokio::time::{interval, Duration};

/// Manages continuous backfill operations alongside the live indexer.
///
/// The `BackfillManager` periodically checks for gaps in historical data
/// using a `BackfillTrigger` and schedules backfill ranges to be processed
/// by the `BackfillEngine`.
pub struct BackfillManager {
    config: SolanaIndexerConfig,
    fetcher: Arc<Fetcher>,
    decoder: Arc<Decoder>,
    storage: Arc<dyn StorageBackend>,
    strategy: Arc<dyn BackfillStrategy>,
    reorg_handler: Arc<dyn ReorgHandler>,
    finalized_tracker: Arc<dyn FinalizedBlockTracker>,
    progress_tracker: Arc<dyn BackfillProgress>,
    trigger: Arc<dyn BackfillTrigger>,
    backfill_handlers: Arc<BackfillHandlerRegistry>,
    cancellation_token: tokio_util::sync::CancellationToken,
    // References to registries needed by BackfillEngine
    decoder_registry: Arc<crate::core::registry::DecoderRegistry>,
    log_decoder_registry: Arc<crate::core::registry::logs::LogDecoderRegistry>,
    account_decoder_registry: Arc<crate::core::registry::account::AccountDecoderRegistry>,
}

impl BackfillManager {
    /// Creates a new backfill manager.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: SolanaIndexerConfig,
        fetcher: Arc<Fetcher>,
        decoder: Arc<Decoder>,
        storage: Arc<dyn StorageBackend>,
        strategy: Arc<dyn BackfillStrategy>,
        reorg_handler: Arc<dyn ReorgHandler>,
        finalized_tracker: Arc<dyn FinalizedBlockTracker>,
        progress_tracker: Arc<dyn BackfillProgress>,
        trigger: Arc<dyn BackfillTrigger>,
        backfill_handlers: Arc<BackfillHandlerRegistry>,
        cancellation_token: tokio_util::sync::CancellationToken,
        decoder_registry: Arc<crate::core::registry::DecoderRegistry>,
        log_decoder_registry: Arc<crate::core::registry::logs::LogDecoderRegistry>,
        account_decoder_registry: Arc<crate::core::registry::account::AccountDecoderRegistry>,
    ) -> Self {
        Self {
            config,
            fetcher,
            decoder,
            storage,
            strategy,
            reorg_handler,
            finalized_tracker,
            progress_tracker,
            trigger,
            backfill_handlers,
            cancellation_token,
            decoder_registry,
            log_decoder_registry,
            account_decoder_registry,
        }
    }

    /// Runs the backfill manager loop.
    ///
    /// This method runs indefinitely, periodically checking for backfill ranges
    /// and processing them until cancellation is requested.
    #[tracing::instrument(skip_all)]
    pub async fn run(self) -> Result<()> {
        log(LogLevel::Info, "Starting BackfillManager...");

        // Initialize schemas for all backfill handlers
        self.backfill_handlers
            .initialize_schemas(self.storage.pool())
            .await?;

        let poll_interval = Duration::from_secs(self.config.backfill.poll_interval_secs);
        let mut interval_timer = interval(poll_interval);

        // Initial tick to start immediately
        interval_timer.tick().await;

        loop {
            tokio::select! {
                _ = self.cancellation_token.cancelled() => {
                    log(LogLevel::Info, "BackfillManager: Cancellation requested, shutting down...");
                    break;
                }
                _ = interval_timer.tick() => {
                    match self.check_and_process_range().await {
                        Ok(processed) => {
                            if processed {
                                log(LogLevel::Info, "BackfillManager: Completed a backfill range");
                            }
                        }
                        Err(e) => {
                            log_error("BackfillManager: Error processing range", &e.to_string());
                            // Continue running - don't exit on individual range errors
                        }
                    }
                }
            }
        }

        log(LogLevel::Success, "BackfillManager: Shutdown complete");
        Ok(())
    }

    /// Checks for a backfill range and processes it if available.
    #[tracing::instrument(skip_all)]
    async fn check_and_process_range(&self) -> Result<bool> {
        // Build context for trigger decision
        let latest_finalized = self
            .finalized_tracker
            .get_latest_finalized_slot(&self.fetcher)
            .await?;

        let last_backfilled = self
            .progress_tracker
            .load_progress(self.storage.as_ref())
            .await?;

        let ctx = BackfillContext {
            latest_finalized_slot: latest_finalized,
            last_backfilled_slot: last_backfilled,
            max_depth: self.config.backfill.max_depth,
            desired_lag_slots: self.config.backfill.desired_lag_slots,
        };

        // Ask trigger for next range
        let range = match self.trigger.next_range(&ctx, self.storage.as_ref()).await? {
            Some(r) => r,
            None => {
                // No work to do
                return Ok(false);
            }
        };

        log(
            LogLevel::Info,
            &format!(
                "BackfillManager: Processing range [{}, {}] ({} slots)",
                range.start_slot,
                range.end_slot,
                range.len()
            ),
        );

        // Process the range
        self.process_range(range).await?;

        Ok(true)
    }

    /// Processes a specific backfill range.
    #[tracing::instrument(skip_all, fields(start_slot = range.start_slot, end_slot = range.end_slot))]
    async fn process_range(&self, range: BackfillRange) -> Result<()> {
        // Create a temporary strategy that uses the range
        let range_strategy = Arc::new(RangeBackfillStrategy {
            range,
            base_strategy: self.strategy.clone(),
        });

        // Create backfill engine for this range
        let engine = BackfillEngine::new(
            self.config.clone(),
            self.fetcher.clone(),
            self.decoder.clone(),
            self.decoder_registry.clone(),
            self.log_decoder_registry.clone(),
            self.account_decoder_registry.clone(),
            // Use empty handler registry for live events (we use backfill handlers)
            Arc::new(crate::types::traits::HandlerRegistry::new()),
            self.storage.clone(),
            range_strategy,
            self.reorg_handler.clone(),
            self.finalized_tracker.clone(),
            self.progress_tracker.clone(),
            self.cancellation_token.clone(),
            self.backfill_handlers.clone(),
        );

        // Run the engine for this range
        engine.start_range(range).await?;

        // Notify handlers that range is complete
        self.backfill_handlers
            .notify_range_complete(&range, self.storage.pool())
            .await?;

        Ok(())
    }
}

/// Temporary strategy adapter that uses a specific range.
struct RangeBackfillStrategy {
    range: BackfillRange,
    base_strategy: Arc<dyn BackfillStrategy>,
}

#[async_trait::async_trait]
impl BackfillStrategy for RangeBackfillStrategy {
    async fn get_slot_range(
        &self,
        _storage: &dyn StorageBackend,
    ) -> Result<(Option<u64>, Option<u64>)> {
        Ok((Some(self.range.start_slot), Some(self.range.end_slot)))
    }

    fn batch_size(&self) -> usize {
        self.base_strategy.batch_size()
    }

    fn concurrency(&self) -> usize {
        self.base_strategy.concurrency()
    }
}
