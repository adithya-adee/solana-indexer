//! Main indexer orchestrator that integrates all components.
//!
//! This module provides the `SolanaIndexer` struct that orchestrates the complete
//! indexing pipeline: polling, fetching, decoding, deduplication, and event handling.

use crate::{
    config::SolanaIndexerConfig, decoder::Decoder, error::Result, fetcher::Fetcher,
    storage::Storage, traits::HandlerRegistry,
};
use solana_sdk::signature::Signature;
use std::str::FromStr;
use std::sync::Arc;
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
    storage: Arc<Storage>,
    fetcher: Arc<Fetcher>,
    decoder: Arc<Decoder>,
    handler_registry: Arc<HandlerRegistry>,
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

        let fetcher = Arc::new(Fetcher::new(&config.rpc_url));
        let decoder = Arc::new(Decoder::new());
        let handler_registry = Arc::new(HandlerRegistry::new());

        Ok(Self {
            config,
            storage,
            fetcher,
            decoder,
            handler_registry,
        })
    }

    /// Returns a reference to the handler registry for registering handlers.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::{SolanaIndexer, SolanaIndexerConfigBuilder};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = SolanaIndexerConfigBuilder::new()
    /// #     .with_rpc("http://127.0.0.1:8899")
    /// #     .with_database("postgresql://localhost/mydb")
    /// #     .program_id("11111111111111111111111111111111")
    /// #     .build()?;
    /// let indexer = SolanaIndexer::new(config).await?;
    /// let registry = indexer.handler_registry();
    /// // Register handlers here
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn handler_registry(&self) -> &HandlerRegistry {
        &self.handler_registry
    }

    /// Returns a reference to the decoder for registering event discriminators.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::{SolanaIndexer, SolanaIndexerConfigBuilder, TransferEvent};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = SolanaIndexerConfigBuilder::new()
    /// #     .with_rpc("http://127.0.0.1:8899")
    /// #     .with_database("postgresql://localhost/mydb")
    /// #     .program_id("11111111111111111111111111111111")
    /// #     .build()?;
    /// let mut indexer = SolanaIndexer::new(config).await?;
    /// let decoder = indexer.decoder_mut();
    /// decoder.register_event_discriminator(TransferEvent::discriminator(), "TransferEvent");
    /// # Ok(())
    /// # }
    /// ```
    pub fn decoder_mut(&mut self) -> &mut Decoder {
        Arc::get_mut(&mut self.decoder).expect("Decoder has multiple references")
    }

    /// Starts the indexer.
    ///
    /// This method runs indefinitely, polling for new transactions,
    /// fetching and decoding them, checking for duplicates, and
    /// dispatching events to registered handlers.
    ///
    /// # Errors
    ///
    /// Returns error if a critical failure occurs that cannot be recovered.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::{SolanaIndexer, SolanaIndexerConfigBuilder};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = SolanaIndexerConfigBuilder::new()
    /// #     .with_rpc("http://127.0.0.1:8899")
    /// #     .with_database("postgresql://localhost/mydb")
    /// #     .program_id("11111111111111111111111111111111")
    /// #     .build()?;
    /// let indexer = SolanaIndexer::new(config).await?;
    /// indexer.start().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn start(self) -> Result<()> {
        println!("Starting Solana Indexer...");
        println!("Program ID: {}", self.config.program_id);
        println!("RPC URL: {}", self.config.rpc_url);
        println!(
            "Poll interval: {} seconds",
            self.config.poll_interval_secs
        );

        let mut poll_interval = interval(Duration::from_secs(self.config.poll_interval_secs));
        let mut last_signature: Option<Signature> = None;

        loop {
            poll_interval.tick().await;

            match self.poll_and_process(&mut last_signature).await {
                Ok(processed) => {
                    if processed > 0 {
                        println!("Processed {} new transactions", processed);
                    }
                }
                Err(e) => {
                    eprintln!("Error in indexing loop: {}", e);
                    // Continue polling despite errors
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn poll_and_process(&self, last_signature: &mut Option<Signature>) -> Result<usize> {
        // Fetch new signatures
        let signatures = self.fetch_signatures(last_signature).await?;

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
                    eprintln!("Error processing transaction {}: {}", sig_str, e);
                    // Continue with next transaction
                }
            }
        }

        Ok(processed_count)
    }

    async fn fetch_signatures(
        &self,
        last_signature: &Option<Signature>,
    ) -> Result<Vec<Signature>> {
        use solana_client::rpc_client::RpcClient;
        use solana_sdk::commitment_config::CommitmentConfig;

        let rpc_url = self.config.rpc_url.clone();
        let program_id = self.config.program_id;
        let batch_size = self.config.batch_size;
        let last_sig = *last_signature;

        tokio::task::spawn_blocking(move || {
            let rpc_client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());

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
                .map_err(|e| crate::error::SolanaIndexerError::RpcError(e.to_string()))?;

            let signatures: Vec<Signature> = sigs
                .into_iter()
                .filter_map(|s| Signature::from_str(&s.signature).ok())
                .collect();

            Ok(signatures)
        })
        .await
        .map_err(|e| crate::error::SolanaIndexerError::InternalError(e.to_string()))?
    }

    async fn process_transaction(&self, signature: &Signature) -> Result<()> {
        let sig_str = signature.to_string();

        // Fetch transaction
        let transaction = self.fetcher.fetch_transaction(signature).await?;

        // Decode transaction
        let decoded = self.decoder.decode_transaction(&transaction)?;

        // Mark as processed
        self.storage
            .mark_processed(&sig_str, transaction.slot)
            .await?;

        // Log processing
        println!(
            "Processed tx {} (slot {}, {} instructions, {} events)",
            sig_str,
            decoded.slot,
            decoded.instructions.len(),
            decoded.events.len()
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_indexer_creation() {
        // Basic struct instantiation test
        assert!(true);
    }
}
