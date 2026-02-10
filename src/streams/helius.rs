//! Helius WebSocket stream handler.

use crate::config::SolanaIndexerConfig;
use crate::utils::error::{Result, SolanaIndexerError};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use solana_sdk::signature::Signature;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

use super::TransactionSource;

/// Helius WebSocket source for acquiring transaction signatures.
pub struct HeliusSource {
    receiver: mpsc::Receiver<Signature>,
    source_name: String,
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

        // API key is already embedded in the WS URL by config.helius_ws_url()
        // so we don't need to extract it separately here unless needed for other API calls.
        if !matches!(config.source, crate::config::SourceConfig::Helius { .. }) {
            return Err(SolanaIndexerError::ConfigError(
                "Not a Helius config".to_string(),
            ));
        }

        let program_id = config.program_id.to_string();
        let (sender, receiver) = mpsc::channel(1000); // Buffer size

        // Spawn background task to handle WS connection
        tokio::spawn(async move {
            loop {
                println!("Connecting to Helius WS: {}", ws_url);
                match connect_async(&ws_url).await {
                    Ok((ws_stream, _)) => {
                        println!("Connected to Helius WS");
                        let (mut write, mut read) = ws_stream.split();

                        // Subscribe to logs for the program
                        // Reference: https://docs.helius.dev/solana-rpc-nodes/websocket-methods
                        let subscribe_msg = json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "method": "logsSubscribe",
                            "params": [
                                {
                                    "mentions": [program_id]
                                },
                                {
                                    "commitment": "confirmed"
                                }
                            ]
                        });

                        if let Err(e) = write.send(Message::Text(subscribe_msg.to_string())).await {
                            eprintln!("Failed to send subscribe message: {}", e);
                            sleep(Duration::from_secs(5)).await;
                            continue;
                        }

                        while let Some(msg) = read.next().await {
                            match msg {
                                Ok(Message::Text(text)) => {
                                    // Parse keys from log notification
                                    if let Ok(parsed) =
                                        serde_json::from_str::<HeliusLogNotification>(&text)
                                        && let Some(params) = parsed.params
                                    {
                                        let signature_str = params.result.value.signature;
                                        if let Ok(sig) = Signature::from_str(&signature_str)
                                            && sender.send(sig).await.is_err()
                                        {
                                            break; // Receiver dropped, stop everything
                                        }
                                    }
                                }
                                Ok(Message::Close(_)) => break,
                                Err(e) => {
                                    eprintln!("WS error: {}", e);
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to connect to Helius WS: {}", e);
                    }
                }

                // Reconnect strategy
                println!("Reconnecting to Helius WS in 5 seconds...");
                sleep(Duration::from_secs(5)).await;
            }
        });

        Ok(Self {
            receiver,
            source_name: "Helius WebSocket".to_string(),
        })
    }
}

#[async_trait]
impl TransactionSource for HeliusSource {
    async fn next_batch(&mut self) -> Result<Vec<Signature>> {
        // Collect available signatures, wait if empty
        let mut signatures = Vec::new();

        // Block for at least one
        if let Some(sig) = self.receiver.recv().await {
            signatures.push(sig);
        } else {
            // Channel closed
            return Ok(vec![]);
        }

        // Drain others if available (up to 100 to match batch size)
        while let Ok(sig) = self.receiver.try_recv() {
            signatures.push(sig);
            if signatures.len() >= 100 {
                break;
            }
        }

        Ok(signatures)
    }

    fn source_name(&self) -> &str {
        &self.source_name
    }
}

#[derive(Deserialize)]
struct HeliusLogNotification {
    params: Option<HeliusLogParams>,
}

#[derive(Deserialize)]
struct HeliusLogParams {
    result: HeliusLogResult,
}

#[derive(Deserialize)]
struct HeliusLogResult {
    value: HeliusLogValue,
}

#[derive(Deserialize)]
struct HeliusLogValue {
    signature: String,
}
