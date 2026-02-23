use crate::config::SolanaIndexerConfig;
use crate::core::decoding::Decoder;
use crate::core::execution::fetcher::Fetcher;
use crate::core::execution::indexer::SolanaIndexer;
use crate::core::registry::account::AccountDecoderRegistry;
use crate::core::registry::logs::LogDecoderRegistry;
use crate::core::registry::DecoderRegistry;
use crate::storage::StorageBackend;
use crate::types::backfill_traits::{
    BackfillHandlerRegistry, BackfillProgress, BackfillRange, BackfillStrategy,
    FinalizedBlockTracker, ReorgHandler,
};
use crate::types::traits::HandlerRegistry;
use crate::utils::error::{Result, SolanaIndexerError};
use crate::utils::logging::{log, log_error, LogLevel};
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
    backfill_handlers: Arc<BackfillHandlerRegistry>,
    storage: Arc<dyn StorageBackend>,
    strategy: Arc<dyn BackfillStrategy>,
    reorg_handler: Arc<dyn ReorgHandler>,
    finalized_tracker: Arc<dyn FinalizedBlockTracker>,
    progress_tracker: Arc<dyn BackfillProgress>,
    cancellation_token: tokio_util::sync::CancellationToken,
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
        cancellation_token: tokio_util::sync::CancellationToken,
        backfill_handlers: Arc<BackfillHandlerRegistry>,
    ) -> Self {
        Self {
            config,
            fetcher,
            decoder,
            decoder_registry,
            log_decoder_registry,
            account_decoder_registry,
            handler_registry,
            backfill_handlers,
            storage,
            strategy,
            reorg_handler,
            finalized_tracker,
            progress_tracker,
            cancellation_token,
        }
    }

    /// Starts backfill for a specific range.
    ///
    /// This is used by BackfillManager to process a specific slot range.
    #[tracing::instrument(skip_all, fields(start_slot = range.start_slot, end_slot = range.end_slot))]
    pub async fn start_range(&self, range: BackfillRange) -> Result<()> {
        log(
            LogLevel::Info,
            &format!(
                "BackfillEngine: Processing range [{}, {}]",
                range.start_slot, range.end_slot
            ),
        );

        let mut current_slot = range.start_slot;
        let end_slot = range.end_slot;
        let concurrency = self.strategy.concurrency();
        let semaphore = Arc::new(Semaphore::new(concurrency));

        while current_slot <= end_slot {
            if self.cancellation_token.is_cancelled() {
                log(LogLevel::Warning, "Backfill cancelled by user.");
                break;
            }

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
                    let mut relevant_signatures = Vec::new();

                    if let Some(transactions) = block.transactions {
                        for tx_with_meta in transactions {
                            if let EncodedTransaction::Json(ui_tx) = &tx_with_meta.transaction {
                                let sigs = &ui_tx.signatures;
                                if let Some(primary_sig) = sigs.first() {
                                    match &ui_tx.message {
                                        UiMessage::Parsed(msg) => {
                                            let is_relevant = msg.account_keys.iter().any(|acc| {
                                                if let Ok(acc_pubkey) =
                                                    solana_sdk::pubkey::Pubkey::from_str(
                                                        &acc.pubkey,
                                                    )
                                                {
                                                    self.config.program_ids.contains(&acc_pubkey)
                                                } else {
                                                    false
                                                }
                                            });
                                            if is_relevant {
                                                relevant_signatures.push((
                                                    primary_sig.clone(),
                                                    block_hash.clone(),
                                                ));
                                            }
                                        }
                                        UiMessage::Raw(msg) => {
                                            let is_relevant = msg.account_keys.iter().any(|key| {
                                                self.config
                                                    .program_ids
                                                    .iter()
                                                    .any(|p| p.to_string() == *key)
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
                        }
                    }

                    if !relevant_signatures.is_empty() {
                        let mut tasks = Vec::new();
                        for (sig_str, blk_hash) in relevant_signatures.into_iter() {
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
                                let backfill_handlers = self.backfill_handlers.clone();
                                let storage = self.storage.clone();
                                let config = self.config.clone();

                                tasks.push(tokio::spawn(async move {
                                    let res = Self::process_backfill_transaction_core(
                                        sig,
                                        fetcher,
                                        decoder,
                                        decoder_registry,
                                        log_decoder_registry,
                                        account_decoder_registry,
                                        backfill_handlers,
                                        storage,
                                        config,
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
                                        log_error(
                                            "Error processing backfill transaction",
                                            &e.to_string(),
                                        );
                                    }
                                }
                                Err(e) => {
                                    log_error("Task join error", &e.to_string());
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
                    log_error(
                        &format!("Slot {} skipped or fetch failed", current_slot),
                        &e.to_string(),
                    );
                }
            }

            current_slot += 1;
        }

        log(
            LogLevel::Success,
            &format!(
                "BackfillEngine: Completed range [{}, {}]",
                range.start_slot, range.end_slot
            ),
        );
        Ok(())
    }

    /// Processes a backfill transaction using BackfillHandlerRegistry.
    ///
    /// Similar to `SolanaIndexer::process_transaction_core` but dispatches
    /// to BackfillHandler<T> instead of EventHandler<T>.
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip_all, fields(signature = %signature))]
    pub(crate) async fn process_backfill_transaction_core(
        signature: Signature,
        fetcher: Arc<Fetcher>,
        decoder: Arc<Decoder>,
        decoder_registry: Arc<DecoderRegistry>,
        log_decoder_registry: Arc<LogDecoderRegistry>,
        account_decoder_registry: Arc<AccountDecoderRegistry>,
        backfill_handlers: Arc<BackfillHandlerRegistry>,
        storage: Arc<dyn StorageBackend>,
        config: SolanaIndexerConfig,
        known_block_hash: Option<String>,
    ) -> Result<()> {
        let sig_str = signature.to_string();

        // Fetch transaction
        let transaction = fetcher.fetch_transaction(&signature).await?;

        // Decode transaction metadata
        let decoded_meta = decoder.decode_transaction(&transaction)?;
        let slot = decoded_meta.slot;

        // Extract native metadata
        let meta = transaction.transaction.meta.as_ref().ok_or_else(|| {
            SolanaIndexerError::DecodingError("Missing transaction metadata".into())
        })?;

        let pre_token_balances_opt: Option<
            Vec<solana_transaction_status::UiTransactionTokenBalance>,
        > = meta.pre_token_balances.clone().into();
        let pre_token_balances = pre_token_balances_opt.unwrap_or_default();
        let post_token_balances_opt: Option<
            Vec<solana_transaction_status::UiTransactionTokenBalance>,
        > = meta.post_token_balances.clone().into();
        let post_token_balances = post_token_balances_opt.unwrap_or_default();

        // Construct context
        let context = crate::types::metadata::TxMetadata {
            slot,
            block_time: transaction.block_time,
            fee: meta.fee,
            pre_balances: meta.pre_balances.clone(),
            post_balances: meta.post_balances.clone(),
            pre_token_balances: pre_token_balances
                .into_iter()
                .map(|b| crate::types::metadata::TokenBalanceInfo {
                    account_index: b.account_index,
                    mint: b.mint,
                    owner: Into::<Option<String>>::into(b.owner).unwrap_or_default(),
                    amount: b.ui_token_amount.amount,
                    decimals: b.ui_token_amount.decimals,
                    program_id: b.program_id.into(),
                })
                .collect(),
            post_token_balances: post_token_balances
                .into_iter()
                .map(|b| crate::types::metadata::TokenBalanceInfo {
                    account_index: b.account_index,
                    mint: b.mint,
                    owner: Into::<Option<String>>::into(b.owner).unwrap_or_default(),
                    amount: b.ui_token_amount.amount,
                    decimals: b.ui_token_amount.decimals,
                    program_id: b.program_id.into(),
                })
                .collect(),
            signature: sig_str.clone(),
        };

        let block_hash = if let Some(h) = known_block_hash {
            h
        } else {
            match fetcher.fetch_block(slot).await {
                Ok(block) => block.blockhash,
                Err(_) => "UNKNOWN".to_string(),
            }
        };

        // Extract UI instructions from the transaction
        let instructions: &[solana_transaction_status::UiInstruction] = match &transaction
            .transaction
            .transaction
        {
            solana_transaction_status::EncodedTransaction::Json(ui_tx) => match &ui_tx.message {
                solana_transaction_status::UiMessage::Parsed(msg) => &msg.instructions,
                _ => &[],
            },
            _ => &[],
        };

        let mut events_processed = 0;

        // Process based on indexing mode - dispatch to BackfillHandler
        if config.indexing_mode.inputs {
            let events = decoder_registry.decode_transaction(instructions);

            for (discriminator, event_data) in events {
                match backfill_handlers
                    .handle_backfill(&discriminator, &event_data, &context, storage.pool())
                    .await
                {
                    Ok(()) => events_processed += 1,
                    Err(e) => {
                        log_error("Backfill handler error", &format!("{sig_str}: {e}"));
                        return Err(e);
                    }
                }
            }
        }

        if config.indexing_mode.logs {
            let events = log_decoder_registry.decode_logs(&decoded_meta.events);

            for (discriminator, event_data) in events {
                match backfill_handlers
                    .handle_backfill(&discriminator, &event_data, &context, storage.pool())
                    .await
                {
                    Ok(()) => events_processed += 1,
                    Err(e) => {
                        log_error("Backfill handler error (logs)", &format!("{sig_str}: {e}"));
                        return Err(e);
                    }
                }
            }

            if config.indexing_mode.accounts {
                let mut writable_accounts = std::collections::HashSet::new();

                if let solana_transaction_status::EncodedTransaction::Json(ui_tx) =
                    &transaction.transaction.transaction
                {
                    match &ui_tx.message {
                        solana_transaction_status::UiMessage::Parsed(msg) => {
                            for account in &msg.account_keys {
                                if account.writable {
                                    if let Ok(pubkey) =
                                        solana_sdk::pubkey::Pubkey::from_str(&account.pubkey)
                                    {
                                        writable_accounts.insert(pubkey);
                                    }
                                }
                            }
                        }
                        solana_transaction_status::UiMessage::Raw(msg) => {
                            for key_str in &msg.account_keys {
                                if let Ok(pubkey) = solana_sdk::pubkey::Pubkey::from_str(key_str) {
                                    writable_accounts.insert(pubkey);
                                }
                            }
                        }
                    }
                };

                if !writable_accounts.is_empty() {
                    let keys: Vec<_> = writable_accounts.into_iter().collect();
                    if let Ok(accounts) = fetcher.fetch_multiple_accounts(&keys).await {
                        for (index, account_option) in accounts.iter().enumerate() {
                            if let Some(account) = account_option {
                                let pubkey = &keys[index];
                                let decoded_list =
                                    account_decoder_registry.decode_account(pubkey, account);
                                for (discriminator, event_data) in decoded_list {
                                    match backfill_handlers
                                        .handle_backfill(
                                            &discriminator,
                                            &event_data,
                                            &context,
                                            storage.pool(),
                                        )
                                        .await
                                    {
                                        Ok(()) => events_processed += 1,
                                        Err(e) => {
                                            log_error(
                                                "Backfill handler error (account)",
                                                &format!("{sig_str}: {e}"),
                                            );
                                            return Err(e);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Mark as processed (backfill transactions are always finalized)
        storage.mark_finalized(slot, &block_hash).await?;
        storage.mark_processed(&sig_str, slot).await?;

        if events_processed > 0 {
            log(
                LogLevel::Success,
                &format!("Processed backfill transaction: {sig_str} ({events_processed} events)"),
            );
        }

        Ok(())
    }

    /// Starts the backfill process.
    ///
    /// This method orchestrates the historical data indexing by:
    /// 1. Determining the start and end slots via the configured strategy.
    /// 2. Iterating through slots concurrently.
    /// 3. Detecting and handling chain reorganizations.
    /// 4. Filtering blocks for relevant transactions.
    /// 5. Processing transactions using the standard indexer pipeline.
    #[tracing::instrument(skip_all)]
    pub async fn start(&self) -> Result<()> {
        log(LogLevel::Info, "Starting backfill engine...");

        let (start_slot_opt, end_slot_opt) =
            self.strategy.get_slot_range(self.storage.as_ref()).await?;

        let mut current_slot = if let Some(s) = start_slot_opt {
            s
        } else if let Some(saved) = self
            .progress_tracker
            .load_progress(self.storage.as_ref())
            .await?
        {
            log(
                LogLevel::Info,
                &format!("Resuming backfill from saved progress: {}", saved),
            );
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

        log(
            LogLevel::Info,
            &format!("Backfill range: {} to {}", current_slot, end_slot),
        );

        let concurrency = self.strategy.concurrency();
        let semaphore = Arc::new(Semaphore::new(concurrency));

        while current_slot <= end_slot {
            if self.cancellation_token.is_cancelled() {
                log(LogLevel::Warning, "Backfill cancelled by user.");
                break;
            }

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
                                                // Iterate over account keys to ensure strict program ID relevance.
                                                // This robustness check prevents false positives if the signature
                                                // was associated with multiple instructions or programs.
                                                let is_relevant =
                                                    msg.account_keys.iter().any(|acc| {
                                                        if let Ok(acc_pubkey) =
                                                            solana_sdk::pubkey::Pubkey::from_str(
                                                                &acc.pubkey,
                                                            )
                                                        {
                                                            self.config
                                                                .program_ids
                                                                .contains(&acc_pubkey)
                                                        } else {
                                                            false
                                                        }
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
                                                        self.config
                                                            .program_ids
                                                            .iter()
                                                            .any(|p| p.to_string() == *key)
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
                        log(
                            LogLevel::Info,
                            &format!(
                                "Slot {}: found {} relevant transactions",
                                current_slot,
                                relevant_signatures.len()
                            ),
                        );

                        let mut tasks = Vec::new();
                        for (sig_str, blk_hash) in relevant_signatures.into_iter() {
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
                                        None, // preloaded_transaction
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
                                        log_error(
                                            "Error processing backfill transaction",
                                            &e.to_string(),
                                        );
                                    }
                                }
                                Err(e) => {
                                    log_error("Task join error", &e.to_string());
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
                    log_error(
                        &format!("Slot {} skipped or fetch failed", current_slot),
                        &e.to_string(),
                    );
                }
            }

            current_slot += 1;
        }

        self.progress_tracker
            .mark_complete(self.storage.as_ref())
            .await?;
        log(LogLevel::Success, "Backfill complete!");
        Ok(())
    }
}
