use crate::config::SolanaIndexerConfig;
use crate::core::account_registry::AccountDecoderRegistry;
use crate::core::decoder::Decoder;
use crate::core::fetcher::Fetcher;
use crate::core::indexer::SolanaIndexer;
use crate::core::log_registry::LogDecoderRegistry;
use crate::core::registry::DecoderRegistry;
use crate::storage::StorageBackend;
use crate::types::backfill_traits::{
    BackfillProgress, BackfillStrategy, FinalizedBlockTracker, ReorgHandler,
};
use crate::types::traits::HandlerRegistry;
use crate::utils::error::{Result, SolanaIndexerError};
use solana_sdk::signature::Signature;
use solana_transaction_status::{EncodedTransaction, UiMessage};
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Semaphore;

pub struct BackfillEngine {
    config: SolanaIndexerConfig,
    fetcher: Arc<Fetcher>,
    decoder: Arc<Decoder>,
    decoder_registry: Arc<DecoderRegistry>,
    log_decoder_registry: Arc<LogDecoderRegistry>,
    account_decoder_registry: Arc<AccountDecoderRegistry>,
    handler_registry: Arc<HandlerRegistry>,
    storage: Arc<dyn StorageBackend>,
    strategy: Arc<dyn BackfillStrategy>,
    reorg_handler: Arc<dyn ReorgHandler>,
    finalized_tracker: Arc<dyn FinalizedBlockTracker>,
    progress_tracker: Arc<dyn BackfillProgress>,
}

impl BackfillEngine {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: SolanaIndexerConfig,
        fetcher: Arc<Fetcher>,
        decoder: Arc<Decoder>,
        decoder_registry: Arc<DecoderRegistry>,
        log_decoder_registry: Arc<LogDecoderRegistry>,
        account_decoder_registry: Arc<AccountDecoderRegistry>,
        handler_registry: Arc<HandlerRegistry>,
        storage: Arc<dyn StorageBackend>,
        strategy: Arc<dyn BackfillStrategy>,
        reorg_handler: Arc<dyn ReorgHandler>,
        finalized_tracker: Arc<dyn FinalizedBlockTracker>,
        progress_tracker: Arc<dyn BackfillProgress>,
    ) -> Self {
        Self {
            config,
            fetcher,
            decoder,
            decoder_registry,
            log_decoder_registry,
            account_decoder_registry,
            handler_registry,
            storage,
            strategy,
            reorg_handler,
            finalized_tracker,
            progress_tracker,
        }
    }

    pub async fn start(&self) -> Result<()> {
        log::info!("Starting backfill engine...");

        let (start_slot_opt, end_slot_opt) =
            self.strategy.get_slot_range(self.storage.as_ref()).await?;

        let mut current_slot = if let Some(s) = start_slot_opt {
            s
        } else if let Some(saved) = self
            .progress_tracker
            .load_progress(self.storage.as_ref())
            .await?
        {
            log::info!("Resuming backfill from saved progress: {}", saved);
            saved + 1
        } else {
            0
        };

        let end_slot = if let Some(e) = end_slot_opt {
            e
        } else {
            self.finalized_tracker
                .get_latest_finalized_slot(&self.fetcher)
                .await?
        };

        log::info!("Backfill range: {} to {}", current_slot, end_slot);

        let concurrency = self.strategy.concurrency();
        let semaphore = Arc::new(Semaphore::new(concurrency));

        while current_slot <= end_slot {
            // Check for reorgs periodically
            if let Some(reorg_event) = self
                .reorg_handler
                .detect_reorg(current_slot, self.storage.as_ref(), &self.fetcher)
                .await?
            {
                self.reorg_handler
                    .handle_reorg(reorg_event, self.storage.as_ref())
                    .await?;
            }

            match self.fetcher.fetch_block(current_slot).await {
                Ok(block) => {
                    let block_hash = block.blockhash;
                    // Filter transactions for our program
                    let mut relevant_signatures = Vec::new();

                    if let Some(transactions) = block.transactions {
                        for tx_with_meta in transactions {
                            // Extract signature and check if program is involved
                            // EncodedTransactionWithStatusMeta has transaction: EncodedTransaction
                            match &tx_with_meta.transaction {
                                EncodedTransaction::Json(ui_tx) => {
                                    let sigs = &ui_tx.signatures;
                                    if let Some(primary_sig) = sigs.first() {
                                        // maximize robustness: check strict program ID match in messages
                                        // UiTransaction -> Message -> AccountKeys
                                        match &ui_tx.message {
                                            UiMessage::Parsed(msg) => {
                                                // iterate account keys
                                                let is_relevant =
                                                    msg.account_keys.iter().any(|acc| {
                                                        acc.pubkey
                                                            == self.config.program_id.to_string()
                                                    });
                                                if is_relevant {
                                                    relevant_signatures.push((
                                                        primary_sig.clone(),
                                                        block_hash.clone(),
                                                    ));
                                                }
                                            }
                                            UiMessage::Raw(msg) => {
                                                let is_relevant =
                                                    msg.account_keys.iter().any(|key| {
                                                        *key == self.config.program_id.to_string()
                                                    });
                                                if is_relevant {
                                                    relevant_signatures.push((
                                                        primary_sig.clone(),
                                                        block_hash.clone(),
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }
                                _ => {
                                    // Binary encoding not supported for backfill filtering in this implementation yet
                                    // Or assume irrelevant if not parsing binary here (for safety)
                                    // To support binary, we'd need to base64 decode and parse raw tx
                                }
                            }
                        }
                    }

                    if !relevant_signatures.is_empty() {
                        log::info!(
                            "Slot {}: found {} relevant transactions",
                            current_slot,
                            relevant_signatures.len()
                        );

                        let mut tasks = Vec::new();
                        for (sig_str, blk_hash) in relevant_signatures {
                            if let Ok(sig) = Signature::from_str(&sig_str) {
                                let permit =
                                    semaphore.clone().acquire_owned().await.map_err(|e| {
                                        SolanaIndexerError::InternalError(e.to_string())
                                    })?;

                                let fetcher = self.fetcher.clone();
                                let decoder = self.decoder.clone();
                                let decoder_registry = self.decoder_registry.clone();
                                let log_decoder_registry = self.log_decoder_registry.clone();
                                let account_decoder_registry =
                                    self.account_decoder_registry.clone();
                                let handler_registry = self.handler_registry.clone();
                                let storage = self.storage.clone();
                                let config = self.config.clone();

                                tasks.push(tokio::spawn(async move {
                                    let res = SolanaIndexer::process_transaction_core(
                                        sig,
                                        fetcher,
                                        decoder,
                                        decoder_registry,
                                        log_decoder_registry,
                                        account_decoder_registry,
                                        handler_registry,
                                        storage,
                                        config,
                                        true, // is_finalized
                                        Some(blk_hash),
                                    )
                                    .await;
                                    drop(permit);
                                    res
                                }));
                            }
                        }

                        for task in tasks {
                            match task.await {
                                Ok(res) => {
                                    if let Err(e) = res {
                                        log::error!("Error processing backfill transaction: {}", e);
                                    }
                                }
                                Err(e) => {
                                    log::error!("Task join error: {}", e);
                                }
                            }
                        }
                    }

                    self.finalized_tracker
                        .mark_finalized(current_slot, &block_hash, self.storage.as_ref())
                        .await?;
                    self.progress_tracker
                        .save_progress(current_slot, self.storage.as_ref())
                        .await?;
                }
                Err(e) => {
                    // Block might be missing or skipped (e.g. slot has no block)
                    println!("Slot {} skipped or fetch failed: {}", current_slot, e);
                }
            }

            current_slot += 1;
        }

        self.progress_tracker
            .mark_complete(self.storage.as_ref())
            .await?;
        log::info!("Backfill complete!");
        Ok(())
    }
}
