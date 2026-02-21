//! Helius WebSocket stream handler.

use crate::config::SolanaIndexerConfig;
use crate::utils::error::{Result, SolanaIndexerError};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::TransactionVersion;
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta, EncodedTransactionWithStatusMeta,
};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

use super::TransactionSource;

/// Helius WebSocket source for acquiring transaction data.
pub struct HeliusSource {
    receiver: mpsc::Receiver<crate::streams::TransactionEvent>,
}

impl HeliusSource {
    /// Creates a new `HeliusSource` instance.
    pub async fn new(config: SolanaIndexerConfig) -> Result<Self> {
        let ws_url = config
            .helius_ws_url()
            .ok_or_else(|| {
                SolanaIndexerError::ConfigError("Helius WS URL not configured".to_string())
            })?
            .to_string();

        if !matches!(config.source, crate::config::SourceConfig::Helius { .. }) {
            return Err(SolanaIndexerError::ConfigError(
                "Not a Helius config".to_string(),
            ));
        }

        let program_ids = config.program_ids.iter().map(|p| p.to_string()).collect();
        let (sender, receiver) = mpsc::channel(1000); // Buffer size

        // Spawn background task to handle WS connection
        tokio::spawn(Self::run_stream(ws_url, program_ids, sender));

        Ok(Self { receiver })
    }

    async fn run_stream(
        ws_url: String,
        program_ids: Vec<String>,
        sender: mpsc::Sender<crate::streams::TransactionEvent>,
    ) {
        loop {
            tracing::info!("Connecting to Helius WS: {}", ws_url);
            match connect_async(&ws_url).await {
                Ok((ws_stream, _)) => {
                    tracing::info!("Connected to Helius WS");
                    let (mut write, mut read) = ws_stream.split();

                    // Subscribe to transaction events
                    // Reference: https://docs.helius.dev/solana-apis/enhanced-websocket-api/transaction-subscribe
                    let subscribe_msg = json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "transactionSubscribe",
                        "params": [
                            {
                                "accountInclude": program_ids
                            },
                            {
                                "commitment": "confirmed",
                                "encoding": "jsonParsed",
                                "transactionDetails": "full",
                                "showRewards": false,
                                "maxSupportedTransactionVersion": 0
                            }
                        ]
                    });

                    if let Err(e) = write.send(Message::Text(subscribe_msg.to_string())).await {
                        tracing::error!("Failed to send subscribe message: {}", e);
                        sleep(Duration::from_secs(5)).await;
                        continue;
                    }

                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                // Attempt to parse as notification
                                match serde_json::from_str::<HeliusTransactionNotification>(&text) {
                                    Ok(notification) => {
                                        if let Some(params) = notification.params {
                                            let result = params.result;
                                            if let Ok(signature) =
                                                Signature::from_str(&result.signature)
                                            {
                                                // Construct the full transaction object
                                                let tx_with_meta =
                                                    EncodedConfirmedTransactionWithStatusMeta {
                                                        slot: result.slot,
                                                        transaction:
                                                            EncodedTransactionWithStatusMeta {
                                                                transaction: result.transaction,
                                                                meta: Some(result.meta),
                                                                version: result.version,
                                                            },
                                                        block_time: result.block_time,
                                                    };

                                                let event = crate::streams::TransactionEvent::FullTransaction {
                                                    signature,
                                                    slot: result.slot,
                                                    tx: Arc::new(tx_with_meta),
                                                };

                                                if sender.send(event).await.is_err() {
                                                    return; // Receiver dropped
                                                }
                                            }
                                        }
                                    }
                                    Err(_) => {
                                        // Ignore ping/pong or other messages for now
                                        // Could be helpful to log debug if needed
                                    }
                                }
                            }
                            Ok(Message::Close(_)) => break,
                            Err(e) => {
                                tracing::error!("WS error: {}", e);
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to connect to Helius WS: {}", e);
                }
            }

            // Reconnect strategy
            tracing::info!("Reconnecting to Helius WS in 5 seconds...");
            sleep(Duration::from_secs(5)).await;
        }
    }
}

#[async_trait]
impl TransactionSource for HeliusSource {
    async fn next_batch(&mut self) -> Result<Vec<crate::streams::TransactionEvent>> {
        // Collect available events, wait if empty
        let mut events = Vec::new();

        // Block for at least one
        if let Some(event) = self.receiver.recv().await {
            events.push(event);
        } else {
            // Channel closed
            return Ok(vec![]);
        }

        // Drain others if available (up to 100 to match batch size)
        while let Ok(event) = self.receiver.try_recv() {
            events.push(event);
            if events.len() >= 100 {
                break;
            }
        }

        Ok(events)
    }

    fn source_name(&self) -> &'static str {
        "Helius WebSocket"
    }
}

#[derive(Deserialize)]
struct HeliusTransactionNotification {
    params: Option<HeliusParams>,
}

#[derive(Deserialize)]
struct HeliusParams {
    result: HeliusResult,
}

#[derive(Deserialize)]
struct HeliusResult {
    signature: String,
    slot: u64,
    #[serde(default)]
    block_time: Option<i64>,
    transaction: solana_transaction_status::EncodedTransaction,
    meta: solana_transaction_status::UiTransactionStatusMeta,
    #[serde(default)]
    version: Option<TransactionVersion>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_helius_notification() {
        let json_data = r#"
        {
            "params": {
                "result": {
                    "signature": "5h6x",
                    "slot": 12345,
                    "transaction": {
                        "signatures": ["5h6x"],
                        "message": {
                            "accountKeys": [],
                            "header": {
                                "numReadonlySignedAccounts": 0,
                                "numReadonlyUnsignedAccounts": 0,
                                "numRequiredSignatures": 1
                            },
                            "instructions": [],
                            "recentBlockhash": "11111111111111111111111111111111"
                        }
                    },
                    "meta": {
                        "err": null,
                        "fee": 5000,
                        "preBalances": [],
                        "postBalances": [],
                        "innerInstructions": [],
                        "logMessages": [],
                        "preTokenBalances": [],
                        "postTokenBalances": [],
                        "rewards": [],
                        "status": {"Ok": null}
                    },
                    "blockTime": 1678900000,
                    "version": 0
                }
            }
        }
        "#;

        let notification: HeliusTransactionNotification =
            serde_json::from_str(json_data).expect("Failed to parse");
        let result = notification.params.unwrap().result;
        assert_eq!(result.slot, 12345);
        assert_eq!(result.signature, "5h6x");
        assert_eq!(result.version, Some(TransactionVersion::Number(0)));
    }
}
