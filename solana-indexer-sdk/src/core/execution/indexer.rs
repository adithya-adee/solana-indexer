//! Main indexer orchestrator that integrates all components.
//!
//! This module provides the `SolanaIndexer` struct that orchestrates the complete
//! indexing pipeline: polling, fetching, decoding, deduplication, and event handling.

use crate::config::SourceConfig;
use crate::{
    config::{SolanaIndexerConfig, StartStrategy},
    core::{
        backfill::defaults::*, backfill::engine::BackfillEngine,
        backfill::manager::BackfillManager, decoding::Decoder, execution::fetcher::Fetcher,
        registry::account::AccountDecoderRegistry, registry::logs::LogDecoderRegistry,
        registry::DecoderRegistry,
    },
    storage::{Storage, StorageBackend},
    streams::TransactionSource,
    types::{
        backfill_traits::{
            BackfillHandlerRegistry, BackfillRange, BackfillTrigger, FinalizedBlockTracker,
        },
        metadata::{TokenBalanceInfo, TxMetadata},
        traits::{HandlerRegistry, SchemaInitializer},
    },
    utils::{
        error::{Result, SolanaIndexerError},
        logging,
    },
};

#[cfg(feature = "helius")]
use crate::streams::helius::HeliusSource;

#[cfg(feature = "websockets")]
use crate::streams::websocket::WebSocketSource;
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
    backfill_handler_registry: Arc<BackfillHandlerRegistry>,
    backfill_trigger: Option<Arc<dyn BackfillTrigger>>,
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
        let backfill_handler_registry =
            Arc::new(BackfillHandlerRegistry::new_bounded(&config.registry));

        Ok(Self {
            config,
            storage,
            fetcher,
            decoder,
            decoder_registry,
            log_decoder_registry,
            account_decoder_registry,
            handler_registry,
            backfill_handler_registry,
            backfill_trigger: None,
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
        let backfill_handler_registry =
            Arc::new(BackfillHandlerRegistry::new_bounded(&config.registry));

        Self {
            config,
            storage,
            fetcher,
            decoder,
            decoder_registry,
            log_decoder_registry,
            account_decoder_registry,
            handler_registry,
            backfill_handler_registry,
            backfill_trigger: None,
            schema_initializers: Vec::new(),
            cancellation_token: tokio_util::sync::CancellationToken::new(),
        }
    }

    /// Returns a reference to the handler registry for registering handlers.
    #[must_use]
    pub fn config(&self) -> &SolanaIndexerConfig {
        &self.config
    }

    /// Returns a reference to the handler registry for registering handlers.
    #[must_use]
    pub fn handler_registry(&self) -> &HandlerRegistry {
        &self.handler_registry
    }

    /// Returns a mutable reference to the handler registry.
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::InternalError` if the registry has multiple references.
    pub fn handler_registry_mut(&mut self) -> Result<&mut HandlerRegistry> {
        Arc::get_mut(&mut self.handler_registry).ok_or_else(|| {
            SolanaIndexerError::InternalError("HandlerRegistry has multiple references".to_string())
        })
    }

    /// Returns a reference to the decoder registry.
    #[must_use]
    pub fn decoder_registry(&self) -> &DecoderRegistry {
        &self.decoder_registry
    }

    /// Returns a mutable reference to the decoder registry.
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::InternalError` if the registry has multiple references.
    pub fn decoder_registry_mut(&mut self) -> Result<&mut DecoderRegistry> {
        Arc::get_mut(&mut self.decoder_registry).ok_or_else(|| {
            SolanaIndexerError::InternalError("DecoderRegistry has multiple references".to_string())
        })
    }

    /// Returns a reference to the log decoder registry.
    #[must_use]
    pub fn log_decoder_registry(&self) -> &LogDecoderRegistry {
        &self.log_decoder_registry
    }

    /// Returns a mutable reference to the log decoder registry.
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::InternalError` if the registry has multiple references.
    pub fn log_decoder_registry_mut(&mut self) -> Result<&mut LogDecoderRegistry> {
        Arc::get_mut(&mut self.log_decoder_registry).ok_or_else(|| {
            SolanaIndexerError::InternalError(
                "LogDecoderRegistry has multiple references".to_string(),
            )
        })
    }

    /// Returns a reference to the account decoder registry.
    #[must_use]
    pub fn account_decoder_registry(&self) -> &AccountDecoderRegistry {
        &self.account_decoder_registry
    }

    /// Returns a mutable reference to the account decoder registry.
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::InternalError` if the registry has multiple references.
    pub fn account_decoder_registry_mut(&mut self) -> Result<&mut AccountDecoderRegistry> {
        Arc::get_mut(&mut self.account_decoder_registry).ok_or_else(|| {
            SolanaIndexerError::InternalError(
                "AccountDecoderRegistry has multiple references".to_string(),
            )
        })
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
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::RegistryCapacityExceeded` if the registry is full.
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
    /// # fn example(indexer: &mut SolanaIndexer) -> Result<(), Box<dyn std::error::Error>> {
    /// indexer.register_decoder("program_id", MyDecoder)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn register_decoder<D, E>(
        &mut self,
        program_id: impl Into<String>,
        decoder: D,
    ) -> Result<()>
    where
        D: crate::types::traits::InstructionDecoder<E> + 'static,
        E: crate::types::events::EventDiscriminator + borsh::BorshSerialize + Send + Sync + 'static,
    {
        use crate::types::traits::DynamicInstructionDecoder;
        let boxed_typed: Box<dyn crate::types::traits::InstructionDecoder<E>> = Box::new(decoder);
        let boxed_dynamic: Box<dyn DynamicInstructionDecoder> = Box::new(boxed_typed);
        self.decoder_registry_mut()?
            .register(program_id.into(), boxed_dynamic)?;
        self.config.indexing_mode.inputs = true;
        Ok(())
    }

    /// Registers a typed log decoder and enables log indexing mode.
    pub fn register_log_decoder<D, E>(
        &mut self,
        program_id: impl Into<String>,
        decoder: D,
    ) -> Result<()>
    where
        D: crate::types::traits::LogDecoder<E> + 'static,
        E: crate::types::events::EventDiscriminator + borsh::BorshSerialize + Send + Sync + 'static,
    {
        use crate::types::traits::DynamicLogDecoder;
        let boxed_typed: Box<dyn crate::types::traits::LogDecoder<E>> = Box::new(decoder);
        let boxed_dynamic: Box<dyn DynamicLogDecoder> = Box::new(boxed_typed);
        self.log_decoder_registry_mut()?
            .register(program_id.into(), boxed_dynamic)?;
        self.config.indexing_mode.logs = true;
        Ok(())
    }

    /// Registers a typed account decoder and enables account indexing mode.
    pub fn register_account_decoder<D, E>(&mut self, decoder: D) -> Result<()>
    where
        D: crate::types::traits::AccountDecoder<E> + 'static,
        E: crate::types::events::EventDiscriminator + borsh::BorshSerialize + Send + Sync + 'static,
    {
        use crate::types::traits::DynamicAccountDecoder;
        let boxed: Box<dyn crate::types::traits::AccountDecoder<E>> = Box::new(decoder);
        let dynamic_boxed: Box<dyn DynamicAccountDecoder> = Box::new(boxed);
        self.account_decoder_registry_mut()?
            .register(dynamic_boxed)?;
        self.config.indexing_mode.accounts = true;
        Ok(())
    }

    /// Returns a mutable reference to the backfill handler registry.
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::InternalError` if the registry has multiple references.
    pub fn backfill_handler_registry_mut(&mut self) -> Result<&mut BackfillHandlerRegistry> {
        Arc::get_mut(&mut self.backfill_handler_registry).ok_or_else(|| {
            SolanaIndexerError::InternalError(
                "BackfillHandlerRegistry has multiple references".to_string(),
            )
        })
    }

    /// Registers a typed backfill handler.
    ///
    /// This generic method automatically handles the boxing and type erasure required by the registry.
    ///
    /// # Arguments
    ///
    /// * `handler` - The typed backfill handler instance
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer_sdk::{SolanaIndexer, TxMetadata};
    /// # use async_trait::async_trait;
    /// # struct MyBackfillHandler;
    /// # struct MyEvent;
    /// # #[async_trait]
    /// # impl solana_indexer_sdk::BackfillHandler<MyEvent> for MyBackfillHandler {
    /// #   async fn handle_backfill(&self, _: MyEvent, _: &TxMetadata, _: &sqlx::PgPool) -> solana_indexer_sdk::Result<()> { Ok(()) }
    /// # }
    /// # impl solana_indexer_sdk::EventDiscriminator for MyEvent { fn discriminator() -> [u8; 8] { [0; 8] } }
    /// # impl borsh::BorshDeserialize for MyEvent {
    /// #   fn deserialize(buf: &mut &[u8]) -> std::io::Result<Self> { Ok(MyEvent) }
    /// #   fn deserialize_reader<R: std::io::Read>(_: &mut R) -> std::io::Result<Self> { Ok(MyEvent) }
    /// # }
    /// # fn example(indexer: &mut SolanaIndexer) -> Result<(), Box<dyn std::error::Error>> {
    /// indexer.register_backfill_handler(MyBackfillHandler)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn register_backfill_handler<H, E>(&mut self, handler: H) -> Result<()>
    where
        H: crate::types::backfill_traits::BackfillHandler<E> + 'static,
        E: crate::types::events::EventDiscriminator
            + borsh::BorshDeserialize
            + Send
            + Sync
            + 'static,
    {
        use crate::types::backfill_traits::DynamicBackfillHandler;
        let boxed_typed: Box<dyn crate::types::backfill_traits::BackfillHandler<E>> =
            Box::new(handler);
        let boxed_dynamic: Box<dyn DynamicBackfillHandler> = Box::new(boxed_typed);

        self.backfill_handler_registry_mut()?
            .register(E::discriminator(), boxed_dynamic)
    }

    /// Sets a custom backfill trigger.
    ///
    /// If not set, a default trigger will be used based on `BackfillConfig`.
    ///
    /// # Arguments
    ///
    /// * `trigger` - The backfill trigger implementation
    pub fn with_backfill_trigger(&mut self, trigger: Arc<dyn BackfillTrigger>) -> Result<()> {
        self.backfill_trigger = Some(trigger);
        Ok(())
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
    /// # fn example(indexer: &mut SolanaIndexer) -> Result<(), Box<dyn std::error::Error>> {
    /// indexer.register_handler(MyHandler)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn register_handler<H, E>(&mut self, handler: H) -> Result<()>
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

        self.handler_registry_mut()?
            .register(E::discriminator(), boxed_dynamic)
    }

    /// Returns a reference to the decoder for registering event discriminators.
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::InternalError` if the decoder has multiple references.
    pub fn decoder_mut(&mut self) -> Result<&mut Decoder> {
        Arc::get_mut(&mut self.decoder).ok_or_else(|| {
            SolanaIndexerError::InternalError("Decoder has multiple references".to_string())
        })
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
            self.cancellation_token.clone(),
            self.backfill_handler_registry.clone(),
        );

        engine.start().await
    }

    /// Manually backfill a specific slot range.
    ///
    /// This is useful when integrating the SDK into a web framework (Axum, Actix, etc.)
    /// and exposing an endpoint that triggers ad-hoc backfill jobs.
    ///
    /// - `from_slot` (inclusive): the starting slot to backfill from.
    /// - `to_slot` (inclusive, optional): the ending slot to backfill to. If `None`,
    ///   the latest finalized slot will be used.
    pub async fn backfill_slots(&self, from_slot: u64, to_slot: Option<u64>) -> Result<()> {
        // Resolve the target end slot if not provided explicitly.
        let finalized_tracker = Arc::new(DefaultFinalizedBlockTracker);
        let effective_end_slot = if let Some(slot) = to_slot {
            slot
        } else {
            finalized_tracker
                .get_latest_finalized_slot(&self.fetcher)
                .await?
        };

        // No-op if the requested range is empty.
        if effective_end_slot < from_slot {
            return Ok(());
        }

        // Reuse the same defaults as `start_backfill` for strategy and helpers.
        let backfill_config = self.config.backfill.clone();
        let strategy = Arc::new(DefaultBackfillStrategy {
            start_slot: Some(from_slot),
            end_slot: Some(effective_end_slot),
            batch_size: backfill_config.batch_size,
            concurrency: backfill_config.concurrency,
        });

        let reorg_handler = Arc::new(DefaultReorgHandler);
        let progress_tracker = Arc::new(DefaultBackfillProgress);

        let engine = BackfillEngine::new(
            self.config.clone(),
            self.fetcher.clone(),
            self.decoder.clone(),
            self.decoder_registry.clone(),
            self.log_decoder_registry.clone(),
            self.account_decoder_registry.clone(),
            // For ad-hoc backfill we only care about backfill handlers,
            // so use an empty live handler registry.
            Arc::new(HandlerRegistry::new()),
            self.storage.clone(),
            strategy,
            reorg_handler,
            finalized_tracker,
            progress_tracker,
            self.cancellation_token.clone(),
            self.backfill_handler_registry.clone(),
        );

        engine
            .start_range(BackfillRange::new(from_slot, effective_end_slot))
            .await
    }

    /// Starts the indexer.
    ///
    /// This method runs indefinitely, fetching and processing transactions
    /// based on the configured source (RPC polling or WebSocket subscription).
    /// If backfill is enabled, it also starts a BackfillManager that runs
    /// continuously alongside the live indexer.
    ///
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

        // Start BackfillManager if enabled
        if self.config.backfill.enabled {
            let backfill_config = self.config.backfill.clone();
            let backfill_fetcher = self.fetcher.clone();
            let backfill_decoder = self.decoder.clone();
            let backfill_storage = self.storage.clone();
            let backfill_strategy = Arc::new(DefaultBackfillStrategy {
                start_slot: backfill_config.start_slot,
                end_slot: backfill_config.end_slot,
                batch_size: backfill_config.batch_size,
                concurrency: backfill_config.concurrency,
            });
            let backfill_reorg_handler = Arc::new(DefaultReorgHandler);
            let backfill_finalized_tracker = Arc::new(DefaultFinalizedBlockTracker);
            let backfill_progress_tracker = Arc::new(DefaultBackfillProgress);
            let backfill_trigger = self.backfill_trigger.clone().unwrap_or_else(|| {
                Arc::new(DefaultBackfillTrigger::new(backfill_config.clone()))
                    as Arc<dyn BackfillTrigger>
            });
            let backfill_handlers = self.backfill_handler_registry.clone();
            let backfill_cancellation = self.cancellation_token.clone();
            let backfill_decoder_registry = self.decoder_registry.clone();
            let backfill_log_decoder_registry = self.log_decoder_registry.clone();
            let backfill_account_decoder_registry = self.account_decoder_registry.clone();

            let manager = BackfillManager::new(
                self.config.clone(),
                backfill_fetcher,
                backfill_decoder,
                backfill_storage,
                backfill_strategy,
                backfill_reorg_handler,
                backfill_finalized_tracker,
                backfill_progress_tracker,
                backfill_trigger,
                backfill_handlers,
                backfill_cancellation,
                backfill_decoder_registry,
                backfill_log_decoder_registry,
                backfill_account_decoder_registry,
            );

            tokio::spawn(async move {
                if let Err(e) = manager.run().await {
                    logging::log_error("BackfillManager error", &e.to_string());
                }
            });
        }

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
            #[cfg(feature = "websockets")]
            SourceConfig::WebSocket { .. } => self.process_websocket_source().await,
            #[cfg(feature = "helius")]
            SourceConfig::Helius { use_websocket, .. } => {
                if *use_websocket {
                    self.process_helius_source().await
                } else {
                    self.process_rpc_source().await
                }
            }
            #[cfg(feature = "websockets")]
            SourceConfig::Hybrid { .. } => self.process_hybrid_source().await,
            #[cfg(feature = "laserstream")]
            SourceConfig::Laserstream { .. } => self.process_laserstream_source().await,
        }
    }

    /// Internal method to run the Laserstream (gRPC) source loop.
    #[cfg(feature = "laserstream")]
    async fn process_laserstream_source(self) -> Result<()> {
        use crate::streams::laserstream::LaserstreamSource;

        logging::log(logging::LogLevel::Info, "Starting Laserstream source...");

        let mut source = LaserstreamSource::new(self.config.clone()).await?;

        // Initialize schema
        for initializer in &self.schema_initializers {
            initializer.initialize(self.storage.pool()).await?;
        }

        loop {
            match source.next_batch().await {
                Ok(events) => {
                    if !events.is_empty() {
                        // Process events
                        // Laserstream (gRPC) likely provides full transaction data,
                        // so we should leverage that to avoid RPC fetches.
                        // For now, we reuse process_transaction_core which can handle full transactions.

                        for event in events {
                            // Clone dependencies for each task/call
                            let fetcher = self.fetcher.clone();
                            let decoder = self.decoder.clone();
                            let decoder_registry = self.decoder_registry.clone();
                            let log_decoder_registry = self.log_decoder_registry.clone();
                            let account_decoder_registry = self.account_decoder_registry.clone();
                            let handler_registry = self.handler_registry.clone();
                            let storage = self.storage.clone();
                            let config = self.config.clone();

                            match event {
                                crate::streams::TransactionEvent::Signature {
                                    signature,
                                    slot: _,
                                } => {
                                    // Fallback if full tx not available
                                    let _ = Self::process_transaction_core(
                                        signature,
                                        fetcher,
                                        decoder,
                                        decoder_registry,
                                        log_decoder_registry,
                                        account_decoder_registry,
                                        handler_registry,
                                        storage,
                                        config,
                                        true, // Assuming confirmed/finalized from stream
                                        None,
                                        None,
                                    )
                                    .await;
                                }
                                crate::streams::TransactionEvent::FullTransaction {
                                    signature,
                                    slot: _,
                                    tx,
                                } => {
                                    let _ = Self::process_transaction_core(
                                        signature,
                                        fetcher,
                                        decoder,
                                        decoder_registry,
                                        log_decoder_registry,
                                        account_decoder_registry,
                                        handler_registry,
                                        storage,
                                        config,
                                        true, // Assuming confirmed/finalized from stream
                                        None,
                                        Some(tx),
                                    )
                                    .await;
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Err(e) => {
                    logging::log_error("Laserstream error", &e.to_string());
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
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
    #[cfg(feature = "websockets")]
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

    #[cfg(feature = "websockets")]
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

    #[cfg(feature = "helius")]
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

                        let preloaded_tx = match &event {
                            crate::streams::TransactionEvent::FullTransaction { tx, .. } => {
                                Some(tx.clone())
                            }
                            _ => None,
                        };

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
                                preloaded_tx,
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
                    None, // preloaded_transaction
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
            None, // preloaded_transaction
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
        preloaded_transaction: Option<
            Arc<solana_transaction_status::EncodedConfirmedTransactionWithStatusMeta>,
        >,
    ) -> Result<()> {
        let sig_str = signature.to_string();

        // Fetch transaction if not preloaded
        let transaction = if let Some(tx) = preloaded_transaction {
            tx
        } else {
            Arc::new(fetcher.fetch_transaction(&signature).await?)
        };

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
                        for (index, account_option) in accounts.iter().enumerate() {
                            if let Some(account) = account_option {
                                let pubkey = &keys[index];
                                let decoded_list =
                                    account_decoder_registry.decode_account(pubkey, account);
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
    async fn test_indexer_creation_rpc() -> Result<()> {
        let config = SolanaIndexerConfigBuilder::new()
            .with_rpc("http://127.0.0.1:8899")
            .with_database("postgresql://localhost/db")
            .program_id("11111111111111111111111111111111")
            .build()?;

        // We can't fully instantiate SolanaIndexer without a real DB, so we verify config
        assert_eq!(config.rpc_url(), "http://127.0.0.1:8899");
        match config.source {
            SourceConfig::Rpc { .. } => {}
            _ => panic!("Expected RPC source"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_indexer_creation_ws() -> Result<()> {
        let config = SolanaIndexerConfigBuilder::new()
            .with_ws("ws://127.0.0.1:8900", "http://127.0.0.1:8899")
            .with_database("postgresql://localhost/db")
            .program_id("11111111111111111111111111111111")
            .build()?;

        assert_eq!(config.rpc_url(), "http://127.0.0.1:8899");
        match config.source {
            SourceConfig::WebSocket { ws_url, .. } => assert_eq!(ws_url, "ws://127.0.0.1:8900"),
            _ => panic!("Expected WebSocket source"),
        }
        Ok(())
    }
}
