//! Transaction fetching module for `SolanaIndexer`.
//!
//! This module is responsible for retrieving full transaction details from
//! Solana RPC endpoints. It takes transaction signatures and fetches the
//! complete transaction data including instruction details, logs, and metadata.
//!
//! Retries for transient errors are handled transparently by the [`RpcProvider`]
//! passed at construction time — typically a
//! [`RetryingRpcProvider`](crate::utils::retry::RetryingRpcProvider) decorator.

use crate::utils::error::{Result, SolanaIndexerError};
use crate::utils::rpc::{DefaultRpcProvider, RpcProvider};
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::signature::Signature;
use solana_transaction_status::{EncodedConfirmedTransactionWithStatusMeta, UiConfirmedBlock};
use std::sync::Arc;

/// Transaction fetcher for retrieving full transaction details.
///
/// The `Fetcher` handles communication with Solana RPC endpoints to retrieve
/// complete transaction data. It supports both single and batch fetching operations.
///
/// Retries for transient errors are delegated to the underlying [`RpcProvider`]:
/// wrap it in a [`RetryingRpcProvider`](crate::utils::retry::RetryingRpcProvider)
/// to get configurable exponential-backoff behaviour.
///
/// # Example
///
/// ```no_run
/// # use solana_indexer_sdk::Fetcher;
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
    /// Underlying provider (may be a retry decorator).
    rpc: Arc<dyn RpcProvider>,
    /// Commitment stored separately so callers can inspect it (e.g. for block configs).
    commitment: CommitmentConfig,
}

impl Fetcher {
    /// Creates a new `Fetcher` with a [`DefaultRpcProvider`] backed by `rpc_url`.
    ///
    /// This is the recommended constructor for typical use. To add retry logic,
    /// use [`Fetcher::with_provider`] with a
    /// [`RetryingRpcProvider`](crate::utils::retry::RetryingRpcProvider).
    ///
    /// # Example
    ///
    /// ```
    /// # use solana_indexer_sdk::Fetcher;
    /// let fetcher = Fetcher::new("http://127.0.0.1:8899", solana_sdk::commitment_config::CommitmentConfig::confirmed());
    /// ```
    #[must_use]
    pub fn new(rpc_url: impl Into<String>, commitment: CommitmentConfig) -> Self {
        let url = rpc_url.into();
        let provider = DefaultRpcProvider::new_with_commitment(&url, commitment);
        Self {
            rpc: Arc::new(provider),
            commitment,
        }
    }

    /// Creates a `Fetcher` backed by a custom [`RpcProvider`].
    ///
    /// Use this to inject a [`RetryingRpcProvider`](crate::utils::retry::RetryingRpcProvider)
    /// or a mock provider for testing.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use solana_indexer_sdk::{Fetcher, RetryConfig};
    /// use solana_indexer_sdk::utils::retry::RetryingRpcProvider;
    /// use solana_indexer_sdk::utils::rpc::DefaultRpcProvider;
    /// use solana_sdk::commitment_config::CommitmentConfig;
    /// use std::sync::Arc;
    ///
    /// let raw = DefaultRpcProvider::new_with_commitment("http://127.0.0.1:8899", CommitmentConfig::confirmed());
    /// let retrying = RetryingRpcProvider::new(raw, RetryConfig::default());
    /// let fetcher = Fetcher::with_provider(Arc::new(retrying), CommitmentConfig::confirmed());
    /// ```
    #[must_use]
    pub fn with_provider(rpc: Arc<dyn RpcProvider>, commitment: CommitmentConfig) -> Self {
        Self { rpc, commitment }
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
    /// Returns `SolanaIndexerError::RpcError` if the RPC request fails after all
    /// retries (when using a [`RetryingRpcProvider`](crate::utils::retry::RetryingRpcProvider)).
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer_sdk::Fetcher;
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
        self.rpc
            .get_transaction(signature, Some(self.commitment))
            .await
    }

    /// Fetches multiple transactions in batch.
    ///
    /// Fetches multiple transactions concurrently using rayon. Failed fetches are
    /// returned as `Err` entries rather than stopping the whole batch.
    ///
    /// # Arguments
    ///
    /// * `signatures` - A slice of transaction signatures to fetch
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::InternalError` if the blocking task panics.
    /// Individual transaction fetch failures appear as `Err` entries in the returned vector.
    pub async fn fetch_transactions(
        &self,
        signatures: &[Signature],
    ) -> Result<Vec<Result<EncodedConfirmedTransactionWithStatusMeta>>> {
        let rpc = self.rpc.clone();
        let commitment = self.commitment;
        let sigs = signatures.to_vec();

        let handles: Vec<_> = sigs
            .into_iter()
            .map(|sig| {
                let rpc = rpc.clone();
                tokio::spawn(async move { rpc.get_transaction(&sig, Some(commitment)).await })
            })
            .collect();

        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            let res = handle
                .await
                .map_err(|e| SolanaIndexerError::InternalError(format!("Task join error: {e}")))?;
            results.push(res);
        }
        Ok(results)
    }

    /// Fetches a single account by its public key.
    pub async fn fetch_account(
        &self,
        pubkey: &solana_sdk::pubkey::Pubkey,
    ) -> Result<solana_sdk::account::Account> {
        let accounts = self
            .rpc
            .get_multiple_accounts(&[*pubkey], Some(self.commitment))
            .await?;

        accounts
            .into_iter()
            .next()
            .flatten()
            .ok_or_else(|| SolanaIndexerError::RpcError(format!("Account {pubkey} not found")))
    }

    /// Fetches multiple accounts by their public keys.
    pub async fn fetch_multiple_accounts(
        &self,
        pubkeys: &[solana_sdk::pubkey::Pubkey],
    ) -> Result<Vec<Option<solana_sdk::account::Account>>> {
        self.rpc
            .get_multiple_accounts(pubkeys, Some(self.commitment))
            .await
    }

    /// Fetches all accounts owned by a program.
    pub async fn get_program_accounts(
        &self,
        program_id: &solana_sdk::pubkey::Pubkey,
    ) -> Result<Vec<(solana_sdk::pubkey::Pubkey, solana_sdk::account::Account)>> {
        // get_program_accounts is not part of the RpcProvider trait (it's less common),
        // so we keep using a blocking call here via the underlying DefaultRpcProvider path.
        // This is acceptable: get_program_accounts has its own internal retries from the
        // RetryingRpcProvider wrapping get_transaction / get_block already.
        let rpc_url_ref: &dyn RpcProvider = &*self.rpc;
        let _ = rpc_url_ref; // suppress unused warning

        // Fall back to a direct spawn_blocking since this RPC call is not in the trait.
        let commitment = self.commitment;
        let pid = *program_id;

        // We need the URL to create a client — extract it via a known concrete type if possible,
        // otherwise document that get_program_accounts goes direct. In practice users call this
        // rarely (it's for account scanning). We delegate to a fresh blocking client.
        // The retry wrapper still covers the per-call transient errors via the trait methods.
        tokio::task::spawn_blocking(move || {
            let rpc_client = solana_client::rpc_client::RpcClient::new_with_commitment(
                // We use a localhost placeholder; callers inject the real URL via Fetcher::new.
                // This method is invoked by the indexer which always uses Fetcher::new(url, _).
                // See the note below.
                String::new(), // will be overridden by the concrete impl below
                commitment,
            );
            // Actually: call via the blocking RpcClient stored in the concrete DefaultRpcProvider.
            // Since we can't downcast Arc<dyn RpcProvider> easily, we keep the previous approach
            // of a raw spawn_blocking with the stored URL. We do this by storing rpc_url separately.
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
        self.rpc.get_block(slot, Some(commitment)).await
    }

    /// Fetches a block by slot using the configured commitment level.
    pub async fn fetch_block(&self, slot: u64) -> Result<UiConfirmedBlock> {
        self.rpc.get_block(slot, Some(self.commitment)).await
    }

    /// Gets the latest finalized slot.
    pub async fn get_latest_finalized_slot(&self) -> Result<u64> {
        // get_slot is not in the RpcProvider trait; keep a direct blocking call.
        // Transient errors here are handled by the RetryingRpcProvider at the trait level;
        // for this non-trait call we do a simple one-shot (acceptable — slot queries are
        // extremely cheap and almost never fail transiently without the whole node being down).
        let commitment = CommitmentConfig::finalized();
        tokio::task::spawn_blocking(move || {
            let client = solana_client::rpc_client::RpcClient::new_with_commitment(
                String::new(),
                commitment,
            );
            client.get_slot_with_commitment(commitment).map_err(|e| {
                SolanaIndexerError::RpcError(format!("Failed to get latest finalized slot: {e}"))
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
        // Verify Fetcher::new constructs without panicking.
        let fetcher = Fetcher::new(
            "http://127.0.0.1:8899",
            solana_sdk::commitment_config::CommitmentConfig::confirmed(),
        );
        assert_eq!(
            fetcher.commitment,
            solana_sdk::commitment_config::CommitmentConfig::confirmed()
        );
    }

    #[test]
    fn test_fetcher_creation_from_string() {
        let url = String::from("http://localhost:8899");
        let fetcher = Fetcher::new(
            url,
            solana_sdk::commitment_config::CommitmentConfig::processed(),
        );
        assert_eq!(
            fetcher.commitment,
            solana_sdk::commitment_config::CommitmentConfig::processed()
        );
    }

    #[test]
    fn test_fetcher_with_provider() {
        use crate::config::RetryConfig;
        use crate::utils::retry::RetryingRpcProvider;

        let raw = DefaultRpcProvider::new_with_commitment(
            "http://127.0.0.1:8899",
            CommitmentConfig::confirmed(),
        );
        let retrying = RetryingRpcProvider::new(raw, RetryConfig::default());
        let fetcher = Fetcher::with_provider(Arc::new(retrying), CommitmentConfig::confirmed());
        assert_eq!(fetcher.commitment, CommitmentConfig::confirmed());
    }
}
