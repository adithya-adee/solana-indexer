#![cfg(feature = "websockets")]

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use solana_indexer_sdk::{SolanaIndexer, SolanaIndexerConfigBuilder, Storage};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::protocol::Message;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Setup common RPC mocks
async fn setup_rpc_mocks(mock_server: &MockServer) {
    // Mock getVersion
    Mock::given(method("POST"))
        .and(body_string_contains("getVersion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": { "solana-core": "1.16.7", "feature-set": 0 },
            "id": 1
        })))
        .mount(mock_server)
        .await;

    // Mock getLatestBlockhash
    Mock::given(method("POST"))
        .and(body_string_contains("getLatestBlockhash"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": {
                "context": { "slot": 1 },
                "value": {
                    "blockhash": "11111111111111111111111111111111",
                    "lastValidBlockHeight": 100
                }
            },
            "id": 1
        })))
        .mount(mock_server)
        .await;
}

#[tokio::test]
async fn test_indexer_websocket_integration() {
    // Load environment variables
    dotenvy::dotenv().ok();

    let database_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("DATABASE_URL not set, skipping integration test");
            return;
        }
    };

    // Setup mock RPC server for getTransaction (called by indexer after WS notification)
    let mock_rpc = MockServer::start().await;
    setup_rpc_mocks(&mock_rpc).await;

    let test_signature =
        "5j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia8"; // Changed last char from 7 to 8

    // Mock getTransaction
    Mock::given(method("POST"))
        .and(body_string_contains("getTransaction"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": {
                "slot": 123456,
                "blockTime": 1678888888,
                "transaction": {
                    "signatures": [test_signature],
                    "message": {
                        "accountKeys": [],
                        "instructions": [],
                        "recentBlockhash": "11111111111111111111111111111111"
                    }
                },
                "meta": {
                    "err": null,
                    "status": { "Ok": null },
                    "fee": 5000,
                    "preBalances": [],
                    "postBalances": [],
                    "innerInstructions": [],
                    "logMessages": [],
                    "preTokenBalances": [],
                    "postTokenBalances": [],
                    "rewards": []
                }
            },
            "id": 1
        })))
        .mount(&mock_rpc)
        .await;

    // Setup mock WebSocket server
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_addr = listener.local_addr().unwrap();
    let ws_url = format!("ws://{}", ws_addr);

    let server_handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws_stream = accept_async(stream).await.unwrap();

        // Wait for subscription request
        if let Some(Ok(Message::Text(_))) = ws_stream.next().await {
            // Send subscription confirmation
            ws_stream
                .send(Message::Text(
                    json!({
                        "jsonrpc": "2.0",
                        "result": 12345,
                        "id": 1
                    })
                    .to_string(),
                ))
                .await
                .unwrap();

            // Send a notification
            ws_stream
                .send(Message::Text(
                    json!({
                        "jsonrpc": "2.0",
                        "method": "logsNotification",
                        "params": {
                            "result": {
                                "context": { "slot": 123456 },
                                "value": {
                                    "signature": test_signature,
                                    "err": null,
                                    "logs": ["Program 11111111111111111111111111111111 success"]
                                }
                            },
                            "subscription": 12345
                        }
                    })
                    .to_string(),
                ))
                .await
                .unwrap();

            // Keep the connection open for a while to allow the client to process
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        }
    });

    // Initialize storage
    let storage = Arc::new(
        Storage::new(&database_url)
            .await
            .expect("Failed to connect to database"),
    );
    storage
        .initialize()
        .await
        .expect("Failed to initialize storage");

    // Clean up
    let _ = sqlx::query("DELETE FROM _solana_indexer_sdk_processed WHERE signature = $1")
        .bind(test_signature)
        .execute(storage.pool())
        .await;

    // Create configuration
    let config = SolanaIndexerConfigBuilder::new()
        .with_ws(ws_url, mock_rpc.uri())
        .with_database(&database_url)
        .program_id("11111111111111111111111111111111")
        .build()
        .expect("Failed to build config");

    let indexer = SolanaIndexer::new_with_storage(config, storage.clone());
    let token = indexer.cancellation_token();

    // Start indexer in background
    let indexer_handle = tokio::spawn(async move { indexer.start().await });

    // Wait for the indexer to process the notification
    // We poll the database instead of a blunt sleep for faster and more reliable tests
    let mut processed = false;
    for _ in 0..10 {
        if storage.is_processed(test_signature).await.unwrap() {
            processed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // Graceful shutdown
    token.cancel();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), indexer_handle).await;

    // Verify processed
    assert!(processed, "WS notification should have been processed");

    // Clean up
    let _ = sqlx::query("DELETE FROM _solana_indexer_sdk_processed WHERE signature = $1")
        .bind(test_signature)
        .execute(storage.pool())
        .await;

    server_handle.abort();
}
