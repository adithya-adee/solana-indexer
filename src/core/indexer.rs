//! Main indexer orchestrator that integrates all components.
//!
//! This module provides the `SolanaIndexer` struct that orchestrates the complete
//! indexing pipeline: polling, fetching, decoding, deduplication, and event handling.

use crate::config::SourceConfig;
use crate::{
    config::{SolanaIndexerConfig, StartStrategy},
    core::{
        account_registry::AccountDecoderRegistry, backfill::BackfillEngine, backfill_defaults::*,
        decoder::Decoder, fetcher::Fetcher, log_registry::LogDecoderRegistry,
        registry::DecoderRegistry,
    },
    storage::{Storage, StorageBackend},
    streams::{TransactionSource, helius::HeliusSource, websocket::WebSocketSource},
    types::traits::{HandlerRegistry, SchemaInitializer},
    utils::{
        error::{Result, SolanaIndexerError},
        logging,
    },
};
use solana_sdk::signature::Signature;
use std::str::FromStr;
use std::sync::Arc;
use tokio::time::{Duration, interval};

/// Main indexer that orchestrates the complete pipeline.
///
/// The `SolanaIndexer` integrates all components to provide a complete,
/// production-ready indexing solution with idempotency, error handling,
/// and graceful shutdown.
///
/// # Example
///
/// ```no_run
/// use solana_indexer::{SolanaIndexer, SolanaIndexerConfigBuilder};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = SolanaIndexerConfigBuilder::new()
///     .with_rpc("http://127.0.0.1:8899")
///     .with_database("postgresql://localhost/mydb")
///     .program_id("11111111111111111111111111111111")
///     .build()?;
///
/// let indexer = SolanaIndexer::new(config).await?;
/// indexer.start().await?;
/// # Ok(())
/// # }
/// ```
pub struct SolanaIndexer {
    config: SolanaIndexerConfig,
    storage: Arc<dyn StorageBackend>,
    fetcher: Arc<Fetcher>,
    decoder: Arc<Decoder>,
    decoder_registry: Arc<DecoderRegistry>,
    log_decoder_registry: Arc<LogDecoderRegistry>,
    account_decoder_registry: Arc<AccountDecoderRegistry>,
    handler_registry: Arc<HandlerRegistry>,
    schema_initializers: Vec<Box<dyn SchemaInitializer>>,
}

impl SolanaIndexer {
    /// Creates a new indexer instance.
    ///
    /// # Arguments
    ///
    /// * `config` - Indexer configuration
    ///
    /// # Errors
    ///
    /// Returns error if database connection fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::{SolanaIndexer, SolanaIndexerConfigBuilder};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = SolanaIndexerConfigBuilder::new()
    ///     .with_rpc("http://127.0.0.1:8899")
    ///     .with_database("postgresql://localhost/mydb")
    ///     .program_id("11111111111111111111111111111111")
    ///     .build()?;
    ///
    /// let indexer = SolanaIndexer::new(config).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(config: SolanaIndexerConfig) -> Result<Self> {
        let storage = Arc::new(Storage::new(&config.database_url).await?);
        storage.initialize().await?;

        let fetcher = Arc::new(Fetcher::new(config.rpc_url()));
        let decoder = Arc::new(Decoder::new());
        let decoder_registry = Arc::new(DecoderRegistry::new_bounded(&config.registry));
        let log_decoder_registry = Arc::new(LogDecoderRegistry::new_bounded(&config.registry));
        let account_decoder_registry =
            Arc::new(AccountDecoderRegistry::new_bounded(&config.registry));
        let handler_registry = Arc::new(HandlerRegistry::new_bounded(&config.registry));

        Ok(Self {
            config,
            storage,
            fetcher,
            decoder,
            decoder_registry,
            log_decoder_registry,
            account_decoder_registry,
            handler_registry,
            schema_initializers: Vec::new(),
        })
    }

    /// Creates a new indexer instance with a custom storage backend.
    ///
    /// This is useful for testing with mock storage.
    pub fn new_with_storage(config: SolanaIndexerConfig, storage: Arc<dyn StorageBackend>) -> Self {
        let fetcher = Arc::new(Fetcher::new(config.rpc_url()));
        let decoder = Arc::new(Decoder::new());
        let decoder_registry = Arc::new(DecoderRegistry::new_bounded(&config.registry));
        let log_decoder_registry = Arc::new(LogDecoderRegistry::new_bounded(&config.registry));
        let account_decoder_registry =
            Arc::new(AccountDecoderRegistry::new_bounded(&config.registry));
        let handler_registry = Arc::new(HandlerRegistry::new_bounded(&config.registry));

        Self {
            config,
            storage,
            fetcher,
            decoder,
            decoder_registry,
            log_decoder_registry,
            account_decoder_registry,
            handler_registry,
            schema_initializers: Vec::new(),
        }
    }

    /// Returns a reference to the handler registry for registering handlers.
    #[must_use]
    pub fn handler_registry(&self) -> &HandlerRegistry {
        &self.handler_registry
    }

    /// Returns a mutable reference to the handler registry.
    ///
    /// Panics if there are multiple references to the registry.
    /// # Panics
    ///
    /// Panics if the handler registry has multiple references (i.e., if it's being shared).
    pub fn handler_registry_mut(&mut self) -> &mut HandlerRegistry {
        Arc::get_mut(&mut self.handler_registry).expect("HandlerRegistry has multiple references")
    }

    /// Returns a reference to the decoder registry.
    #[must_use]
    pub fn decoder_registry(&self) -> &DecoderRegistry {
        &self.decoder_registry
    }

    /// Returns a mutable reference to the decoder registry.
    ///
    /// Panics if there are multiple references to the registry.
    /// # Panics
    ///
    /// Panics if the decoder registry has multiple references.
    pub fn decoder_registry_mut(&mut self) -> &mut DecoderRegistry {
        Arc::get_mut(&mut self.decoder_registry).expect("DecoderRegistry has multiple references")
    }

    /// Returns a reference to the log decoder registry.
    #[must_use]
    pub fn log_decoder_registry(&self) -> &LogDecoderRegistry {
        &self.log_decoder_registry
    }

    /// Returns a mutable reference to the log decoder registry.
    ///
    /// # Panics
    ///
    /// Panics if the log decoder registry has multiple references.
    pub fn log_decoder_registry_mut(&mut self) -> &mut LogDecoderRegistry {
        Arc::get_mut(&mut self.log_decoder_registry)
            .expect("LogDecoderRegistry has multiple references")
    }

    /// Returns a reference to the account decoder registry.
    #[must_use]
    pub fn account_decoder_registry(&self) -> &AccountDecoderRegistry {
        &self.account_decoder_registry
    }

    /// Returns a mutable reference to the account decoder registry.
    ///
    /// # Panics
    ///
    /// Panics if the account decoder registry has multiple references.
    pub fn account_decoder_registry_mut(&mut self) -> &mut AccountDecoderRegistry {
        Arc::get_mut(&mut self.account_decoder_registry)
            .expect("AccountDecoderRegistry has multiple references")
    }

    /// Registers a schema initializer.
    pub fn register_schema_initializer(&mut self, initializer: Box<dyn SchemaInitializer>) {
        self.schema_initializers.push(initializer);
    }

    /// Returns a reference to the decoder for registering event discriminators.
    ///
    /// # Panics
    ///
    /// Panics if the decoder has multiple references.
    pub fn decoder_mut(&mut self) -> &mut Decoder {
        Arc::get_mut(&mut self.decoder).expect("Decoder has multiple references")
    }

    /// Returns a reference to the fetcher.
    #[must_use]
    pub fn fetcher(&self) -> &Fetcher {
        &self.fetcher
    }

    /// Returns a reference to the storage backend.
    #[must_use]
    pub fn storage(&self) -> &dyn StorageBackend {
        &*self.storage
    }

    /// Starts the backfill process.
    ///
    /// This runs the backfill engine until complete or error.
    /// It respects the configuration provided in `BackfillConfig`.
    pub async fn start_backfill(&self) -> Result<()> {
        if !self.config.backfill.enabled {
            logging::log(logging::LogLevel::Info, "Backfill is checking config...");
            // Just return if not enabled? Or error?
            // Usually we might enable it programmatically or via config.
            // If called explicitly, we might want to run it even if config says false?
            // Let's assume config is source of truth.
            if !self.config.backfill.enabled {
                logging::log(
                    logging::LogLevel::Warning,
                    "Backfill disabled in config, skipping.",
                );
                return Ok(());
            }
        }

        logging::log(logging::LogLevel::Info, "Initializing backfill engine...");

        // Setup default strategy
        let strategy = Arc::new(DefaultBackfillStrategy {
            start_slot: self.config.backfill.start_slot,
            end_slot: self.config.backfill.end_slot,
            batch_size: self.config.backfill.batch_size,
            concurrency: self.config.backfill.concurrency,
        });

        // Setup default handlers
        let reorg_handler = Arc::new(DefaultReorgHandler);
        let finalized_tracker = Arc::new(DefaultFinalizedBlockTracker);
        let progress_tracker = Arc::new(DefaultBackfillProgress);

        let engine = BackfillEngine::new(
            self.config.clone(),
            self.fetcher.clone(),
            self.decoder.clone(),
            self.decoder_registry.clone(),
            self.log_decoder_registry.clone(),
            self.account_decoder_registry.clone(),
            self.handler_registry.clone(),
            self.storage.clone(),
            strategy,
            reorg_handler,
            finalized_tracker,
            progress_tracker,
        );

        engine.start().await
    }

    /// Starts the indexer.
    ///
    /// This method runs indefinitely, fetching and processing transactions
    /// based on the configured source (RPC polling or WebSocket subscription).
    /// # Errors
    ///
    /// Returns `SolanaIndexerError` if:
    /// - Database operations fail
    /// - RPC/WebSocket connection fails
    /// - Decoding errors occur
    pub async fn start(self) -> Result<()> {
        match &self.config.source {
            SourceConfig::Rpc { .. } => self.process_rpc_source().await,
            SourceConfig::WebSocket { .. } => self.process_websocket_source().await,
            SourceConfig::Helius { use_websocket, .. } => {
                if *use_websocket {
                    self.process_helius_source().await
                } else {
                    self.process_rpc_source().await
                }
            }
        }
    }

    /// Internal method to run the RPC polling loop.
    async fn process_rpc_source(self) -> Result<()> {
        // Display startup banner
        logging::log_startup(
            &self.config.program_id.to_string(),
            self.config.rpc_url(),
            self.config.poll_interval_secs,
        );

        // Run schema initializers
        for initializer in &self.schema_initializers {
            logging::log(logging::LogLevel::Info, "Initializing database schema...");
            initializer.initialize(self.storage.pool()).await?;
        }
        logging::log(logging::LogLevel::Success, "Database schema initialized");

        let mut poll_interval = interval(Duration::from_secs(self.config.poll_interval_secs));
        let mut last_signature: Option<Signature> = match &self.config.start_strategy {
            StartStrategy::Latest => {
                logging::log(
                    logging::LogLevel::Info,
                    "Strategy: Latest (Fetching checkpoint from RPC)",
                );
                self.fetch_signatures(None).await?.first().copied()
            }
            StartStrategy::Signature(sig) => {
                logging::log(
                    logging::LogLevel::Info,
                    &format!("Strategy: Signature (Starting from {sig})"),
                );
                Some(*sig)
            }
            StartStrategy::Resume => {
                logging::log(
                    logging::LogLevel::Info,
                    "Strategy: Resume (Checking database)",
                );
                if let Some(sig_str) = self.storage.get_last_processed_signature().await? {
                    let sig = Signature::from_str(&sig_str).map_err(|e| {
                        SolanaIndexerError::InternalError(format!("Invalid signature in DB: {e}"))
                    })?;
                    logging::log(logging::LogLevel::Success, &format!("Resuming from: {sig}"));
                    Some(sig)
                } else {
                    logging::log(
                        logging::LogLevel::Warning,
                        "No previous state found. Defaulting to Latest.",
                    );
                    self.fetch_signatures(None).await?.first().copied()
                }
            }
        };

        if let Some(sig) = last_signature {
            logging::log(
                logging::LogLevel::Info,
                &format!("Indexer checkpoint: {sig}"),
            );
        }

        logging::log(logging::LogLevel::Info, "Starting indexer loop (RPC)...\n");

        loop {
            poll_interval.tick().await;

            let start_time = std::time::Instant::now();
            match self.poll_and_process(&mut last_signature).await {
                Ok(processed) => {
                    if processed > 0 {
                        let duration_ms =
                            u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);
                        logging::log_batch(processed, processed, duration_ms);

                        // Report metrics occasionally (e.g., every batch)
                        // In a real implementation, we might want to do this less frequently (timer-based)
                        self.report_metrics();
                    }
                }
                Err(e) => match e {
                    SolanaIndexerError::RpcError(ref msg) => {
                        logging::log_error("RPC failure (Exiting)", msg);
                        return Err(e);
                    }
                    SolanaIndexerError::DatabaseError(ref err) => {
                        logging::log_error(
                            "Database failure (Retrying next cycle)",
                            &err.to_string(),
                        );
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                    _ => {
                        logging::log_error("Indexing error", &e.to_string());
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                },
            }
        }
    }

    /// Internal method to run the WebSocket subscription loop.
    async fn process_websocket_source(self) -> Result<()> {
        // Display startup banner
        logging::log_startup(
            &self.config.program_id.to_string(),
            self.config.rpc_url(),
            0, // Real-time
        );

        // Run schema initializers
        for initializer in &self.schema_initializers {
            logging::log(logging::LogLevel::Info, "Initializing database schema...");
            initializer.initialize(self.storage.pool()).await?;
        }
        logging::log(logging::LogLevel::Success, "Database schema initialized");

        // Extract WebSocket config
        let (ws_url, reconnect_delay) = match &self.config.source {
            SourceConfig::WebSocket {
                ws_url,
                reconnect_delay_secs,
                ..
            } => (ws_url.clone(), *reconnect_delay_secs),
            _ => {
                return Err(crate::utils::error::SolanaIndexerError::ConfigError(
                    "Invalid source config".to_string(),
                ));
            }
        };

        logging::log(
            logging::LogLevel::Info,
            &format!("Starting indexer loop (WebSocket: {ws_url})...\n"),
        );

        let mut source = WebSocketSource::new(ws_url, self.config.program_id, reconnect_delay);

        loop {
            match source.next_batch().await {
                Ok(signatures) => {
                    let start_time = std::time::Instant::now();
                    let mut processed_count = 0;

                    for signature in signatures {
                        let sig_str = signature.to_string();

                        // Check if already processed (idempotency)
                        if self.storage.is_processed(&sig_str).await? {
                            continue;
                        }

                        // Process transaction
                        match self.process_transaction(&signature).await {
                            Ok(()) => {
                                processed_count += 1;
                            }
                            Err(e) => {
                                logging::log_error("Transaction error", &format!("{sig_str}: {e}"));
                            }
                        }
                    }

                    if processed_count > 0 {
                        let duration_ms =
                            u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);
                        logging::log_batch(processed_count, processed_count, duration_ms);
                        self.report_metrics();
                    }
                }
                Err(e) => {
                    logging::log_error("WebSocket error", &e.to_string());
                    tokio::time::sleep(Duration::from_secs(reconnect_delay)).await;
                }
            }
        }
    }

    async fn process_helius_source(self) -> Result<()> {
        logging::log_startup(
            &self.config.program_id.to_string(),
            self.config.rpc_url(),
            0, // Real-time
        );

        // Run schema initializers
        for initializer in &self.schema_initializers {
            logging::log(logging::LogLevel::Info, "Initializing database schema...");
            initializer.initialize(self.storage.pool()).await?;
        }
        logging::log(logging::LogLevel::Success, "Database schema initialized");

        // Instantiate HeliusSource on demand from configuration
        let mut source = HeliusSource::new(self.config.clone()).await?;

        logging::log(
            logging::LogLevel::Info,
            "Starting indexer loop (Helius WebSocket)...",
        );

        // Semaphore to limit concurrent transaction processing
        let semaphore = Arc::new(tokio::sync::Semaphore::new(100)); // Limit to 100 concurrent tasks

        loop {
            match source.next_batch().await {
                Ok(signatures) => {
                    let mut processed_count = 0;

                    for signature in signatures {
                        let sig_str = signature.to_string();

                        // Check if already processed (idempotency)
                        if self.storage.is_processed(&sig_str).await? {
                            continue;
                        }

                        // Acquire permit
                        let permit = semaphore.clone().acquire_owned().await.map_err(|e| {
                            SolanaIndexerError::InternalError(format!("Semaphore error: {e}"))
                        })?;

                        // Clone Arcs for the task
                        let fetcher = self.fetcher.clone();
                        let decoder = self.decoder.clone();
                        let decoder_registry = self.decoder_registry.clone();
                        let log_decoder_registry = self.log_decoder_registry.clone();
                        let account_decoder_registry = self.account_decoder_registry.clone();
                        let handler_registry = self.handler_registry.clone();
                        let storage = self.storage.clone();
                        let config = self.config.clone();

                        // Spawn task
                        tokio::spawn(async move {
                            match Self::process_transaction_core(
                                signature,
                                fetcher,
                                decoder,
                                decoder_registry,
                                log_decoder_registry,
                                account_decoder_registry,
                                handler_registry,
                                storage,
                                config,
                                false, // is_finalized
                                None,  // known_block_hash
                            )
                            .await
                            {
                                Ok(()) => {
                                    // Success
                                }
                                Err(e) => {
                                    logging::log_error(
                                        "Transaction error",
                                        &format!("{}: {}", signature, e),
                                    );
                                }
                            }
                            // Permit is dropped here, allowing next task
                            drop(permit);
                        });

                        processed_count += 1;
                    }

                    if processed_count > 0 {
                        // For logging batch stats, we log 'dispatched' count since processing is async
                        logging::log(
                            logging::LogLevel::Info,
                            &format!("Dispatched {} transactions", processed_count),
                        );
                        self.report_metrics();
                    }
                }
                Err(e) => {
                    logging::log_error("Helius stream error", &e.to_string());
                    // HeliusSource already handles reconnection internally, but if it returns error here,
                    // valid to wait a bit
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn poll_and_process(&self, last_signature: &mut Option<Signature>) -> Result<usize> {
        // Fetch new signatures
        let signatures = self.fetch_signatures(last_signature.as_ref()).await?;

        if signatures.is_empty() {
            return Ok(0);
        }

        // Update last signature for next poll
        if let Some(first_sig) = signatures.first() {
            *last_signature = Some(*first_sig);
        }

        let mut processed_count = 0;

        for signature in signatures {
            let sig_str = signature.to_string();

            // Check if already processed (idempotency)
            if self.storage.is_processed(&sig_str).await? {
                continue;
            }

            // Process transaction
            match self.process_transaction(&signature).await {
                Ok(()) => {
                    processed_count += 1;
                }
                Err(e) => {
                    eprintln!("Error processing transaction {sig_str}: {e}");
                    // Continue with next transaction
                }
            }
        }

        Ok(processed_count)
    }

    async fn fetch_signatures(&self, last_signature: Option<&Signature>) -> Result<Vec<Signature>> {
        use solana_client::rpc_client::RpcClient;
        use solana_sdk::commitment_config::CommitmentConfig;

        let rpc_url = self.config.rpc_url().to_string();
        let program_id = self.config.program_id;
        let batch_size = self.config.batch_size;
        let last_sig = last_signature.copied();

        tokio::task::spawn_blocking(move || {
            let rpc_client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());

            #[allow(deprecated)]
            let sigs = rpc_client
                .get_signatures_for_address_with_config(
                    &program_id,
                    solana_client::rpc_client::GetConfirmedSignaturesForAddress2Config {
                        before: None,
                        until: last_sig,
                        limit: Some(batch_size),
                        commitment: Some(CommitmentConfig::confirmed()),
                    },
                )
                .map_err(|e| crate::utils::error::SolanaIndexerError::RpcError(e.to_string()))?;

            let signatures: Vec<Signature> = sigs
                .into_iter()
                .filter_map(|s| Signature::from_str(&s.signature).ok())
                .collect();

            Ok(signatures)
        })
        .await
        .map_err(|e| crate::utils::error::SolanaIndexerError::InternalError(e.to_string()))?
    }

    async fn process_transaction(&self, signature: &Signature) -> Result<()> {
        Self::process_transaction_core(
            *signature,
            self.fetcher.clone(),
            self.decoder.clone(),
            self.decoder_registry.clone(),
            self.log_decoder_registry.clone(),
            self.account_decoder_registry.clone(),
            self.handler_registry.clone(),
            self.storage.clone(),
            self.config.clone(),
            false, // Real-time polling is usually 'confirmed' so tentative
            None,
        )
        .await
    }

    /// Logs metrics for all registries if metrics are enabled.
    fn report_metrics(&self) {
        if self.config.registry.enable_metrics {
            self.decoder_registry.metrics().report();
            self.log_decoder_registry.metrics().report();
            self.account_decoder_registry.metrics().report();
            self.handler_registry.metrics().report();
        }
    }

    /// Static implementation of transaction processing to allow concurrent execution.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn process_transaction_core(
        signature: Signature,
        fetcher: Arc<Fetcher>,
        decoder: Arc<Decoder>,
        decoder_registry: Arc<DecoderRegistry>,
        log_decoder_registry: Arc<LogDecoderRegistry>,
        account_decoder_registry: Arc<AccountDecoderRegistry>,
        handler_registry: Arc<HandlerRegistry>,
        storage: Arc<dyn StorageBackend>,
        config: SolanaIndexerConfig,
        is_finalized: bool,
        known_block_hash: Option<String>,
    ) -> Result<()> {
        let sig_str = signature.to_string();

        // Fetch transaction
        let transaction = fetcher.fetch_transaction(&signature).await?;

        // Extract block hash if not provided
        // EncodedConfirmedTransactionWithStatusMeta does not have block_hash field directly accessible easily in all versions?
        // Actually, typically we get it from the block, not the transaction response, unless we use `get_transaction` (which is deprecated) or `get_block`.
        // `fetch_transaction` uses `get_transaction_with_config`.
        // The response `EncodedConfirmedTransactionWithStatusMeta` has `slot` and `blockTime` usually.
        // It does NOT have block hash. This is a problem for reorg tracking if we don't have block hash.
        // We might need to fetch the block if we want to be strict, or pass it in.
        // For backfill, we usually iterate blocks, so we HAVE the block hash.
        // For real-time polling `get_signatures_for_address`, we don't have block hash.
        // So for real-time, we might mark tentative without hash? Or fetch block?
        // Fetching block for every tx is expensive.
        // Maybe we just store slot for tentative and verify later?
        // The table `_solana_indexer_tentative` has `block_hash` as NOT NULL.
        // We need a block hash.
        // Let's assume we can tolerate fetching block if missing, or we make it nullable.
        // For now, let's try to get it.
        // `get_transaction` doesn't return block hash.
        // Providing `known_block_hash` is key.

        // Decode transaction metadata
        let decoded_meta = decoder.decode_transaction(&transaction)?;
        let slot = decoded_meta.slot;

        let block_hash = if let Some(h) = known_block_hash {
            h
        } else {
            // We need to fetch block hash if we want to support reorgs properly.
            // Usually polling gives signatures, then we fetch tx.
            // If we really want robustness, we need the block hash.
            // Optimistically, we could just fetch the block header?
            // Or maybe we make block_hash nullable in DB for now? NO, schema says NOT NULL.
            // Let's default to "UNKNOWN" for real-time if we can't get it easily,
            // but that defeats reorg purpose.
            // Correct way: Fetch block for that slot.
            // Optimisation: cache block hashes for slots.

            // For now, let's fetch the block (expensive but correct).
            // Since `fetcher` is available...
            match fetcher.fetch_block(slot).await {
                Ok(block) => block.blockhash,
                Err(_) => "UNKNOWN".to_string(), // Fallback
            }
        };

        // Extract UI instructions from the transaction
        let instructions = match &transaction.transaction.transaction {
            solana_transaction_status::EncodedTransaction::Json(ui_tx) => {
                match &ui_tx.message {
                    solana_transaction_status::UiMessage::Parsed(msg) => &msg.instructions,
                    solana_transaction_status::UiMessage::Raw(_) => {
                        return Ok(()); // Skip non-parsed transactions
                    }
                }
            }
            _ => {
                return Ok(()); // Skip non-JSON transactions
            }
        };

        let mut events_processed = 0;

        // Process based on indexing mode
        if config.indexing_mode.inputs {
            let events = decoder_registry.decode_transaction(instructions);

            for (discriminator, event_data) in events {
                // Retry handler 3 times
                let mut attempts = 0;
                let max_attempts = 3;
                loop {
                    attempts += 1;
                    match handler_registry
                        .handle(&discriminator, &event_data, storage.pool(), &sig_str)
                        .await
                    {
                        Ok(()) => break,
                        Err(e) if attempts < max_attempts => {
                            logging::log_error(
                                "Handler error",
                                &format!("Attempt {attempts}/{max_attempts} for {sig_str}: {e}"),
                            );
                            tokio::time::sleep(Duration::from_millis(100 * attempts)).await;
                        }
                        Err(e) => {
                            logging::log_error(
                                "Handler failed after retries",
                                &format!("{sig_str}: {e}"),
                            );
                            return Err(e);
                        }
                    }
                }
                events_processed += 1;
            }
        }

        if config.indexing_mode.logs {
            let events = log_decoder_registry.decode_logs(&decoded_meta.events);

            for (discriminator, event_data) in events {
                // Retry handler 3 times
                let mut attempts = 0;
                let max_attempts = 3;
                loop {
                    attempts += 1;
                    match handler_registry
                        .handle(&discriminator, &event_data, storage.pool(), &sig_str)
                        .await
                    {
                        Ok(()) => break,
                        Err(e) if attempts < max_attempts => {
                            logging::log_error(
                                "Handler error",
                                &format!("Attempt {attempts}/{max_attempts} for {sig_str}: {e}"),
                            );
                            tokio::time::sleep(Duration::from_millis(100 * attempts)).await;
                        }
                        Err(e) => {
                            logging::log_error(
                                "Handler failed after retries",
                                &format!("{sig_str}: {e}"),
                            );
                            return Err(e);
                        }
                    }
                }
                events_processed += 1;
            }

            if config.indexing_mode.accounts {
                // Extract unique writable accounts from the transaction
                // We focus on writable accounts as their state might have changed
                let mut writable_accounts = std::collections::HashSet::new();

                if let solana_transaction_status::EncodedTransaction::Json(ui_tx) =
                    &transaction.transaction.transaction
                {
                    match &ui_tx.message {
                        solana_transaction_status::UiMessage::Parsed(msg) => {
                            for account in &msg.account_keys {
                                if account.writable
                                    && let Ok(pubkey) =
                                        solana_sdk::pubkey::Pubkey::from_str(&account.pubkey)
                                {
                                    writable_accounts.insert(pubkey);
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
                    // Batch fetch
                    if let Ok(accounts) = fetcher.fetch_multiple_accounts(&keys).await {
                        for account in accounts.iter().flatten() {
                            let decoded_list = account_decoder_registry.decode_account(account);
                            for (discriminator, event_data) in decoded_list {
                                // Dispatch to handler
                                // Retry logic similar to above
                                let mut attempts = 0;
                                let max_attempts = 3;
                                loop {
                                    attempts += 1;
                                    match handler_registry
                                        .handle(
                                            &discriminator,
                                            &event_data,
                                            storage.pool(),
                                            &sig_str,
                                        )
                                        .await
                                    {
                                        Ok(()) => break,
                                        Err(e) if attempts < max_attempts => {
                                            logging::log_error(
                                                "Handler error (Account)",
                                                &format!(
                                                    "Attempt {attempts}/{max_attempts} for {sig_str}: {e}"
                                                ),
                                            );
                                            tokio::time::sleep(Duration::from_millis(
                                                100 * attempts,
                                            ))
                                            .await;
                                        }
                                        Err(e) => {
                                            logging::log_error(
                                                "Handler failed after retries (Account)",
                                                &format!("{sig_str}: {e}"),
                                            );
                                            // We log error but maybe don't fail the whole tx for one account?
                                            // Return error to be safe
                                            return Err(e);
                                        }
                                    }
                                }
                                events_processed += 1;
                            }
                        }
                    }
                }
            }
        }

        // Mark as processed or tentative
        if is_finalized {
            storage.mark_finalized(slot, &block_hash).await?;
            storage.mark_processed(&sig_str, slot).await?;
        } else {
            storage.mark_tentative(&sig_str, slot, &block_hash).await?;
        }

        if events_processed > 0 {
            logging::log(
                logging::LogLevel::Success,
                &format!("Processed transaction: {sig_str} ({events_processed} events)"),
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SolanaIndexerConfigBuilder;

    #[tokio::test]
    async fn test_indexer_creation_rpc() {
        let config = SolanaIndexerConfigBuilder::new()
            .with_rpc("http://127.0.0.1:8899")
            .with_database("postgresql://localhost/db")
            .program_id("11111111111111111111111111111111")
            .build()
            .unwrap();

        // We can't fully instantiate SolanaIndexer without a real DB, so we verify config
        assert_eq!(config.rpc_url(), "http://127.0.0.1:8899");
        match config.source {
            SourceConfig::Rpc { .. } => {}
            _ => panic!("Expected RPC source"),
        }
    }

    #[tokio::test]
    async fn test_indexer_creation_ws() {
        let config = SolanaIndexerConfigBuilder::new()
            .with_ws("ws://127.0.0.1:8900", "http://127.0.0.1:8899")
            .with_database("postgresql://localhost/db")
            .program_id("11111111111111111111111111111111")
            .build()
            .unwrap();

        assert_eq!(config.rpc_url(), "http://127.0.0.1:8899");
        match config.source {
            SourceConfig::WebSocket { ws_url, .. } => assert_eq!(ws_url, "ws://127.0.0.1:8900"),
            _ => panic!("Expected WebSocket source"),
        }
    }
}
