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
    streams::{helius::HeliusSource, websocket::WebSocketSource, TransactionSource},
    types::{
        metadata::{TokenBalanceInfo, TxMetadata},
        traits::{HandlerRegistry, SchemaInitializer},
    },
    utils::{
        error::{Result, SolanaIndexerError},
        logging,
    },
};
use solana_sdk::signature::Signature;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::time::{interval, Duration};

/// Main indexer that orchestrates the complete pipeline.
///
/// The `SolanaIndexer` integrates all components to provide a complete,
/// production-ready indexing solution with idempotency, error handling,
/// and graceful shutdown.
///
/// # Example
///
/// ```no_run
/// use solana_indexer_sdk::{SolanaIndexer, SolanaIndexerConfigBuilder};
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let config = SolanaIndexerConfigBuilder::new()
///         .with_rpc("https://api.mainnet-beta.solana.com")
///         .with_database("postgresql://postgres:password@localhost/indexer")
///         .program_id("675k1q2wE7s6L3R29fs6tcMbtFD4vT759Wcx3CY6CSLg")
///         .build()?;
///
///     let mut indexer = SolanaIndexer::new(config).await?;
///
///     // Register decoders and handlers
///     // indexer.register_decoder("program_name", MyDecoder);
///     // indexer.register_handler(MyHandler);
///
///     indexer.start().await?;
///     Ok(())
/// }
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
    cancellation_token: tokio_util::sync::CancellationToken,
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
    /// # use solana_indexer_sdk::{SolanaIndexer, SolanaIndexerConfigBuilder};
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

        let fetcher = Arc::new(Fetcher::new(
            config.rpc_url(),
            config.commitment_level.into(),
        ));
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
            cancellation_token: tokio_util::sync::CancellationToken::new(),
        })
    }

    /// Creates a new indexer instance with a custom storage backend.
    ///
    /// This is useful for testing with mock storage.
    pub fn new_with_storage(config: SolanaIndexerConfig, storage: Arc<dyn StorageBackend>) -> Self {
        let fetcher = Arc::new(Fetcher::new(
            config.rpc_url(),
            config.commitment_level.into(),
        ));
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
            cancellation_token: tokio_util::sync::CancellationToken::new(),
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

    /// Registers a typed instruction decoder.
    ///
    /// This generic method automatically handles the boxing and type erasure required by the registry,
    /// simplifying the API for developers.
    ///
    /// # Arguments
    ///
    /// * `program_id` - The program ID associated with this decoder
    /// * `decoder` - The typed decoder instance
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer_sdk::SolanaIndexer;
    /// # struct MyDecoder;
    /// # struct MyEvent;
    /// # impl solana_indexer_sdk::InstructionDecoder<MyEvent> for MyDecoder { fn decode(&self, _: &solana_transaction_status::UiInstruction) -> Option<MyEvent> { None } }
    /// # impl solana_indexer_sdk::EventDiscriminator for MyEvent { fn discriminator() -> [u8; 8] { [0; 8] } }
    /// # impl borsh::BorshSerialize for MyEvent { fn serialize<W: std::io::Write>(&self, _: &mut W) -> std::io::Result<()> { Ok(()) } }
    /// # fn example(indexer: &mut SolanaIndexer) {
    /// indexer.register_decoder("program_id", MyDecoder);
    /// # }
    /// ```
    pub fn register_decoder<D, E>(&mut self, program_id: impl Into<String>, decoder: D)
    where
        D: crate::types::traits::InstructionDecoder<E> + 'static,
        E: crate::types::events::EventDiscriminator + borsh::BorshSerialize + Send + Sync + 'static,
    {
        use crate::types::traits::DynamicInstructionDecoder;
        let boxed_typed: Box<dyn crate::types::traits::InstructionDecoder<E>> = Box::new(decoder);
        // Box<dyn InstructionDecoder<E>> automatically implements DynamicInstructionDecoder
        // but we need to box it again to match the registry's expectation of Box<dyn DynamicInstructionDecoder>
        // Use an explicit cast/coercion to ensure correct vtable dispatch
        let boxed_dynamic: Box<dyn DynamicInstructionDecoder> = Box::new(boxed_typed);
        self.decoder_registry_mut()
            .register(program_id.into(), boxed_dynamic)
            .expect("Failed to register decoder");
    }

    /// Registers a typed event handler.
    ///
    /// This generic method automatically handles the boxing and type erasure required by the registry.
    ///
    /// # Arguments
    ///
    /// * `handler` - The typed handler instance
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer_sdk::{SolanaIndexer, TxMetadata};
    /// # use async_trait::async_trait;
    /// # struct MyHandler;
    /// # struct MyEvent;
    /// # #[async_trait]
    /// # impl solana_indexer_sdk::EventHandler<MyEvent> for MyHandler { async fn handle(&self, _: MyEvent, _: &TxMetadata, _: &sqlx::PgPool) -> solana_indexer_sdk::Result<()> { Ok(()) } }
    /// # impl solana_indexer_sdk::EventDiscriminator for MyEvent { fn discriminator() -> [u8; 8] { [0; 8] } }
    /// # impl borsh::BorshDeserialize for MyEvent {
    /// #   fn deserialize(buf: &mut &[u8]) -> std::io::Result<Self> { Ok(MyEvent) }
    /// #   fn deserialize_reader<R: std::io::Read>(_: &mut R) -> std::io::Result<Self> { Ok(MyEvent) }
    /// # }
    /// # fn example(indexer: &mut SolanaIndexer) {
    /// indexer.register_handler(MyHandler);
    /// # }
    /// ```
    pub fn register_handler<H, E>(&mut self, handler: H)
    where
        H: crate::types::traits::EventHandler<E> + 'static,
        E: crate::types::events::EventDiscriminator
            + borsh::BorshDeserialize
            + Send
            + Sync
            + 'static,
    {
        use crate::types::traits::DynamicEventHandler;
        let boxed_typed: Box<dyn crate::types::traits::EventHandler<E>> = Box::new(handler);
        let boxed_dynamic: Box<dyn DynamicEventHandler> = Box::new(boxed_typed);

        // We propagate any error from register (e.g. registry full) by panicking or handling?
        // The original method returned Result but this convenience method swallows it or panics?
        // Better to unwrap or changing API to return Result?
        // The release assessment example usage showed no Result: `indexer.register_decoder(...)`
        // But `register` returns generic Result via `?`.
        // Let's make these methods return nothing and panic on error for simplicity (convention in builders/setup),
        // or log error. Given "Painful API", let's keep it simple.
        // Actually, `register` on registries might fail if full.
        // Let's just unwrap/expect for now as this is setup phase.
        self.handler_registry_mut()
            .register(E::discriminator(), boxed_dynamic)
            .expect("Failed to register handler");
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
        let token = self.cancellation_token.clone();

        // Spawn signal handler
        tokio::spawn(async move {
            if let Ok(()) = tokio::signal::ctrl_c().await {
                logging::log(logging::LogLevel::Info, "Received Ctrl+C, shutting down...");
                token.cancel();
            }
        });

        // Spawn background cleanup task
        let cleanup_token = self.cancellation_token.clone();
        let storage = self.storage.clone();
        let threshold = self.config.stale_tentative_threshold;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60)); // Check every minute
            loop {
                tokio::select! {
                    _ = cleanup_token.cancelled() => break,
                    _ = interval.tick() => {
                        match storage.cleanup_stale_tentative_transactions(threshold).await {
                            Ok(count) => {
                                if count > 0 {
                                    logging::log(logging::LogLevel::Info, &format!("Cleaned up {} stale tentative transactions", count));
                                }
                            }
                            Err(e) => {
                                logging::log_error("Cleanup error", &e.to_string());
                            }
                        }
                    }
                }
            }
        });

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
            SourceConfig::Hybrid { .. } => self.process_hybrid_source().await,
        }
    }

    /// Triggers a graceful shutdown programmatically.
    pub fn shutdown(&self) {
        self.cancellation_token.cancel();
    }

    /// Returns a clone of the cancellation token.
    pub fn cancellation_token(&self) -> tokio_util::sync::CancellationToken {
        self.cancellation_token.clone()
    }

    /// Internal method to run the RPC polling loop.
    async fn process_rpc_source(self) -> Result<()> {
        // Display startup banner
        logging::log_startup(
            &self
                .config
                .program_ids
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<String>>()
                .join(", "),
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
                self.fetch_signatures(None)
                    .await?
                    .first()
                    .map(|e| e.signature())
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
                    self.fetch_signatures(None)
                        .await?
                        .first()
                        .map(|e| e.signature())
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
            &self
                .config
                .program_ids
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<String>>()
                .join(", "),
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

        let mut source =
            WebSocketSource::new(ws_url, self.config.program_ids.clone(), reconnect_delay);

        loop {
            if self.cancellation_token.is_cancelled() {
                logging::log(logging::LogLevel::Info, "Graceful shutdown complete.");
                break;
            }

            let batch = tokio::select! {
                 _ = self.cancellation_token.cancelled() => {
                    logging::log(logging::LogLevel::Info, "Graceful shutdown initiated...");
                    break;
                 }
                 res = source.next_batch() => res,
            };

            match batch {
                Ok(signatures) => {
                    let start_time = std::time::Instant::now();
                    let mut processed_count = 0;

                    for event in signatures {
                        let signature = event.signature();
                        let sig_str = signature.to_string();

                        // Check if already processed (idempotency)
                        if self.storage.is_processed(&sig_str).await? {
                            continue;
                        }

                        // Optimization for LogEvents
                        match &event {
                            crate::streams::TransactionEvent::LogEvent {
                                logs,
                                err: None,
                                slot,
                                ..
                            } if self.config.indexing_mode.logs
                                && !self.config.indexing_mode.inputs
                                && !self.config.indexing_mode.accounts =>
                            {
                                // Parse logs directly
                                match self.decoder.parse_event_logs(logs) {
                                    Ok(parsed_events) => {
                                        let decoded =
                                            self.log_decoder_registry.decode_logs(&parsed_events);

                                        // Construct partial context for log optimization
                                        let context = TxMetadata {
                                            slot: *slot,
                                            block_time: None, // Not available in log event
                                            fee: 0,           // Not available
                                            pre_balances: vec![],
                                            post_balances: vec![],
                                            pre_token_balances: vec![],
                                            post_token_balances: vec![],
                                            signature: sig_str.clone(),
                                        };

                                        // Handle decoded events
                                        for (discriminator, event_data) in decoded {
                                            self.handler_registry
                                                .handle(
                                                    &discriminator,
                                                    &event_data,
                                                    &context,
                                                    self.storage.pool(),
                                                )
                                                .await?;
                                        }

                                        // Mark as processed
                                        self.storage.mark_processed(&sig_str, *slot).await?;

                                        processed_count += 1;
                                        continue;
                                    }
                                    Err(e) => {
                                        logging::log_error(
                                            "Log parsing error",
                                            &format!("{}: {}", sig_str, e),
                                        );
                                    }
                                }
                            }
                            _ => {}
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
        Ok(())
    }

    async fn process_hybrid_source(self) -> Result<()> {
        logging::log_startup(
            &self
                .config
                .program_ids
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<String>>()
                .join(", "),
            self.config.rpc_url(),
            0, // Real-time + Gap filling
        );

        // Run schema initializers
        for initializer in &self.schema_initializers {
            logging::log(logging::LogLevel::Info, "Initializing database schema...");
            initializer.initialize(self.storage.pool()).await?;
        }

        // Extract Hybrid config
        let (ws_url, rpc_url, poll_interval, reconnect_delay, gap_threshold) =
            match &self.config.source {
                SourceConfig::Hybrid {
                    ws_url,
                    rpc_url,
                    poll_interval_secs,
                    reconnect_delay_secs,
                    gap_threshold_slots,
                } => (
                    ws_url.clone(),
                    rpc_url.clone(),
                    *poll_interval_secs,
                    *reconnect_delay_secs,
                    *gap_threshold_slots,
                ),
                _ => {
                    return Err(crate::utils::error::SolanaIndexerError::ConfigError(
                        "Invalid source config".to_string(),
                    ));
                }
            };

        logging::log(
            logging::LogLevel::Info,
            &format!("Starting indexer loop (Hybrid: WS={ws_url}, RPC={rpc_url})...\n"),
        );

        let mut source = crate::streams::hybrid::HybridSource::new(
            ws_url,
            rpc_url,
            self.config.program_ids.clone(),
            poll_interval,
            reconnect_delay,
            gap_threshold,
        );

        loop {
            if self.cancellation_token.is_cancelled() {
                logging::log(logging::LogLevel::Info, "Graceful shutdown complete.");
                break;
            }

            let batch = tokio::select! {
                 _ = self.cancellation_token.cancelled() => {
                    logging::log(logging::LogLevel::Info, "Graceful shutdown initiated...");
                    break;
                 }
                 res = source.next_batch() => res,
            };

            match batch {
                Ok(signatures) => {
                    let start_time = std::time::Instant::now();
                    let mut processed_count = 0;

                    for event in signatures {
                        let signature = event.signature();
                        let sig_str = signature.to_string();

                        // Check if already processed (idempotency)
                        if self.storage.is_processed(&sig_str).await? {
                            continue;
                        }

                        // Optimization for LogEvents
                        match &event {
                            crate::streams::TransactionEvent::LogEvent {
                                logs,
                                err: None,
                                slot,
                                ..
                            } if self.config.indexing_mode.logs
                                && !self.config.indexing_mode.inputs
                                && !self.config.indexing_mode.accounts =>
                            {
                                // Parse logs directly
                                match self.decoder.parse_event_logs(logs) {
                                    Ok(parsed_events) => {
                                        let decoded =
                                            self.log_decoder_registry.decode_logs(&parsed_events);

                                        // Construct partial context for log optimization
                                        let context = TxMetadata {
                                            slot: *slot,
                                            block_time: None, // Not available in log event
                                            fee: 0,           // Not available
                                            pre_balances: vec![],
                                            post_balances: vec![],
                                            pre_token_balances: vec![],
                                            post_token_balances: vec![],
                                            signature: sig_str.clone(),
                                        };

                                        // Handle decoded events
                                        for (discriminator, event_data) in decoded {
                                            self.handler_registry
                                                .handle(
                                                    &discriminator,
                                                    &event_data,
                                                    &context,
                                                    self.storage.pool(),
                                                )
                                                .await?;
                                        }

                                        // Mark as processed
                                        self.storage.mark_processed(&sig_str, *slot).await?;

                                        processed_count += 1;
                                        continue;
                                    }
                                    Err(e) => {
                                        logging::log_error(
                                            "Log parsing error",
                                            &format!("{}: {}", sig_str, e),
                                        );
                                    }
                                }
                            }
                            _ => {}
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
                    logging::log_error("Hybrid Source Error", &e.to_string());
                    // Reconnection/Retries handled internally by HybridSource (WS/RPC)
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }

        Ok(())
    }

    async fn process_helius_source(self) -> Result<()> {
        logging::log_startup(
            &self
                .config
                .program_ids
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<String>>()
                .join(", "),
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
            if self.cancellation_token.is_cancelled() {
                logging::log(logging::LogLevel::Info, "Graceful shutdown complete.");
                break;
            }

            let batch = tokio::select! {
                 _ = self.cancellation_token.cancelled() => {
                    logging::log(logging::LogLevel::Info, "Graceful shutdown initiated...");
                    break;
                 }
                 res = source.next_batch() => res,
            };

            match batch {
                Ok(signatures) => {
                    let mut processed_count = 0;

                    for event in signatures {
                        let signature = event.signature();
                        let sig_str = signature.to_string();

                        // Check if already processed (idempotency)
                        if self.storage.is_processed(&sig_str).await? {
                            continue;
                        }

                        // Optimization: If indexing mode is Logs Only, decode logs directly
                        match &event {
                            crate::streams::TransactionEvent::LogEvent {
                                logs,
                                err: None,
                                slot,
                                ..
                            } if self.config.indexing_mode.logs
                                && !self.config.indexing_mode.inputs
                                && !self.config.indexing_mode.accounts =>
                            {
                                // Parse logs
                                match self.decoder.parse_event_logs(logs) {
                                    Ok(parsed_events) => {
                                        let decoded =
                                            self.log_decoder_registry.decode_logs(&parsed_events);

                                        // Construct partial context for log optimization
                                        let context = TxMetadata {
                                            slot: *slot,
                                            block_time: None, // Not available in log event
                                            fee: 0,           // Not available
                                            pre_balances: vec![],
                                            post_balances: vec![],
                                            pre_token_balances: vec![],
                                            post_token_balances: vec![],
                                            signature: sig_str.clone(),
                                        };

                                        // Handle decoded events
                                        for (discriminator, event_data) in decoded {
                                            self.handler_registry
                                                .handle(
                                                    &discriminator,
                                                    &event_data,
                                                    &context,
                                                    self.storage.pool(),
                                                )
                                                .await?;
                                        }
                                        // Mark as processed
                                        self.storage.mark_processed(&sig_str, *slot).await?;

                                        // Skip full processing
                                        processed_count += 1;
                                        continue;
                                    }
                                    Err(e) => {
                                        logging::log_error(
                                            "Log parsing error",
                                            &format!("{}: {}", sig_str, e),
                                        );
                                        // Fallback to full fetch?
                                    }
                                }
                            }
                            _ => {}
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
        Ok(())
    }

    async fn poll_and_process(&self, last_signature: &mut Option<Signature>) -> Result<usize> {
        // Fetch new signatures
        let signatures = self.fetch_signatures(last_signature.as_ref()).await?;

        if signatures.is_empty() {
            return Ok(0);
        }

        // Update last signature for next poll
        if let Some(first_event) = signatures.first() {
            *last_signature = Some(first_event.signature());
        }

        let concurrency = self.config.worker_threads;
        let semaphore = Arc::new(Semaphore::new(concurrency));
        let mut tasks = Vec::new();

        for event in signatures {
            let signature = event.signature();
            let sig_str = signature.to_string();

            // Check if already processed (idempotency)
            if self.storage.is_processed(&sig_str).await? {
                continue;
            }

            // Acquire permit
            let permit =
                semaphore.clone().acquire_owned().await.map_err(|e| {
                    SolanaIndexerError::InternalError(format!("Semaphore error: {e}"))
                })?;

            let fetcher = self.fetcher.clone();
            let decoder = self.decoder.clone();
            let decoder_registry = self.decoder_registry.clone();
            let log_decoder_registry = self.log_decoder_registry.clone();
            let account_decoder_registry = self.account_decoder_registry.clone();
            let handler_registry = self.handler_registry.clone();
            let storage = self.storage.clone();
            let config = self.config.clone();

            tasks.push(tokio::spawn(async move {
                let res = Self::process_transaction_core(
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
                    None,
                )
                .await;
                drop(permit);
                (sig_str, res)
            }));
        }

        let mut processed_count = 0;
        for task in tasks {
            match task.await {
                Ok((sig_str, res)) => match res {
                    Ok(()) => processed_count += 1,
                    Err(e) => {
                        logging::log_error("Transaction error", &format!("{sig_str}: {e}"));
                        eprintln!("Error processing transaction {sig_str}: {e}");
                    }
                },
                Err(e) => {
                    logging::log_error("Task join error", &e.to_string());
                }
            }
        }

        Ok(processed_count)
    }

    async fn fetch_signatures(
        &self,
        last_signature: Option<&Signature>,
    ) -> Result<Vec<crate::streams::TransactionEvent>> {
        use solana_client::rpc_client::RpcClient;
        use solana_sdk::commitment_config::CommitmentConfig;

        let rpc_url = self.config.rpc_url().to_string();
        let program_ids = self.config.program_ids.clone();
        let batch_size = self.config.batch_size;
        let last_sig = last_signature.copied();

        tokio::task::spawn_blocking(move || {
            let rpc_client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());
            let mut all_events = Vec::new();

            for program_id in program_ids {
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
                    .map_err(|e| {
                        crate::utils::error::SolanaIndexerError::RpcError(e.to_string())
                    })?;

                let events: Vec<crate::streams::TransactionEvent> = sigs
                    .into_iter()
                    .filter_map(|s| {
                        Signature::from_str(&s.signature).ok().map(|sig| {
                            crate::streams::TransactionEvent::Signature {
                                signature: sig,
                                slot: s.slot,
                            }
                        })
                    })
                    .collect();
                all_events.extend(events);
            }

            Ok(all_events)
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
        let context = TxMetadata {
            slot,
            block_time: transaction.block_time,
            fee: meta.fee,
            pre_balances: meta.pre_balances.clone(),
            post_balances: meta.post_balances.clone(),
            pre_token_balances: pre_token_balances
                .into_iter()
                .map(|b| TokenBalanceInfo {
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
                .map(|b| TokenBalanceInfo {
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
                        .handle(&discriminator, &event_data, &context, storage.pool())
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
                        .handle(&discriminator, &event_data, &context, storage.pool())
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
                                #[allow(clippy::collapsible_if)]
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
                                            &context,
                                            storage.pool(),
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
