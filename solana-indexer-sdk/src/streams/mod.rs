//! Input sources for acquiring transaction data from Solana.
//!
//! This module contains different strategies for acquiring transaction data,
//! including polling (RPC) and WebSocket subscriptions.
//!
//! The `TransactionSource` trait provides a unified interface for both approaches.

use crate::utils::error::Result;
use async_trait::async_trait;
use solana_sdk::signature::Signature;

/// Represents a transaction event from a source.
#[derive(Debug, Clone)]
pub enum TransactionEvent {
    /// A simple signature (e.g., from RPC polling)
    Signature { signature: Signature, slot: u64 },
    /// A signature with associated logs (e.g., from WebSocket logsSubscribe)
    LogEvent {
        signature: Signature,
        logs: Vec<String>,
        err: Option<serde_json::Value>,
        slot: u64,
    },
    /// A full transaction with metadata (e.g., from Helius transactionSubscribe)
    FullTransaction {
        signature: Signature,
        slot: u64,
        tx: std::sync::Arc<solana_transaction_status::EncodedConfirmedTransactionWithStatusMeta>,
    },
}

impl TransactionEvent {
    /// Returns the signature associated with this event.
    pub fn signature(&self) -> Signature {
        match self {
            TransactionEvent::Signature { signature, .. } => *signature,
            TransactionEvent::LogEvent { signature, .. } => *signature,
            TransactionEvent::FullTransaction { signature, .. } => *signature,
        }
    }

    /// Returns the slot associated with this event.
    pub fn slot(&self) -> u64 {
        match self {
            TransactionEvent::Signature { slot, .. } => *slot,
            TransactionEvent::LogEvent { slot, .. } => *slot,
            TransactionEvent::FullTransaction { slot, .. } => *slot,
        }
    }
}

/// Unified interface for transaction sources (Polling or WebSocket)
///
/// This trait allows `SolanaIndexer` to work with any transaction source,
/// whether it's polling an RPC endpoint or subscribing via WebSocket.
#[async_trait]
pub trait TransactionSource: Send + Sync {
    /// Get the next batch of transaction events
    ///
    /// This method blocks until new events are available.
    /// For polling sources, this waits for the poll interval.
    /// For WebSocket sources, this waits for incoming notifications.
    ///
    /// # Returns
    ///
    /// A vector of transaction events to process
    async fn next_batch(&mut self) -> Result<Vec<TransactionEvent>>;

    /// Get a human-readable name for this source (for logging)
    fn source_name(&self) -> &str;
}

pub mod helius;
pub mod hybrid;
pub mod laserstream;
pub mod poller;
pub mod websocket;
