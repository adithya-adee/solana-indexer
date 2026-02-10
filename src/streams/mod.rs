//! Input sources for acquiring transaction data from Solana.
//!
//! This module contains different strategies for acquiring transaction data,
//! including polling (RPC) and WebSocket subscriptions.
//!
//! The `TransactionSource` trait provides a unified interface for both approaches.

use crate::utils::error::Result;
use async_trait::async_trait;
use solana_sdk::signature::Signature;

/// Unified interface for transaction sources (Polling or WebSocket)
///
/// This trait allows `SolanaIndexer` to work with any transaction source,
/// whether it's polling an RPC endpoint or subscribing via WebSocket.
#[async_trait]
pub trait TransactionSource: Send + Sync {
    /// Get the next batch of transaction signatures
    ///
    /// This method blocks until new signatures are available.
    /// For polling sources, this waits for the poll interval.
    /// For WebSocket sources, this waits for incoming notifications.
    ///
    /// # Returns
    ///
    /// A vector of transaction signatures to process
    async fn next_batch(&mut self) -> Result<Vec<Signature>>;

    /// Get a human-readable name for this source (for logging)
    fn source_name(&self) -> &str;
}

pub mod helius;
pub mod poller;
pub mod websocket;
