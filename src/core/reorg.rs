use crate::core::fetcher::Fetcher;
use crate::storage::StorageBackend;
use crate::types::metadata::TxMetadata;
use crate::types::traits::HandlerRegistry;
use crate::utils::error::Result;
use solana_sdk::commitment_config::CommitmentConfig;
use std::sync::Arc;
use tokio::time::{Duration, sleep};
use tokio_util::sync::CancellationToken;

/// Monitors block finalization and handles reorgs.
pub struct FinalityMonitor {
    fetcher: Arc<Fetcher>,
    storage: Arc<dyn StorageBackend>,
    handler_registry: Arc<HandlerRegistry>,
    cancellation_token: CancellationToken,
}

impl FinalityMonitor {
    /// Creates a new `FinalityMonitor`.
    pub fn new(
        fetcher: Arc<Fetcher>,
        storage: Arc<dyn StorageBackend>,
        handler_registry: Arc<HandlerRegistry>,
        cancellation_token: CancellationToken,
    ) -> Self {
        Self {
            fetcher,
            storage,
            handler_registry,
            cancellation_token,
        }
    }

    /// Starts the finalization monitor loop.
    pub async fn start(self) -> Result<()> {
        log::info!("Starting FinalityMonitor...");

        loop {
            tokio::select! {
                _ = self.cancellation_token.cancelled() => {
                    log::info!("FinalityMonitor stopping...");
                    break;
                }
                _ = sleep(Duration::from_secs(10)) => {
                    if let Err(e) = self.process_cycle().await {
                        log::error!("FinalityMonitor error: {}", e);
                    }
                }
            }
        }

        log::info!("FinalityMonitor stopped.");
        Ok(())
    }

    async fn process_cycle(&self) -> Result<()> {
        // 1. Get latest finalized slot from RPC
        let finalized_slot = self.fetcher.get_latest_finalized_slot().await?;

        // 2. Get tentative slots <= finalized_slot from storage
        let tentative_slots = self.storage.get_tentative_slots_le(finalized_slot).await?;

        for slot in tentative_slots {
            // 3. Fetch block with finalized commitment
            match self
                .fetcher
                .fetch_block_with_commitment(slot, CommitmentConfig::finalized())
                .await
            {
                Ok(block) => {
                    // 4. Compare hash
                    let stored_hash = self.storage.get_block_hash(slot).await?;
                    if let Some(stored) = stored_hash {
                        if stored != block.blockhash {
                            log::warn!(
                                "REORG DETECTED at slot {}: stored hash {} != finalized hash {}",
                                slot,
                                stored,
                                block.blockhash
                            );

                            // REORG!
                            // Identify affected transactions
                            let affected_sigs =
                                self.storage.get_tentative_transactions(slot).await?;

                            // Notify handlers
                            for sig in affected_sigs {
                                let metadata = TxMetadata {
                                    signature: sig.clone(),
                                    slot,
                                    block_time: block.block_time,
                                    fee: 0,
                                    pre_balances: vec![],
                                    post_balances: vec![],
                                    pre_token_balances: vec![],
                                    post_token_balances: vec![],
                                };
                                if let Err(e) = self
                                    .handler_registry
                                    .handle_rollback(&metadata, self.storage.pool())
                                    .await
                                {
                                    log::error!(
                                        "Error handling rollback for transaction {}: {}",
                                        sig,
                                        e
                                    );
                                }
                            }

                            // Rollback storage logic
                            self.storage.rollback_slot(slot).await?;
                        } else {
                            // MATCH! Mark finalized
                            self.storage.mark_finalized(slot, &block.blockhash).await?;
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Failed to fetch finalized block {}: {}", slot, e);
                    // If block is missing in finalized chain but we have it tentative, it might be skipped or orphaned.
                    // Ideally we should rollback if we are sure it's orphaned.
                    // For now, retry next cycle.
                }
            }
        }
        Ok(())
    }
}
