use async_trait::async_trait;
use serde_json::json;
use solana_indexer::utils::error::Result;
use solana_indexer::{SolanaIndexer, SolanaIndexerConfigBuilder, StorageBackend};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::sync::Arc;
use std::sync::Mutex;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Mock Storage implementation
struct MockStorage {
    pool: PgPool,
    processed_signatures: Arc<Mutex<Vec<String>>>,
}

impl MockStorage {
    fn new() -> Self {
        // Connect lazy avoids actual network connection until used
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://mock:5432/mock")
            .unwrap();
        Self {
            pool,
            processed_signatures: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl StorageBackend for MockStorage {
    async fn initialize(&self) -> Result<()> {
        Ok(())
    }

    async fn is_processed(&self, signature: &str) -> Result<bool> {
        let sigs = self.processed_signatures.lock().unwrap();
        Ok(sigs.contains(&signature.to_string()))
    }

    async fn mark_processed(&self, signature: &str, _slot: u64) -> Result<()> {
        let mut sigs = self.processed_signatures.lock().unwrap();
        sigs.push(signature.to_string());
        Ok(())
    }

    async fn get_last_processed_slot(&self) -> Result<Option<u64>> {
        Ok(None)
    }

    fn pool(&self) -> &PgPool {
        &self.pool
    }
}

// Setup common mocks for RPC
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

    // Mock getLatestBlockhash (often called by sdk)
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
async fn test_indexer_orchestration_no_transactions() {
    let mock_server = MockServer::start().await;
    setup_rpc_mocks(&mock_server).await;

    // Mock getSignaturesForAddress to return empty list
    Mock::given(method("POST"))
        .and(body_string_contains("getSignaturesForAddress"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": [],
            "id": 1
        })))
        .mount(&mock_server)
        .await;

    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(mock_server.uri())
        .with_database("postgresql://mock/db")
        .program_id("11111111111111111111111111111111")
        .with_poll_interval(1)
        .build()
        .unwrap();

    let storage = Arc::new(MockStorage::new());
    let indexer = SolanaIndexer::new_with_storage(config, storage.clone());

    // We can't easily run indexer.start() because it loops forever.
    // Instead we test internal methods if they were public, or we spawn it and kill it?
    // start() is the only public entry point that runs the loop.
    // However, we can use `timeout` to run it for a bit.

    let result = tokio::time::timeout(std::time::Duration::from_millis(100), indexer.start()).await;

    // Timeout is expected
    assert!(result.is_err());
}

#[tokio::test]
async fn test_indexer_process_transaction_flow() {
    let mock_server = MockServer::start().await;
    setup_rpc_mocks(&mock_server).await;

    let sig_str =
        "5j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia7";

    // Mock getSignaturesForAll
    Mock::given(method("POST"))
        .and(body_string_contains("getSignaturesForAddress"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": [
                {
                    "signature": sig_str,
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
    Mock::given(method("POST"))
        .and(body_string_contains("getTransaction"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": {
                "slot": 123456,
                "blockTime": 1678888888,
                "transaction": {
                    "signatures": [sig_str],
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
        .mount(&mock_server)
        .await;

    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(mock_server.uri())
        .with_database("postgresql://mock/db")
        .program_id("11111111111111111111111111111111")
        .with_poll_interval(1)
        .build()
        .unwrap();

    let storage = Arc::new(MockStorage::new());
    let indexer = SolanaIndexer::new_with_storage(config, storage.clone());

    // Run for a short time to allow processing
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), indexer.start()).await;

    // Verify transaction was marked as processed
    let is_processed = storage.is_processed(sig_str).await.unwrap();
    assert!(is_processed);
}
