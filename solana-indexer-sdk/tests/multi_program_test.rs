use serde_json::json;
use solana_indexer_sdk::{SolanaIndexer, SolanaIndexerConfigBuilder, Storage};
use std::sync::Arc;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

async fn test_multi_program_indexing() {
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

    let program1 = "11111111111111111111111111111111"; // System
    let program2 = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"; // Token

    let sig1 = solana_sdk::signature::Keypair::new().to_base58_string();
    let sig2 = solana_sdk::signature::Keypair::new().to_base58_string();

    // Mock getSignaturesForAddress for both programs
    Mock::given(method("POST"))
        .and(body_string_contains("getSignaturesForAddress"))
        .and(body_string_contains(program1))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": [
                {
                    "signature": sig1,
                    "slot": 100,
                    "err": null,
                    "memo": null,
                    "blockTime": 1000
                }
            ],
            "id": 1
        })))
        .mount(&mock_server)
        .await;

    // Mock for Program 2
    Mock::given(method("POST"))
        .and(body_string_contains("getSignaturesForAddress"))
        .and(body_string_contains(program2))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": [
                {
                    "signature": sig2,
                    "slot": 100,
                    "err": null,
                    "memo": null,
                    "blockTime": 1000
                }
            ],
            "id": 1
        })))
        .mount(&mock_server)
        .await;

    // Mock getTransaction for both
    let s1 = sig1.clone();
    let s2 = sig2.clone();
    Mock::given(method("POST"))
        .and(body_string_contains("getTransaction"))
        .respond_with(move |req: &wiremock::Request| {
            let body_str = String::from_utf8_lossy(&req.body);
            if body_str.contains(&s1) {
                ResponseTemplate::new(200).set_body_json(json!({
                    "jsonrpc": "2.0",
                    "result": {
                        "slot": 100,
                        "transaction": { "signatures": [s1], "message": { "accountKeys": [], "instructions": [], "recentBlockhash": "hash" } },
                        "meta": { "err": null, "status": { "Ok": null }, "fee": 0, "preBalances": [], "postBalances": [], "innerInstructions": [] }
                    },
                    "id": 1
                }))
            } else if body_str.contains(&s2) {
                ResponseTemplate::new(200).set_body_json(json!({
                    "jsonrpc": "2.0",
                    "result": {
                        "slot": 100,
                        "transaction": { "signatures": [s2], "message": { "accountKeys": [], "instructions": [], "recentBlockhash": "hash" } },
                        "meta": { "err": null, "status": { "Ok": null }, "fee": 0, "preBalances": [], "postBalances": [], "innerInstructions": [] }
                    },
                    "id": 1
                }))
            } else {
                ResponseTemplate::new(404)
            }
        })
        .mount(&mock_server)
        .await;

    // Setup Storage
    let storage = Arc::new(Storage::new(&database_url).await.expect("Failed DB"));
    storage.initialize().await.expect("Failed Init");

    // Mock getBlock
    Mock::given(method("POST"))
        .and(body_string_contains("getBlock"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": {
                "blockhash": "11111111111111111111111111111111",
                "previousBlockhash": "11111111111111111111111111111111",
                "parentSlot": 99,
                "transactions": [],
                "rewards": [],
                "blockTime": 1000,
                "blockHeight": 100
            },
            "id": 1
        })))
        .mount(&mock_server)
        .await;

    // Clear DB
    let _ = sqlx::query(
        "DELETE FROM _solana_indexer_sdk_processed WHERE signature = $1 OR signature = $2",
    )
    .bind(&sig1)
    .bind(&sig2)
    .execute(storage.pool())
    .await;

    let _ = sqlx::query(
        "DELETE FROM _solana_indexer_sdk_tentative WHERE signature = $1 OR signature = $2",
    )
    .bind(&sig1)
    .bind(&sig2)
    .execute(storage.pool())
    .await;

    // Config 1
    let config1 = SolanaIndexerConfigBuilder::new()
        .with_rpc(mock_server.uri())
        .with_database(&database_url)
        .program_id(program1)
        .with_poll_interval(1)
        .with_start_signature("1111111111111111111111111111111111111111111111111111111111111111")
        .build()
        .unwrap();

    // Config 2
    let config2 = SolanaIndexerConfigBuilder::new()
        .with_rpc(mock_server.uri())
        .with_database(&database_url)
        .program_id(program2)
        .with_poll_interval(1)
        .with_start_signature("1111111111111111111111111111111111111111111111111111111111111111")
        .build()
        .unwrap();

    let indexer1 = SolanaIndexer::new_with_storage(config1, storage.clone());
    let indexer2 = SolanaIndexer::new_with_storage(config2, storage.clone());

    // Run both indexers concurrently
    let t1 = tokio::spawn(async move {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), indexer1.start()).await;
    });

    let t2 = tokio::spawn(async move {
        // Offset slightly to test concurrent access
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), indexer2.start()).await;
    });

    let _ = tokio::join!(t1, t2);

    // Allow background tasks (spawned by poll_and_process) to complete their DB writes
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Verify both are processed
    let p1 = storage.is_processed(&sig1).await.unwrap();
    let p2 = storage.is_processed(&sig2).await.unwrap();

    assert!(p1, "Sig1 should be processed");
    assert!(p2, "Sig2 should be processed");

    // Clean up
    let _ = sqlx::query("DELETE FROM _solana_indexer_sdk_processed WHERE signature IN ($1, $2)")
        .bind(sig1)
        .bind(sig2)
        .execute(storage.pool())
        .await;
}
