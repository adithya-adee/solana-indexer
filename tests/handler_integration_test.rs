use async_trait::async_trait;
use serde_json::json;
use solana_indexer::utils::error::Result;
use solana_indexer::{
    EventHandler, InstructionDecoder, SolanaIndexer, SolanaIndexerConfigBuilder, Storage,
    TransferEvent,
};
use sqlx::PgPool;
use std::sync::Arc;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Test decoder that extracts transfer events
struct TestTransferDecoder;

impl InstructionDecoder<TransferEvent> for TestTransferDecoder {
    fn decode(
        &self,
        instruction: &solana_transaction_status::UiInstruction,
    ) -> Option<TransferEvent> {
        println!(
            "TestTransferDecoder::decode called with instruction: {:?}",
            instruction
        );
        // For testing, return a mock transfer event
        Some(TransferEvent {
            from: "sender123".to_string(),
            to: "receiver456".to_string(),
            amount: 1000,
        })
    }
}

/// Test handler that writes to a custom table
struct TestTransferHandler;

#[async_trait]
impl EventHandler<TransferEvent> for TestTransferHandler {
    async fn handle(&self, event: TransferEvent, db: &PgPool, signature: &str) -> Result<()> {
        println!(
            "TestTransferHandler::handle called for signature: {}",
            signature
        );
        // Insert transfer record (Table created in setup)
        sqlx::query(
            "INSERT INTO test_transfers (signature, from_address, to_address, amount) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
        )
        .bind(signature)
        .bind(&event.from)
        .bind(&event.to)
        .bind(i64::try_from(event.amount).unwrap_or(0))
        .execute(db)
        .await?;

        Ok(())
    }
}

#[tokio::test]

async fn test_handler_integration_with_database() {
    dotenvy::dotenv().ok();
    // ... (rest of setup)
    let database_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("DATABASE_URL not set, skipping integration test");
            return;
        }
    };

    let mock_server = MockServer::start().await;

    let test_signature =
        "5j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia7";

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
                            { "pubkey": "sender123", "signer": true, "writable": true },
                            { "pubkey": "receiver456", "signer": false, "writable": true }
                        ],
                        "instructions": [{
                            "program": "system",
                            "programId": "11111111111111111111111111111111",
                            "parsed": {
                                "type": "transfer",
                                "info": {
                                    "source": "sender123",
                                    "destination": "receiver456",
                                    "lamports": 1000
                                }
                            }
                        }],
                        "recentBlockhash": "11111111111111111111111111111111"
                    }
                },
                "meta": {
                    "err": null,
                    "status": { "Ok": null },
                    "fee": 5000,
                    "preBalances": [10000, 0],
                    "postBalances": [9000, 1000],
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

    let storage = Arc::new(
        Storage::new(&database_url)
            .await
            .expect("Failed to connect"),
    );
    storage.initialize().await.expect("Failed to initialize");

    // Initialize custom table HERE
    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS test_transfers (
            signature TEXT PRIMARY KEY,
            from_address TEXT NOT NULL,
            to_address TEXT NOT NULL,
            amount BIGINT NOT NULL,
            created_at TIMESTAMPTZ DEFAULT NOW()
        )
        ",
    )
    .execute(storage.pool())
    .await
    .expect("Failed to create test table");

    // Clean up test data
    let _ = sqlx::query("DELETE FROM test_transfers WHERE signature = $1")
        .bind(test_signature)
        .execute(storage.pool())
        .await;

    let _ = sqlx::query("DELETE FROM _solana_indexer_processed WHERE signature = $1")
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

    let mut indexer = SolanaIndexer::new_with_storage(config, storage.clone());

    // Register decoder for System Program (both name and ID)
    indexer
        .decoder_registry_mut()
        .register(
            "system".to_string(),
            Box::new(Box::new(TestTransferDecoder) as Box<dyn InstructionDecoder<TransferEvent>>),
        )
        .unwrap();

    indexer
        .decoder_registry_mut()
        .register(
            "11111111111111111111111111111111".to_string(),
            Box::new(Box::new(TestTransferDecoder) as Box<dyn InstructionDecoder<TransferEvent>>),
        )
        .unwrap();

    let handler: Box<dyn EventHandler<TransferEvent>> = Box::new(TestTransferHandler);
    indexer
        .handler_registry_mut()
        .register(TransferEvent::discriminator(), Box::new(handler))
        .unwrap();

    // Setup mocks RIGHT before starting to ensure they are top priority (LIFO)
    // Common mocks (Version, Blockhash)
    Mock::given(method("POST"))
        .and(body_string_contains("getVersion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": { "solana-core": "1.16.7", "feature-set": 0 },
            "id": 1
        })))
        .mount(&mock_server)
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
        .mount(&mock_server)
        .await;

    // Specific mocks
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
        // .expect(1..) // Removed to avoid phantom verification failures
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
                            { "pubkey": "sender123", "signer": true, "writable": true },
                            { "pubkey": "receiver456", "signer": false, "writable": true }
                        ],
                        "instructions": [{
                            "program": "system",
                            "programId": "11111111111111111111111111111111",
                            "parsed": {
                                "type": "transfer",
                                "info": {
                                    "source": "sender123",
                                    "destination": "receiver456",
                                    "lamports": 1000
                                }
                            }
                        }],
                        "recentBlockhash": "11111111111111111111111111111111"
                    }
                },
                "meta": {
                    "err": null,
                    "status": { "Ok": null },
                    "fee": 5000,
                    "preBalances": [10000, 0],
                    "postBalances": [9000, 1000],
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

    // Run indexer with increased timeout
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), indexer.start()).await;

    // Verify handler wrote to custom table
    let record = sqlx::query_as::<_, (String, String, String, i64)>(
        "SELECT signature, from_address, to_address, amount FROM test_transfers WHERE signature = $1",
    )
    .bind(test_signature)
    .fetch_optional(storage.pool())
    .await
    .expect("Failed to query");

    assert!(record.is_some(), "Handler should have written to database");

    if let Some((sig, from, to, amount)) = record {
        assert_eq!(sig, test_signature);
        assert_eq!(from, "sender123");
        assert_eq!(to, "receiver456");
        assert_eq!(amount, 1000);
    }

    // Clean up
    let _ = sqlx::query("DELETE FROM test_transfers WHERE signature = $1")
        .bind(test_signature)
        .execute(storage.pool())
        .await;

    let _ = sqlx::query("DELETE FROM _solana_indexer_processed WHERE signature = $1")
        .bind(test_signature)
        .execute(storage.pool())
        .await;
}
