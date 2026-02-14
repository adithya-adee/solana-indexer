//! WebSocket-based transaction source for real-time indexing
//!
//! This module provides a WebSocket client that subscribes to Solana program
//! notifications and yields transaction signatures in real-time.

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use solana_sdk::{pubkey::Pubkey, signature::Signature};
use std::str::FromStr;
use tokio::time::{sleep, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use super::TransactionSource;
use crate::utils::error::{Result, SolanaIndexerError};

/// WebSocket-based input source for acquiring transaction signatures.
///
/// The `WebSocketSource` subscribes to a Solana RPC WebSocket endpoint for
/// real-time notifications of new transactions related to a specific program ID.
/// This is ideal for production environments requiring low latency and high throughput.
///
/// # Example
///
/// ```no_run
/// use solana_indexer_sdk::streams::websocket::WebSocketSource;
/// use solana_indexer_sdk::SolanaIndexerConfigBuilder;
/// use solana_sdk::pubkey::Pubkey;
/// use std::str::FromStr;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = SolanaIndexerConfigBuilder::new()
///     .with_ws("ws://127.0.0.1:8900", "http://127.0.0.1:8899")
///     .with_database("postgresql://localhost/db")
///     .program_id("11111111111111111111111111111111")
///     .build()?;
/// let program_id = Pubkey::from_str("11111111111111111111111111111111").unwrap();
/// let source = WebSocketSource::new("ws://127.0.0.1:8900", vec![program_id], 5);
/// # Ok(())
/// # }
/// ```
pub struct WebSocketSource {
    /// WebSocket URL (ws:// or wss://)
    ws_url: String,
    /// Program ID to subscribe to
    program_ids: Vec<Pubkey>,
    /// Reconnection delay in seconds
    reconnect_delay_secs: u64,
    /// Internal state
    state: WebSocketState,
}

/// Internal WebSocket state
enum WebSocketState {
    Disconnected,
    Connected {
        #[allow(dead_code)] // Kept for future unsubscribe functionality
        subscription_id: u64,
        receiver: tokio::sync::mpsc::UnboundedReceiver<crate::streams::TransactionEvent>,
    },
}

/// WebSocket notification from Solana (logsSubscribe)
#[derive(Debug, Deserialize)]
struct LogsNotification {
    pub(super) params: LogsNotificationParams,
}

#[derive(Debug, Deserialize)]
struct LogsNotificationParams {
    pub(super) result: LogsNotificationResult,
}

#[derive(Debug, Deserialize)]
struct LogsNotificationResult {
    pub(super) value: LogsNotificationValue,
    pub(super) context: LogsNotificationContext,
}

#[derive(Debug, Deserialize)]
struct LogsNotificationContext {
    pub(super) slot: u64,
}

#[derive(Debug, Deserialize)]
struct LogsNotificationValue {
    pub(super) signature: String,
    pub(super) logs: Vec<String>,
    pub(super) err: Option<serde_json::Value>,
}

/// Subscription response from Solana
#[derive(Debug, Deserialize)]
struct SubscriptionResponse {
    result: u64,
}

impl WebSocketSource {
    /// Creates a new `WebSocketSource` instance.
    ///
    /// # Arguments
    ///
    /// * `ws_url` - The WebSocket URL (e.g., `<ws://127.0.0.1:8900>`)
    /// * `program_id` - The program ID to monitor for notifications
    /// * `reconnect_delay_secs` - Seconds to wait before reconnecting after a disconnect
    ///
    /// # Example
    ///
    /// ```
    /// # use solana_indexer_sdk::streams::websocket::WebSocketSource;
    /// # use solana_sdk::pubkey::Pubkey;
    /// let source = WebSocketSource::new(
    ///     "ws://api.mainnet-beta.solana.com",
    ///     vec![Pubkey::default()],
    ///     5
    /// );
    /// ```
    pub fn new(
        ws_url: impl Into<String>,
        program_ids: Vec<Pubkey>,
        reconnect_delay_secs: u64,
    ) -> Self {
        Self {
            ws_url: ws_url.into(),
            program_ids,
            reconnect_delay_secs,
            state: WebSocketState::Disconnected,
        }
    }

    /// Connects to WebSocket and subscribes to program notifications
    async fn connect(&mut self) -> Result<()> {
        use crate::utils::logging;

        logging::log(
            logging::LogLevel::Info,
            &format!("Connecting to WebSocket: {}", self.ws_url),
        );

        // Connect to WebSocket
        let (ws_stream, _) = connect_async(&self.ws_url).await.map_err(|e| {
            SolanaIndexerError::RpcError(format!("WebSocket connection failed: {e}"))
        })?;

        let (mut write, mut read) = ws_stream.split();

        // Subscribe to transaction logs mentioning this program
        // This captures all transactions that involve the program
        let subscribe_request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "logsSubscribe",
            "params": [
                {
                    "mentions": self.program_ids.iter().map(|p| p.to_string()).collect::<Vec<String>>()
                },
                {
                    "commitment": "confirmed"
                }
            ]
        });

        write
            .send(Message::Text(subscribe_request.to_string()))
            .await
            .map_err(|e| {
                SolanaIndexerError::RpcError(format!("Failed to send subscription: {e}"))
            })?;

        // Wait for subscription confirmation
        let subscription_id = loop {
            #[allow(clippy::collapsible_if)]
            if let Some(Ok(Message::Text(text))) = read.next().await {
                if let Ok(response) = serde_json::from_str::<SubscriptionResponse>(&text) {
                    break response.result;
                }
            }
        };

        logging::log(
            logging::LogLevel::Success,
            &format!("WebSocket subscribed (ID: {subscription_id})"),
        );

        // Create channel for events
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn background task to handle incoming messages
        tokio::spawn(async move {
            while let Some(Ok(Message::Text(text))) = read.next().await {
                #[allow(clippy::collapsible_if)]
                if let Ok(notification) = serde_json::from_str::<LogsNotification>(&text) {
                    if let Ok(sig) =
                        Signature::from_str(&notification.params.result.value.signature)
                    {
                        let event = crate::streams::TransactionEvent::LogEvent {
                            signature: sig,
                            logs: notification.params.result.value.logs,
                            err: notification.params.result.value.err,
                            slot: notification.params.result.context.slot,
                        };
                        let _ = tx.send(event);
                    }
                }
            }
        });

        self.state = WebSocketState::Connected {
            subscription_id,
            receiver: rx,
        };

        Ok(())
    }

    /// Ensures connection is established, reconnecting if necessary
    async fn ensure_connected(&mut self) -> Result<()> {
        match &self.state {
            WebSocketState::Disconnected => {
                self.connect().await?;
            }
            WebSocketState::Connected { receiver, .. } => {
                // Check if receiver is still alive
                if receiver.is_closed() {
                    use crate::utils::logging;
                    logging::log(
                        logging::LogLevel::Warning,
                        "WebSocket disconnected, reconnecting...",
                    );
                    sleep(Duration::from_secs(self.reconnect_delay_secs)).await;
                    self.state = WebSocketState::Disconnected;
                    self.connect().await?;
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl TransactionSource for WebSocketSource {
    async fn next_batch(&mut self) -> Result<Vec<crate::streams::TransactionEvent>> {
        self.ensure_connected().await?;

        match &mut self.state {
            WebSocketState::Connected { receiver, .. } => {
                let mut events = Vec::new();

                // Wait for at least one event
                if let Some(event) = receiver.recv().await {
                    events.push(event);

                    // Collect any additional events that are immediately available
                    while let Ok(event) = receiver.try_recv() {
                        events.push(event);
                        if events.len() >= 10 {
                            // Batch size limit
                            break;
                        }
                    }
                }

                Ok(events)
            }
            WebSocketState::Disconnected => Err(SolanaIndexerError::InternalError(
                "WebSocket not connected".to_string(),
            )),
        }
    }

    fn source_name(&self) -> &'static str {
        "WebSocket"
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_websocket_source_creation() {
        let ws_url = "ws://127.0.0.1:8900";
        let program_id = Pubkey::new_unique();
        let reconnect_delay = 5;

        let source = WebSocketSource::new(ws_url, vec![program_id], reconnect_delay);

        assert_eq!(source.ws_url, ws_url);
        assert_eq!(source.program_ids, vec![program_id]);
        assert_eq!(source.reconnect_delay_secs, reconnect_delay);

        match source.state {
            WebSocketState::Disconnected => {}
            WebSocketState::Connected { .. } => panic!("Expected initially disconnected state"),
        }
    }

    #[test]
    fn test_logs_notification_deserialization() {
        let json_data = r#"{
            "jsonrpc": "2.0",
            "method": "logsNotification",
            "params": {
                "result": {
                    "context": {
                        "slot": 5208469
                    },
                    "value": {
                        "signature": "5h6xBEauJ3PK6rJ9pG4Q8Xc6rJ9pG4Q8Xc6rJ9pG4Q8Xc6rJ9pG4Q8Xc6rJ9pG4Q8Xc6rJ9pG4Q8Xc",
                        "err": null,
                        "logs": [
                            "Program 11111111111111111111111111111111 invoke [1]",
                            "Program 11111111111111111111111111111111 success"
                        ]
                    }
                },
                "subscription": 12345
            }
        }"#;

        let notification: LogsNotification = serde_json::from_str(json_data).unwrap();
        assert_eq!(
            notification.params.result.value.signature,
            "5h6xBEauJ3PK6rJ9pG4Q8Xc6rJ9pG4Q8Xc6rJ9pG4Q8Xc6rJ9pG4Q8Xc6rJ9pG4Q8Xc6rJ9pG4Q8Xc"
        );
        assert_eq!(notification.params.result.value.logs.len(), 2);
    }
}
