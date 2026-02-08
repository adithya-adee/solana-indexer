//! Transaction fetching module for `SolanaIndexer`.
//!
//! This module is responsible for retrieving full transaction details from
//! Solana RPC endpoints. It takes transaction signatures and fetches the
//! complete transaction data including instruction details, logs, and metadata.

use crate::utils::error::{Result, SolanaIndexerError};
use solana_client::rpc_client::RpcClient;
use solana_client::rpc_config::RpcTransactionConfig;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::signature::Signature;
use solana_transaction_status::{EncodedConfirmedTransactionWithStatusMeta, UiTransactionEncoding};

/// Transaction fetcher for retrieving full transaction details.
///
/// The `Fetcher` handles communication with Solana RPC endpoints to retrieve
/// complete transaction data. It supports both single and batch fetching operations.
///
/// # Example
///
/// ```no_run
/// # use solana_indexer::Fetcher;
/// # use solana_sdk::signature::Signature;
/// # use std::str::FromStr;
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let fetcher = Fetcher::new("http://127.0.0.1:8899");
///
/// let signature = Signature::from_str("5j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia7")?;
/// let transaction = fetcher.fetch_transaction(&signature).await?;
/// # Ok(())
/// # }
/// ```
pub struct Fetcher {
    /// RPC endpoint URL
    rpc_url: String,
}

impl Fetcher {
    /// Creates a new `Fetcher` instance.
    ///
    /// # Arguments
    ///
    /// * `rpc_url` - The Solana RPC endpoint URL
    ///
    /// # Example
    ///
    /// ```
    /// # use solana_indexer::Fetcher;
    /// let fetcher = Fetcher::new("http://127.0.0.1:8899");
    /// ```
    #[must_use]
    pub fn new(rpc_url: impl Into<String>) -> Self {
        Self {
            rpc_url: rpc_url.into(),
        }
    }

    /// Fetches a single transaction by its signature.
    ///
    /// This method retrieves the full transaction details including:
    /// - Transaction metadata
    /// - Instruction data
    /// - Account keys
    /// - Log messages
    /// - Compute units consumed
    ///
    /// # Arguments
    ///
    /// * `signature` - The transaction signature to fetch
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::RpcError` if:
    /// - The RPC request fails
    /// - The transaction is not found
    /// - The network is unreachable
    ///
    /// # Returns
    ///
    /// The full transaction data with status metadata.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::Fetcher;
    /// # use solana_sdk::signature::Signature;
    /// # use std::str::FromStr;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let fetcher = Fetcher::new("http://127.0.0.1:8899");
    /// let sig = Signature::from_str("5j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia7")?;
    ///
    /// match fetcher.fetch_transaction(&sig).await {
    ///     Ok(tx) => println!("Transaction fetched successfully"),
    ///     Err(e) => eprintln!("Failed to fetch transaction: {}", e),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn fetch_transaction(
        &self,
        signature: &Signature,
    ) -> Result<EncodedConfirmedTransactionWithStatusMeta> {
        let rpc_url = self.rpc_url.clone();
        let sig = *signature;

        tokio::task::spawn_blocking(move || {
            let rpc_client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());

            let config = RpcTransactionConfig {
                encoding: Some(UiTransactionEncoding::JsonParsed),
                commitment: Some(CommitmentConfig::confirmed()),
                max_supported_transaction_version: Some(0),
            };

            rpc_client
                .get_transaction_with_config(&sig, config)
                .map_err(|e| {
                    SolanaIndexerError::RpcError(format!("Failed to fetch transaction {sig}: {e}"))
                })
        })
        .await
        .map_err(|e| SolanaIndexerError::InternalError(format!("Task join error: {e}")))?
    }

    /// Fetches multiple transactions in batch.
    ///
    /// This method fetches multiple transactions concurrently for improved performance.
    /// Failed fetches are logged but don't stop the batch operation.
    ///
    /// # Arguments
    ///
    /// * `signatures` - A slice of transaction signatures to fetch
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::RpcError` if the RPC client cannot be created.
    /// Individual transaction fetch failures are returned in the result vector.
    ///
    /// # Returns
    ///
    /// A vector of results, one for each signature. Successfully fetched transactions
    /// are `Ok(transaction)`, while failures are `Err(error)`.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::Fetcher;
    /// # use solana_sdk::signature::Signature;
    /// # use std::str::FromStr;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let fetcher = Fetcher::new("http://127.0.0.1:8899");
    /// let signatures = vec![
    ///     Signature::from_str("5j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia7")?,
    /// ];
    ///
    /// let results = fetcher.fetch_transactions(&signatures).await?;
    /// for (i, result) in results.iter().enumerate() {
    ///     match result {
    ///         Ok(tx) => println!("Transaction {} fetched successfully", i),
    ///         Err(e) => eprintln!("Transaction {} failed: {}", i, e),
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn fetch_transactions(
        &self,
        signatures: &[Signature],
    ) -> Result<Vec<Result<EncodedConfirmedTransactionWithStatusMeta>>> {
        use rayon::prelude::*;

        let rpc_url = self.rpc_url.clone();
        let sigs = signatures.to_vec();

        tokio::task::spawn_blocking(move || {
            let rpc_client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());

            // Use rayon for parallel fetching
            let results: Vec<Result<EncodedConfirmedTransactionWithStatusMeta>> = sigs
                .par_iter()
                .map(|sig| {
                    let config = RpcTransactionConfig {
                        encoding: Some(UiTransactionEncoding::JsonParsed), // Use JsonParsed for human-readable instructions
                        commitment: Some(CommitmentConfig::confirmed()),
                        max_supported_transaction_version: Some(0),
                    };
                    rpc_client
                        .get_transaction_with_config(sig, config)
                        .map_err(|e| {
                            SolanaIndexerError::RpcError(format!(
                                "Failed to fetch transaction {sig}: {e}"
                            ))
                        })
                })
                .collect();

            Ok(results)
        })
        .await
        .map_err(|e| SolanaIndexerError::InternalError(format!("Task join error: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fetcher_creation() {
        let fetcher = Fetcher::new("http://127.0.0.1:8899");
        assert_eq!(fetcher.rpc_url, "http://127.0.0.1:8899");
    }

    #[test]
    fn test_fetcher_creation_from_string() {
        let url = String::from("http://localhost:8899");
        let fetcher = Fetcher::new(url);
        assert_eq!(fetcher.rpc_url, "http://localhost:8899");
    }
}
