//! Hybrid transaction source combining WebSocket and RPC polling.
//!
//! This module implements a strategy that uses WebSocket for low-latency real-time events
//! and background RPC polling to detect and fill gaps (e.g., due to dropped UDP packets or connection issues).

use super::{TransactionEvent, TransactionSource};
use crate::utils::error::{Result, SolanaIndexerError};
use crate::utils::logging;
use async_trait::async_trait;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{pubkey::Pubkey, signature::Signature};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{interval, Duration};

/// Hybrid input source combining WebSocket and RPC.
pub struct HybridSource {
    receiver: Mutex<mpsc::Receiver<Result<Vec<TransactionEvent>>>>,
}

impl HybridSource {
    /// Creates a new `HybridSource` instance.
    pub fn new(
        ws_url: impl Into<String>,
        rpc_url: impl Into<String>,
        program_ids: Vec<Pubkey>,
        poll_interval_secs: u64,
        reconnect_delay_secs: u64,
        _gap_threshold_slots: u64,
    ) -> Self {
        let ws_url = ws_url.into();
        let rpc_url = rpc_url.into();
        let (tx, receiver) = mpsc::channel(100);

        // Shared state for highest seen slot
        let metrics = Arc::new(HybridMetrics::default());

        // Spawn WebSocket task
        let tx_ws = tx.clone();
        let metrics_ws = metrics.clone();
        let ws_url_clone = ws_url.clone();
        let program_ids_ws = program_ids.clone();
        tokio::spawn(async move {
            let mut ws_source = super::websocket::WebSocketSource::new(
                ws_url_clone,
                program_ids_ws,
                reconnect_delay_secs,
            );

            loop {
                match ws_source.next_batch().await {
                    Ok(events) => {
                        // Update highest slot seen
                        for event in &events {
                            let slot = event.slot();
                            metrics_ws.update_slot(slot);
                        }

                        if tx_ws.send(Ok(events)).await.is_err() {
                            break; // Receiver dropped
                        }
                    }
                    Err(e) => {
                        if tx_ws.send(Err(e)).await.is_err() {
                            break;
                        }
                        tokio::time::sleep(Duration::from_secs(reconnect_delay_secs)).await;
                    }
                }
            }
        });

        // Spawn Poller task for gap detection
        let tx_rpc = tx.clone();
        let program_ids_rpc = program_ids;
        tokio::spawn(async move {
            let rpc_client = RpcClient::new(rpc_url);
            let mut interval = interval(Duration::from_secs(poll_interval_secs));

            // We start polling from "now" roughly.
            // Or we could try to determine the latest slot.
            // For simplicity, we just poll the latest confirmed signatures.
            let mut last_polled_signature: Option<Signature> = None;

            loop {
                interval.tick().await;
                for program_id in &program_ids_rpc {
                    // 1. Fetch latest signatures
                    let mut config =
                        solana_client::rpc_client::GetConfirmedSignaturesForAddress2Config {
                            limit: Some(50),
                            commitment: Some(
                                solana_sdk::commitment_config::CommitmentConfig::confirmed(),
                            ),
                            ..Default::default()
                        };

                    if let Some(until) = last_polled_signature {
                        config.until = Some(until);
                    }

                    match rpc_client
                        .get_signatures_for_address_with_config(program_id, config)
                        .await
                    {
                        Ok(signatures) => {
                            if signatures.is_empty() {
                                continue;
                            }

                            // Update last polled signature to the most recent one (first in the list)
                            // Update last polled signature to the most recent one (first in the list)
                            #[allow(clippy::collapsible_if)]
                            if let Some(first) = signatures.first() {
                                if let Ok(sig) = Signature::from_str(&first.signature) {
                                    last_polled_signature = Some(sig);
                                }
                            }

                            let max_ws_slot = metrics.get_max_slot();
                            let mut gap_events = Vec::new();

                            for sig_info in signatures {
                                // Only emit events that might have been missed
                                // If the slot is > gap_threshold from max_ws_slot?
                                // Logic:
                                // We are polling independently.
                                // If WS is healthy, max_ws_slot should be close to sig_info.slot.
                                // If sig_info.slot > max_ws_slot + gap_threshold, it means WS is lagging significantly.
                                // But here we just want to ensure we catch ALL signatures.
                                // The duplication is handled by Storage (idempotency).
                                // So we can send everything found via RPC polling that hasn't been seen by WS recently?
                                // Actually, sending duplicates is fine as long as DB handles it.
                                // The main purpose is to fill gaps.

                                // Let's just send them as Signature events.
                                if let Ok(sig) = Signature::from_str(&sig_info.signature) {
                                    gap_events.push(TransactionEvent::Signature {
                                        signature: sig,
                                        slot: sig_info.slot,
                                    });
                                }
                            }

                            if !gap_events.is_empty() {
                                logging::log(
                                    logging::LogLevel::Info,
                                    &format!(
                                        "Hybrid Monitor: Found {} signatures via RPC (Max WS Slot: {})",
                                        gap_events.len(),
                                        max_ws_slot
                                    ),
                                );
                                if tx_rpc.send(Ok(gap_events)).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Err(e) => {
                            logging::log_error("Hybrid Poller Error", &e.to_string());
                        }
                    }
                }
            }
        });

        Self {
            receiver: Mutex::new(receiver),
        }
    }
}

#[async_trait]
impl TransactionSource for HybridSource {
    async fn next_batch(&mut self) -> Result<Vec<TransactionEvent>> {
        let mut receiver = self.receiver.lock().await;
        receiver.recv().await.ok_or_else(|| {
            SolanaIndexerError::InternalError("Hybrid source channel closed".to_string())
        })?
    }

    fn source_name(&self) -> &'static str {
        "Hybrid (WS + RPC)"
    }
}

#[derive(Default)]
struct HybridMetrics {
    max_slot: AtomicU64,
}

impl HybridMetrics {
    fn update_slot(&self, slot: u64) {
        self.max_slot.fetch_max(slot, Ordering::Relaxed);
    }

    fn get_max_slot(&self) -> u64 {
        self.max_slot.load(Ordering::Relaxed)
    }
}
