//! Polling-based input source for `SolStream`.
//!
//! This module implements a polling strategy that periodically queries
//! Solana RPC endpoints for new transaction signatures.

use crate::common::config::SolanaIndexerConfig;
use crate::common::error::{Result, SolanaIndexerError};
use crate::decoder::Decoder;
use crate::fetcher::Fetcher;
use solana_client::rpc_client::RpcClient;
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
/// # use solana_indexer::{Poller, SolanaIndexerConfigBuilder};
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
    /// # use solana_indexer::{Poller, SolanaIndexerConfigBuilder};
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
        Self {
            config,
            last_signature: None,
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
        let program_id = self.config.program_id;
        let batch_size = self.config.batch_size;
        let last_sig = self.last_signature;
        let rpc_url = self.config.rpc_url.clone();

        let signatures = tokio::task::spawn_blocking(move || {
            use solana_client::rpc_client::GetConfirmedSignaturesForAddress2Config;

            // Create RPC client in the blocking task
            let rpc_client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());

            let config = GetConfirmedSignaturesForAddress2Config {
                before: None,
                until: last_sig,
                limit: Some(batch_size),
                commitment: Some(CommitmentConfig::confirmed()),
            };

            rpc_client
                .get_signatures_for_address_with_config(&program_id, config)
                .map_err(|e| {
                    SolanaIndexerError::RpcError(format!("Failed to fetch signatures: {e}"))
                })
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
    /// # use solana_indexer::{Poller, SolanaIndexerConfigBuilder};
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
        let fetcher = Fetcher::new(self.config.rpc_url.clone());
        let decoder = Decoder::new();

        println!("Starting poller with RPC: {}", self.config.rpc_url);
        println!("Monitoring program: {}", self.config.program_id);

        loop {
            interval.tick().await;

            match self.fetch_new_signatures().await {
                Ok(signatures) => {
                    if !signatures.is_empty() {
                        println!("Fetched {} new signatures", signatures.len());

                        // Process each signature through the pipeline
                        for signature in &signatures {
                            match self
                                .process_transaction(&fetcher, &decoder, signature)
                                .await
                            {
                                Ok(()) => {
                                    println!("✓ Processed transaction: {signature}");
                                }
                                Err(e) => {
                                    eprintln!("✗ Error processing transaction {signature}: {e}");
                                    // Continue processing other transactions
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    // Log error but continue polling
                    eprintln!("Error fetching signatures: {e}");
                    // TODO: Implement retry logic with exponential backoff
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
        println!("  Slot: {}", decoded_tx.slot);
        println!("  Instructions: {}", decoded_tx.instructions.len());
        println!("  Events: {}", decoded_tx.events.len());

        if let Some(compute_units) = decoded_tx.compute_units_consumed {
            println!("  Compute units: {compute_units}");
        }

        // Display instruction details
        for instruction in &decoded_tx.instructions {
            println!(
                "    - Instruction {}: {} ({})",
                instruction.index, instruction.instruction_type, instruction.program_id
            );
        }

        // Display event details
        for event in &decoded_tx.events {
            println!("    - Event: {:?}", event.event_type);
            if let Some(data) = &event.data {
                println!("      Data: {data}");
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
            rpc_url: "http://127.0.0.1:8899".to_string(),
            database_url: "postgresql://localhost/db".to_string(),
            program_id: solana_sdk::pubkey::Pubkey::default(),
            poll_interval_secs: 5,
            batch_size: 100,
        };

        let poller = Poller::new(config);
        assert!(poller.last_signature.is_none());
    }
}
