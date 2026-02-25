//! Polling-based input source for `SolStream`.
//!
//! This module implements a polling strategy that periodically queries
//! Solana RPC endpoints for new transaction signatures.

use crate::config::SolanaIndexerConfig;
use crate::core::decoding::Decoder;
use crate::core::execution::fetcher::Fetcher;
use crate::utils::error::{Result, SolanaIndexerError};
use solana_client::rpc_client::{GetConfirmedSignaturesForAddress2Config, RpcClient};
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::signature::Signature;
use std::time::Duration;
use tokio::time;

/// Polling-based input source for acquiring transaction signatures.
///
/// The `Poller` periodically queries an RPC endpoint for new transactions
/// related to a specific program ID. This is ideal for local development
/// and moderate-throughput scenarios.
///
/// # Example
///
/// ```no_run
/// # use solana_indexer_sdk::{Poller, SolanaIndexerConfigBuilder};
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = SolanaIndexerConfigBuilder::new()
///     .with_rpc("http://127.0.0.1:8899")
///     .with_database("postgresql://localhost/db")
///     .program_id("11111111111111111111111111111111")
///     .build()?;
///
/// let poller = Poller::new(config);
/// // poller.start().await?;
/// # Ok(())
/// # }
/// ```
pub struct Poller {
    /// Configuration for the poller
    config: SolanaIndexerConfig,

    /// Last processed signature (for pagination)
    last_signature: Option<Signature>,
}

impl Poller {
    /// Creates a new `Poller` instance.
    ///
    /// # Arguments
    ///
    /// * `config` - Configuration containing RPC URL, program ID, and polling settings
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer_sdk::{Poller, SolanaIndexerConfigBuilder};
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = SolanaIndexerConfigBuilder::new()
    ///     .with_rpc("http://127.0.0.1:8899")
    ///     .with_database("postgresql://localhost/db")
    ///     .program_id("11111111111111111111111111111111")
    ///     .build()?;
    ///
    /// let poller = Poller::new(config);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn new(config: SolanaIndexerConfig) -> Self {
        let last_signature = match config.start_strategy {
            crate::config::StartStrategy::Signature(sig) => Some(sig),
            _ => None, // Will be initialized by the indexer
        };
        Self {
            config,
            last_signature,
        }
    }

    /// Fetches new transaction signatures for the configured program.
    ///
    /// This method queries the RPC endpoint for signatures related to the
    /// program ID, using pagination to avoid re-processing old transactions.
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::RpcError` if the RPC request fails.
    ///
    /// # Returns
    ///
    /// A vector of new transaction signatures to process.
    pub async fn fetch_new_signatures(&mut self) -> Result<Vec<Signature>> {
        // Capture values needed for the blocking task
        let program_ids = self.config.program_ids.clone();
        let batch_size = self.config.batch_size;
        let last_sig = self.last_signature;
        let rpc_url = self.config.rpc_url().to_string();

        let signatures = tokio::task::spawn_blocking(move || {
            // Create RPC client in the blocking task
            let rpc_client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());
            let mut all_sigs: Vec<
                solana_client::rpc_response::RpcConfirmedTransactionStatusWithSignature,
            > = Vec::new();

            for program_id in program_ids {
                let config = GetConfirmedSignaturesForAddress2Config {
                    before: None,
                    until: last_sig,
                    limit: Some(batch_size),
                    commitment: Some(CommitmentConfig::confirmed()),
                };

                let sigs = rpc_client
                    .get_signatures_for_address_with_config(&program_id, config)
                    .map_err(|e| {
                        SolanaIndexerError::RpcError(format!("Failed to fetch signatures: {e}"))
                    })?;
                all_sigs.extend(sigs);
            }
            Ok::<_, SolanaIndexerError>(all_sigs)
        })
        .await
        .map_err(|e| SolanaIndexerError::InternalError(format!("Task join error: {e}")))??;

        // Extract signatures and update last_signature for pagination
        let sigs: Vec<Signature> = signatures
            .iter()
            .filter_map(|info| info.signature.parse().ok())
            .collect();

        if let Some(first_sig) = sigs.first() {
            self.last_signature = Some(*first_sig);
        }

        Ok(sigs)
    }

    /// Starts the polling loop.
    ///
    /// This method runs indefinitely, periodically fetching new signatures
    /// at the configured polling interval. In a complete implementation,
    /// this would integrate with the fetcher, decoder, and handler pipeline.
    ///
    /// # Errors
    ///
    /// Returns errors from RPC operations or processing failures.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer_sdk::{Poller, SolanaIndexerConfigBuilder};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = SolanaIndexerConfigBuilder::new()
    ///     .with_rpc("http://127.0.0.1:8899")
    ///     .with_database("postgresql://localhost/db")
    ///     .program_id("11111111111111111111111111111111")
    ///     .build()?;
    ///
    /// let mut poller = Poller::new(config);
    /// // This will run indefinitely
    /// // poller.start().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn start(&mut self) -> Result<()> {
        let poll_interval = Duration::from_secs(self.config.poll_interval_secs);
        let mut interval = time::interval(poll_interval);

        // Initialize fetcher and decoder
        let fetcher = Fetcher::new(self.config.rpc_url(), self.config.commitment_level.into());
        let decoder = Decoder::new();

        tracing::info!("Starting poller with RPC: {}", self.config.rpc_url());
        tracing::info!(
            "Monitoring programs: {}",
            self.config
                .program_ids
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<String>>()
                .join(", ")
        );

        loop {
            interval.tick().await;

            match self.fetch_new_signatures().await {
                Ok(signatures) => {
                    if !signatures.is_empty() {
                        tracing::info!("Fetched {} new signatures", signatures.len());

                        // Process each signature through the pipeline
                        for signature in &signatures {
                            match self
                                .process_transaction(&fetcher, &decoder, signature)
                                .await
                            {
                                Ok(()) => {
                                    tracing::info!("✓ Processed transaction: {signature}");
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "✗ Error processing transaction {signature}: {e}"
                                    );
                                    // Continue processing other transactions
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    if let SolanaIndexerError::RpcError(ref msg) = e {
                        tracing::error!("RPC failure in poller (Exiting): {msg}");
                        return Err(e);
                    }
                    tracing::error!("Error fetching signatures: {e}");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    /// Processes a single transaction through the fetcher and decoder pipeline.
    ///
    /// # Arguments
    ///
    /// * `fetcher` - The fetcher instance for retrieving transaction data
    /// * `decoder` - The decoder instance for parsing transaction data
    /// * `signature` - The transaction signature to process
    ///
    /// # Errors
    ///
    /// Returns errors from fetching or decoding operations.
    async fn process_transaction(
        &self,
        fetcher: &Fetcher,
        decoder: &Decoder,
        signature: &Signature,
    ) -> Result<()> {
        // Fetch transaction data
        let transaction = fetcher.fetch_transaction(signature).await?;

        // Decode transaction
        let decoded_tx = decoder.decode_transaction(&transaction)?;

        // Log decoded information
        tracing::debug!("  Slot: {}", decoded_tx.slot);
        tracing::debug!("  Instructions: {}", decoded_tx.instructions.len());
        tracing::debug!("  Events: {}", decoded_tx.events.len());

        if let Some(compute_units) = decoded_tx.compute_units_consumed {
            tracing::debug!("  Compute units: {compute_units}");
        }

        // Display instruction details
        for instruction in &decoded_tx.instructions {
            tracing::debug!(
                "    - Instruction {}: {} ({})",
                instruction.index,
                instruction.instruction_type,
                instruction.program_id
            );
        }

        // Display event details
        for event in &decoded_tx.events {
            tracing::debug!("    - Event: {:?}", event.event_type);
            if let Some(data) = &event.data {
                tracing::debug!("      Data: {data}");
            }
        }

        // TODO: Pass to idempotency tracker
        // TODO: Pass to event handlers
        // TODO: Mark as processed

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_poller_creation() {
        let config = SolanaIndexerConfig {
            database_url: "postgresql://localhost/db".to_string(),
            program_ids: vec![solana_sdk::pubkey::Pubkey::default()],
            accounts_to_decode: vec![],
            poll_interval_secs: 5,
            batch_size: 100,
            source: crate::config::SourceConfig::Rpc {
                rpc_url: "http://127.0.0.1:8899".to_string(),
                poll_interval_secs: 5,
                batch_size: 100,
            },
            indexing_mode: crate::config::IndexingMode::inputs(),
            start_strategy: crate::config::StartStrategy::Latest,
            backfill: Default::default(),
            registry: Default::default(),
            stale_tentative_threshold: 1000,
            worker_threads: 10,
            commitment_level: crate::config::CommitmentLevel::Confirmed,
            retry: Default::default(),
        };

        let poller = Poller::new(config);
        assert!(poller.last_signature.is_none());
    }
}
