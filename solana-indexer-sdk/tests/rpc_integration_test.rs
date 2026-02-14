use serde_json::json;
use solana_indexer_sdk::{SolanaIndexer, SolanaIndexerConfigBuilder, Storage};
use std::sync::Arc;
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
async fn test_indexer_real_db_integration() {
    // Load environment variables
    dotenvy::dotenv().ok();

    // Get database URL from environment
    let database_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("DATABASE_URL not set, skipping integration test");
            return;
        }
    };

    // Setup mock RPC server
    // Setup mock RPC server
    let mock_server = MockServer::start().await;
    setup_rpc_mocks(&mock_server).await;

    let test_signature = solana_sdk::signature::Keypair::new().to_base58_string();

    // Mock getSignaturesForAddress
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

    // Mock getTransaction
    let sig_clone = test_signature.clone();
    Mock::given(method("POST"))
        .and(body_string_contains("getTransaction"))
        .respond_with(move |req: &wiremock::Request| {
            let body_str = String::from_utf8_lossy(&req.body);
            if body_str.contains(&sig_clone) {
                ResponseTemplate::new(200).set_body_json(json!({
                    "jsonrpc": "2.0",
                    "result": {
                        "slot": 123456,
                        "blockTime": 1678888888,
                        "transaction": {
                            "signatures": [sig_clone],
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
                }))
            } else {
                ResponseTemplate::new(404)
            }
        })
        .mount(&mock_server)
        .await;

    // Mock getBlock
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

    // Initialize real storage
    let storage = Arc::new(
        Storage::new(&database_url)
            .await
            .expect("Failed to connect to database"),
    );
    storage
        .initialize()
        .await
        .expect("Failed to initialize storage");

    // Clean up any existing test data
    let _ = sqlx::query("DELETE FROM _solana_indexer_sdk_processed WHERE signature = $1")
        .bind(&test_signature)
        .execute(storage.pool())
        .await;

    let _ = sqlx::query("DELETE FROM _solana_indexer_sdk_tentative WHERE signature = $1")
        .bind(&test_signature)
        .execute(storage.pool())
        .await;

    // Create indexer configuration
    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(mock_server.uri())
        .with_database(&database_url)
        .program_id("11111111111111111111111111111111")
        .with_poll_interval(1)
        .with_start_signature("1111111111111111111111111111111111111111111111111111111111111111")
        .build()
        .expect("Failed to build config");

    // Create indexer with real storage
    let indexer = SolanaIndexer::new_with_storage(config, storage.clone());

    // Run indexer for a short time to process the mocked transaction
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), indexer.start()).await;

    // Verify transaction was stored in database
    let is_processed = storage
        .is_processed(&test_signature)
        .await
        .expect("Failed to check if processed");

    assert!(
        is_processed,
        "Transaction should be marked as processed in database"
    );

    // Verify we can query the record directly
    let record = sqlx::query_scalar::<_, String>(
        "SELECT signature FROM _solana_indexer_sdk_tentative WHERE signature = $1",
    )
    .bind(&test_signature)
    .fetch_optional(storage.pool())
    .await
    .expect("Failed to query database");

    assert_eq!(
        record,
        Some(test_signature.to_string()),
        "Transaction signature should exist in database"
    );

    // Clean up test data
    let _ = sqlx::query("DELETE FROM _solana_indexer_sdk_processed WHERE signature = $1")
        .bind(&test_signature)
        .execute(storage.pool())
        .await;
}
