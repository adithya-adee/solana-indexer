//! Main indexer orchestrator that integrates all components.
//!
//! This module provides the `SolanaIndexer` struct that orchestrates the complete
//! indexing pipeline: polling, fetching, decoding, deduplication, and event handling.

use crate::config::SourceConfig;
use crate::{
    config::{IndexingMode, SolanaIndexerConfig, StartStrategy},
    core::{
        account_registry::AccountDecoderRegistry, decoder::Decoder, fetcher::Fetcher,
        log_registry::LogDecoderRegistry, registry::DecoderRegistry,
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
        let decoder_registry = Arc::new(DecoderRegistry::new());
        let log_decoder_registry = Arc::new(LogDecoderRegistry::new());
        let account_decoder_registry = Arc::new(AccountDecoderRegistry::new());
        let handler_registry = Arc::new(HandlerRegistry::new());

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
        let decoder_registry = Arc::new(DecoderRegistry::new());
        let log_decoder_registry = Arc::new(LogDecoderRegistry::new());
        let account_decoder_registry = Arc::new(AccountDecoderRegistry::new());
        let handler_registry = Arc::new(HandlerRegistry::new());

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
                                handler_registry,
                                storage,
                                config,
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
            self.handler_registry.clone(),
            self.storage.clone(),
            self.config.clone(),
        )
        .await
    }

    /// Static implementation of transaction processing to allow concurrent execution.
    #[allow(clippy::too_many_arguments)]
    async fn process_transaction_core(
        signature: Signature,
        fetcher: Arc<Fetcher>,
        decoder: Arc<Decoder>,
        decoder_registry: Arc<DecoderRegistry>,
        log_decoder_registry: Arc<LogDecoderRegistry>,
        handler_registry: Arc<HandlerRegistry>,
        storage: Arc<dyn StorageBackend>,
        config: SolanaIndexerConfig,
    ) -> Result<()> {
        let sig_str = signature.to_string();

        // Fetch transaction
        let transaction = fetcher.fetch_transaction(&signature).await?;

        // Decode transaction metadata (slot, etc.)
        let decoded_meta = decoder.decode_transaction(&transaction)?;

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
        if matches!(
            config.indexing_mode,
            IndexingMode::Inputs | IndexingMode::All
        ) {
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

        if matches!(config.indexing_mode, IndexingMode::Logs | IndexingMode::All) {
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
        }

        // Mark as processed
        storage.mark_processed(&sig_str, transaction.slot).await?;

        // Log processing with colorful output
        if events_processed > 0 {
            crate::utils::logging::log_transaction(&sig_str, decoded_meta.slot, events_processed);
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
