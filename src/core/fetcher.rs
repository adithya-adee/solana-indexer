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
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta, UiConfirmedBlock, UiTransactionEncoding,
};

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
/// let fetcher = Fetcher::new("http://127.0.0.1:8899", solana_sdk::commitment_config::CommitmentConfig::confirmed());
///
/// let signature = Signature::from_str("5j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia7")?;
/// let transaction = fetcher.fetch_transaction(&signature).await?;
/// # Ok(())
/// # }
/// ```
pub struct Fetcher {
    /// RPC endpoint URL
    rpc_url: String,
    /// Commitment configuration for fetching
    commitment: CommitmentConfig,
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
    /// let fetcher = Fetcher::new("http://127.0.0.1:8899", solana_sdk::commitment_config::CommitmentConfig::confirmed());
    /// ```
    #[must_use]
    pub fn new(rpc_url: impl Into<String>, commitment: CommitmentConfig) -> Self {
        Self {
            rpc_url: rpc_url.into(),
            commitment,
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
    /// let fetcher = Fetcher::new("http://127.0.0.1:8899", solana_sdk::commitment_config::CommitmentConfig::confirmed());
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

        let max_retries = 5;
        let mut attempt = 0;

        loop {
            attempt += 1;

            // We use a new client per attempt or reuse one?
            // In spawned blocking task we can't easily reuse across awaits unless we move it in/out.
            // Spawning a new task for each attempt is simpler for error handling but maybe slightly more overhead.
            // Let's keep the spawn_blocking wrapping the RPC call.

            let rpc_url_clone = rpc_url.clone();
            let default_commitment = self.commitment;
            let result = tokio::task::spawn_blocking(move || {
                let rpc_client = RpcClient::new_with_commitment(rpc_url_clone, default_commitment);

                let config = RpcTransactionConfig {
                    encoding: Some(UiTransactionEncoding::JsonParsed),
                    commitment: Some(default_commitment),
                    max_supported_transaction_version: Some(0),
                };

                rpc_client.get_transaction_with_config(&sig, config)
            })
            .await
            .map_err(|e| SolanaIndexerError::InternalError(format!("Task join error: {e}")))?;

            match result {
                Ok(tx) => return Ok(tx),
                Err(e) => {
                    if attempt >= max_retries {
                        return Err(SolanaIndexerError::RpcError(format!(
                            "Failed to fetch transaction {sig} after {max_retries} attempts: {e}"
                        )));
                    }

                    // Simple backoff: 100ms * 2^attempt
                    let backoff = std::time::Duration::from_millis(100 * (1 << attempt));
                    eprintln!(
                        "⚠️ Fetch failed for {sig} (Attempt {attempt}/{max_retries}): {e}. Retrying in {:?}...",
                        backoff
                    );
                    tokio::time::sleep(backoff).await;
                }
            }
        }
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
    /// let fetcher = Fetcher::new("http://127.0.0.1:8899", solana_sdk::commitment_config::CommitmentConfig::confirmed());
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

        let default_commitment = self.commitment;
        tokio::task::spawn_blocking(move || {
            let rpc_client = RpcClient::new_with_commitment(rpc_url, default_commitment);

            // Use rayon for parallel fetching
            let results: Vec<Result<EncodedConfirmedTransactionWithStatusMeta>> = sigs
                .par_iter()
                .map(|sig| {
                    let config = RpcTransactionConfig {
                        encoding: Some(UiTransactionEncoding::JsonParsed), // Use JsonParsed for human-readable instructions
                        commitment: Some(default_commitment),
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

    /// Fetches a single account by its public key.
    ///
    /// # Arguments
    ///
    /// * `pubkey` - The public key of the account to fetch
    ///
    /// # Returns
    ///
    /// The account data if found.
    pub async fn fetch_account(
        &self,
        pubkey: &solana_sdk::pubkey::Pubkey,
    ) -> Result<solana_sdk::account::Account> {
        let rpc_url = self.rpc_url.clone();
        let key = *pubkey;

        let default_commitment = self.commitment;
        tokio::task::spawn_blocking(move || {
            let rpc_client = RpcClient::new_with_commitment(rpc_url, default_commitment);
            rpc_client.get_account(&key).map_err(|e| {
                SolanaIndexerError::RpcError(format!("Failed to fetch account {key}: {e}"))
            })
        })
        .await
        .map_err(|e| SolanaIndexerError::InternalError(format!("Task join error: {e}")))?
    }

    /// Fetches multiple accounts by their public keys.
    ///
    /// # Arguments
    ///
    /// * `pubkeys` - A slice of public keys to fetch
    ///
    /// # Returns
    ///
    /// A vector of optional accounts. `None` indicates the account does not exist.
    pub async fn fetch_multiple_accounts(
        &self,
        pubkeys: &[solana_sdk::pubkey::Pubkey],
    ) -> Result<Vec<Option<solana_sdk::account::Account>>> {
        let rpc_url = self.rpc_url.clone();
        let keys = pubkeys.to_vec();

        let default_commitment = self.commitment;
        tokio::task::spawn_blocking(move || {
            let rpc_client = RpcClient::new_with_commitment(rpc_url, default_commitment);
            rpc_client.get_multiple_accounts(&keys).map_err(|e| {
                SolanaIndexerError::RpcError(format!("Failed to fetch multiple accounts: {e}"))
            })
        })
        .await
        .map_err(|e| SolanaIndexerError::InternalError(format!("Task join error: {e}")))?
    }

    /// Fetches all accounts owned by a program.
    ///
    /// # Arguments
    ///
    /// * `program_id` - The program ID to fetch accounts for
    ///
    /// # Returns
    ///
    /// A vector of (Pubkey, Account) tuples.
    pub async fn get_program_accounts(
        &self,
        program_id: &solana_sdk::pubkey::Pubkey,
    ) -> Result<Vec<(solana_sdk::pubkey::Pubkey, solana_sdk::account::Account)>> {
        let rpc_url = self.rpc_url.clone();
        let pid = *program_id;
        let default_commitment = self.commitment;

        tokio::task::spawn_blocking(move || {
            let rpc_client = RpcClient::new_with_commitment(rpc_url, default_commitment);
            rpc_client.get_program_accounts(&pid).map_err(|e| {
                SolanaIndexerError::RpcError(format!("Failed to fetch program accounts: {e}"))
            })
        })
        .await
        .map_err(|e| SolanaIndexerError::InternalError(format!("Task join error: {e}")))?
    }

    /// Fetches a block with a specific commitment level.
    pub async fn fetch_block_with_commitment(
        &self,
        slot: u64,
        commitment: CommitmentConfig,
    ) -> Result<UiConfirmedBlock> {
        let rpc_url = self.rpc_url.clone();
        tokio::task::spawn_blocking(move || {
            let rpc_client = RpcClient::new_with_commitment(rpc_url, commitment);
            rpc_client
                .get_block_with_config(
                    slot,
                    solana_client::rpc_config::RpcBlockConfig {
                        encoding: Some(UiTransactionEncoding::Base64),
                        transaction_details: Some(
                            solana_transaction_status::TransactionDetails::Full,
                        ),
                        rewards: Some(false),
                        commitment: Some(commitment),
                        max_supported_transaction_version: Some(0),
                    },
                )
                .map_err(|e| SolanaIndexerError::RpcError(e.to_string()))
        })
        .await
        .map_err(|e| SolanaIndexerError::InternalError(format!("Task join error: {e}")))?
    }

    /// Fetches a block by slot.
    pub async fn fetch_block(&self, slot: u64) -> Result<UiConfirmedBlock> {
        let rpc_url = self.rpc_url.clone();

        let max_retries = 5;
        let mut attempt = 0;

        loop {
            attempt += 1;
            let rpc_url_clone = rpc_url.clone();

            let result = tokio::task::spawn_blocking(move || {
                let rpc_client =
                    RpcClient::new_with_commitment(rpc_url_clone, CommitmentConfig::confirmed());
                // Using get_block_with_encoding
                let config = solana_client::rpc_config::RpcBlockConfig {
                    encoding: Some(UiTransactionEncoding::JsonParsed),
                    transaction_details: None,
                    rewards: None,
                    commitment: Some(CommitmentConfig::finalized()),
                    max_supported_transaction_version: Some(0),
                };
                rpc_client.get_block_with_config(slot, config).map_err(|e| {
                    SolanaIndexerError::RpcError(format!("Failed to fetch block {slot}: {e}"))
                })
            })
            .await
            .map_err(|e| SolanaIndexerError::InternalError(format!("Task join error: {e}")))?;

            match result {
                Ok(block) => return Ok(block),
                Err(e) => {
                    // Check if oversight/skip (optional handling)
                    // But for general errors:
                    if attempt >= max_retries {
                        return Err(e);
                    }

                    let backoff = std::time::Duration::from_millis(100 * (1 << attempt));
                    log::warn!(
                        "⚠️ Fetch block failed for {slot} (Attempt {attempt}/{max_retries}): {e}. Retrying...",
                    );
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }

    /// Gets the latest finalized slot.
    pub async fn get_latest_finalized_slot(&self) -> Result<u64> {
        let rpc_url = self.rpc_url.clone();

        tokio::task::spawn_blocking(move || {
            let rpc_client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());
            rpc_client
                .get_slot_with_commitment(CommitmentConfig::finalized())
                .map_err(|e| {
                    SolanaIndexerError::RpcError(format!(
                        "Failed to get latest finalized slot: {e}"
                    ))
                })
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
        let fetcher = Fetcher::new(
            "http://127.0.0.1:8899",
            solana_sdk::commitment_config::CommitmentConfig::confirmed(),
        );
        assert_eq!(fetcher.rpc_url, "http://127.0.0.1:8899");
    }

    #[test]
    fn test_fetcher_creation_from_string() {
        let url = String::from("http://localhost:8899");
        let fetcher = Fetcher::new(
            url,
            solana_sdk::commitment_config::CommitmentConfig::processed(),
        );
        assert_eq!(fetcher.rpc_url, "http://localhost:8899");
    }
}
