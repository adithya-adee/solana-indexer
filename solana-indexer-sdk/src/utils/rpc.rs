use async_trait::async_trait;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_response::RpcConfirmedTransactionStatusWithSignature;
use solana_sdk::{
    account::Account, commitment_config::CommitmentConfig, pubkey::Pubkey, signature::Signature,
};
use solana_transaction_status::EncodedConfirmedTransactionWithStatusMeta;

use crate::utils::error::Result;

#[async_trait]
pub trait RpcProvider: Send + Sync {
    async fn get_signatures_for_address(
        &self,
        address: &Pubkey,
        before: Option<Signature>,
        until: Option<Signature>,
        limit: usize,
        commitment: Option<CommitmentConfig>,
    ) -> Result<Vec<RpcConfirmedTransactionStatusWithSignature>>;

    async fn get_transaction(
        &self,
        signature: &Signature,
        commitment: Option<CommitmentConfig>,
    ) -> Result<EncodedConfirmedTransactionWithStatusMeta>;

    async fn get_multiple_accounts(
        &self,
        pubkeys: &[Pubkey],
        commitment: Option<CommitmentConfig>,
    ) -> Result<Vec<Option<Account>>>;

    async fn get_block(
        &self,
        slot: u64,
        commitment: Option<CommitmentConfig>,
    ) -> Result<solana_transaction_status::UiConfirmedBlock>;
}

pub struct DefaultRpcProvider {
    client: RpcClient,
}

impl DefaultRpcProvider {
    /// Creates a provider connecting to `rpc_url` with the default commitment level.
    pub fn new(rpc_url: &str) -> Self {
        Self {
            client: RpcClient::new(rpc_url.to_string()),
        }
    }

    /// Creates a provider with a specific commitment configuration.
    pub fn new_with_commitment(rpc_url: &str, commitment: CommitmentConfig) -> Self {
        Self {
            client: RpcClient::new_with_commitment(rpc_url.to_string(), commitment),
        }
    }
}

#[async_trait]
impl RpcProvider for DefaultRpcProvider {
    async fn get_signatures_for_address(
        &self,
        address: &Pubkey,
        before: Option<Signature>,
        until: Option<Signature>,
        limit: usize,
        commitment: Option<CommitmentConfig>,
    ) -> Result<Vec<RpcConfirmedTransactionStatusWithSignature>> {
        let config = solana_client::rpc_client::GetConfirmedSignaturesForAddress2Config {
            before,
            until,
            limit: Some(limit),
            commitment,
        };
        Ok(self
            .client
            .get_signatures_for_address_with_config(address, config)
            .await
            .map_err(|e| crate::utils::error::SolanaIndexerError::RpcClientError(Box::new(e)))?)
    }

    async fn get_transaction(
        &self,
        signature: &Signature,
        commitment: Option<CommitmentConfig>,
    ) -> Result<EncodedConfirmedTransactionWithStatusMeta> {
        Ok(self
            .client
            .get_transaction_with_config(
                signature,
                solana_client::rpc_config::RpcTransactionConfig {
                    encoding: Some(solana_transaction_status::UiTransactionEncoding::Json),
                    commitment,
                    max_supported_transaction_version: Some(0),
                },
            )
            .await
            .map_err(|e| crate::utils::error::SolanaIndexerError::RpcClientError(Box::new(e)))?)
    }

    async fn get_multiple_accounts(
        &self,
        pubkeys: &[Pubkey],
        commitment: Option<CommitmentConfig>,
    ) -> Result<Vec<Option<Account>>> {
        Ok(self
            .client
            .get_multiple_accounts_with_commitment(pubkeys, commitment.unwrap_or_default())
            .await
            .map_err(|e| crate::utils::error::SolanaIndexerError::RpcClientError(Box::new(e)))?
            .value)
    }

    async fn get_block(
        &self,
        slot: u64,
        commitment: Option<CommitmentConfig>,
    ) -> Result<solana_transaction_status::UiConfirmedBlock> {
        Ok(self
            .client
            .get_block_with_config(
                slot,
                solana_client::rpc_config::RpcBlockConfig {
                    encoding: Some(solana_transaction_status::UiTransactionEncoding::Json),
                    transaction_details: Some(solana_transaction_status::TransactionDetails::Full),
                    rewards: Some(false),
                    commitment,
                    max_supported_transaction_version: Some(0),
                },
            )
            .await
            .map_err(|e| crate::utils::error::SolanaIndexerError::RpcClientError(Box::new(e)))?)
    }
}
