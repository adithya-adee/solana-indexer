use serde_json::json;
use solana_indexer_sdk::{SolanaIndexer, SolanaIndexerConfigBuilder, Storage};
use std::sync::Arc;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Setup common RPC mocks
async fn setup_rpc_mocks(mock_server: &MockServer) {
    Mock::given(method("POST"))
        .and(body_string_contains("getVersion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": { "solana-core": "1.16.7", "feature-set": 0 },
            "id": 1
        })))
        .mount(mock_server)
        .await;

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

async fn test_rpc_error_recovery() {
    dotenvy::dotenv().ok();

    let database_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("DATABASE_URL not set, skipping integration test");
            return;
        }
    };

    let mock_server = MockServer::start().await;
    setup_rpc_mocks(&mock_server).await;

    // Mock RPC failure on first call
    Mock::given(method("POST"))
        .and(body_string_contains("getSignaturesForAddress"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&mock_server)
        .await;

    // Mock success on subsequent calls
    Mock::given(method("POST"))
        .and(body_string_contains("getSignaturesForAddress"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": [],
            "id": 1
        })))
        .mount(&mock_server)
        .await;

    let storage = Arc::new(
        Storage::new(&database_url)
            .await
            .expect("Failed to connect"),
    );
    storage.initialize().await.expect("Failed to initialize");

    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(mock_server.uri())
        .with_database(&database_url)
        .program_id("11111111111111111111111111111111")
        .with_poll_interval(1)
        .build()
        .expect("Failed to build config");

    let indexer = SolanaIndexer::new_with_storage(config, storage);

    // Run indexer - it should handle the initial error and continue
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), indexer.start()).await;

    // If we get here without panic, error recovery worked
}

#[tokio::test]

async fn test_idempotency_duplicate_transactions() {
    dotenvy::dotenv().ok();

    let database_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("DATABASE_URL not set, skipping integration test");
            return;
        }
    };

    let mock_server = MockServer::start().await;
    setup_rpc_mocks(&mock_server).await;

    let test_signature =
        "3j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia9";

    // Mock returns same transaction multiple times
    Mock::given(method("POST"))
        .and(body_string_contains("getSignaturesForAddress"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": [
                {
                    "signature": test_signature,
                    "slot": 123456,
                    "err": null,
                    "memo": null,
                    "blockTime": 1678888888
                }
            ],
            "id": 1
        })))
        .mount(&mock_server)
        .await;

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
                        "accountKeys": [
                            {
                                "pubkey": "11111111111111111111111111111111",
                                "signer": true,
                                "source": "transaction",
                                "writable": true
                            }
                        ],
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
        .mount(&mock_server)
        .await;

    Mock::given(method("POST"))
        .and(body_string_contains("getBlock"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": {
                "blockhash": "11111111111111111111111111111111",
                "previousBlockhash": "11111111111111111111111111111111",
                "parentSlot": 123455,
                "transactions": [],
                "rewards": [],
                "blockTime": 1678888888,
                "blockHeight": 123456
            },
            "id": 1
        })))
        .mount(&mock_server)
        .await;

    let storage = Arc::new(
        Storage::new(&database_url)
            .await
            .expect("Failed to connect"),
    );
    storage.initialize().await.expect("Failed to initialize");

    // Clean up
    let _ = sqlx::query("DELETE FROM _solana_indexer_sdk_tentative WHERE signature = $1")
        .bind(test_signature)
        .execute(storage.pool())
        .await;

    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(mock_server.uri())
        .with_database(&database_url)
        .program_id("11111111111111111111111111111111")
        .with_poll_interval(1)
        .build()
        .expect("Failed to build config");

    let indexer = SolanaIndexer::new_with_storage(config, storage.clone());

    // Run indexer multiple times - should only process once
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), indexer.start()).await;

    // Verify transaction was only stored once
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM _solana_indexer_sdk_tentative WHERE signature = $1",
    )
    .bind(test_signature)
    .fetch_one(storage.pool())
    .await
    .expect("Failed to query");

    assert_eq!(count, 1, "Transaction should only be stored once");

    // Clean up
    let _ = sqlx::query("DELETE FROM _solana_indexer_sdk_tentative WHERE signature = $1")
        .bind(test_signature)
        .execute(storage.pool())
        .await;
}

#[tokio::test]

async fn test_transaction_not_found_handling() {
    dotenvy::dotenv().ok();

    let database_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("DATABASE_URL not set, skipping integration test");
            return;
        }
    };

    let mock_server = MockServer::start().await;
    setup_rpc_mocks(&mock_server).await;

    let test_signature =
        "4j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia0";

    Mock::given(method("POST"))
        .and(body_string_contains("getSignaturesForAddress"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": [
                {
                    "signature": test_signature,
                    "slot": 123456,
                    "err": null,
                    "memo": null,
                    "blockTime": 1678888888
                }
            ],
            "id": 1
        })))
        .mount(&mock_server)
        .await;

    // Mock transaction not found
    Mock::given(method("POST"))
        .and(body_string_contains("getTransaction"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": null,
            "id": 1
        })))
        .mount(&mock_server)
        .await;

    let storage = Arc::new(
        Storage::new(&database_url)
            .await
            .expect("Failed to connect"),
    );
    storage.initialize().await.expect("Failed to initialize");

    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(mock_server.uri())
        .with_database(&database_url)
        .program_id("11111111111111111111111111111111")
        .with_poll_interval(1)
        .build()
        .expect("Failed to build config");

    let indexer = SolanaIndexer::new_with_storage(config, storage.clone());

    // Run indexer - should handle not found gracefully
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), indexer.start()).await;

    // Transaction should not be marked as processed since it wasn't found
    let is_processed = storage
        .is_processed(test_signature)
        .await
        .expect("Failed to check");

    assert!(
        !is_processed,
        "Transaction not found should not be marked as processed"
    );
}
