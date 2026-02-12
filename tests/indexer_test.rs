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
    finalized_slots: Arc<Mutex<Vec<u64>>>,
    tentative_signatures: Arc<Mutex<Vec<String>>>,
    backfill_progress: Arc<Mutex<Option<u64>>>,
    block_hashes: Arc<Mutex<std::collections::HashMap<u64, String>>>,
    tentative_transactions: Arc<Mutex<std::collections::HashMap<u64, Vec<String>>>>,
}

impl MockStorage {
    fn new() -> Self {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://mock:5432/mock")
            .unwrap();
        Self {
            pool,
            processed_signatures: Arc::new(Mutex::new(Vec::new())),
            finalized_slots: Arc::new(Mutex::new(Vec::new())),
            tentative_signatures: Arc::new(Mutex::new(Vec::new())),
            backfill_progress: Arc::new(Mutex::new(None)),
            block_hashes: Arc::new(Mutex::new(std::collections::HashMap::new())),
            tentative_transactions: Arc::new(Mutex::new(std::collections::HashMap::new())),
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
        if sigs.contains(&signature.to_string()) {
            return Ok(true);
        }
        let tent_sigs = self.tentative_signatures.lock().unwrap();
        Ok(tent_sigs.contains(&signature.to_string()))
    }

    async fn mark_processed(&self, signature: &str, _slot: u64) -> Result<()> {
        let mut sigs = self.processed_signatures.lock().unwrap();
        sigs.push(signature.to_string());
        Ok(())
    }

    async fn get_last_processed_slot(&self) -> Result<Option<u64>> {
        Ok(None)
    }

    async fn get_last_processed_signature(&self) -> Result<Option<String>> {
        let sigs = self.processed_signatures.lock().unwrap();
        Ok(sigs.last().cloned())
    }

    fn pool(&self) -> &PgPool {
        &self.pool
    }

    async fn mark_tentative(&self, signature: &str, slot: u64, block_hash: &str) -> Result<()> {
        self.tentative_signatures
            .lock()
            .unwrap()
            .push(signature.to_string());
        self.tentative_transactions
            .lock()
            .unwrap()
            .entry(slot)
            .or_default()
            .push(signature.to_string());
        self.block_hashes
            .lock()
            .unwrap()
            .insert(slot, block_hash.to_string());
        Ok(())
    }

    async fn mark_finalized(&self, slot: u64, block_hash: &str) -> Result<()> {
        self.finalized_slots.lock().unwrap().push(slot);
        self.block_hashes
            .lock()
            .unwrap()
            .insert(slot, block_hash.to_string());
        Ok(())
    }

    async fn get_tentative_transactions(&self, slot: u64) -> Result<Vec<String>> {
        let txs = self.tentative_transactions.lock().unwrap();
        Ok(txs.get(&slot).cloned().unwrap_or_default())
    }

    async fn rollback_slot(&self, slot: u64) -> Result<()> {
        let mut txs = self.tentative_transactions.lock().unwrap();
        if let Some(sigs) = txs.remove(&slot) {
            let mut tent_sigs = self.tentative_signatures.lock().unwrap();
            tent_sigs.retain(|s| !sigs.contains(s));
        }
        self.block_hashes.lock().unwrap().remove(&slot);
        Ok(())
    }

    async fn get_block_hash(&self, slot: u64) -> Result<Option<String>> {
        Ok(self.block_hashes.lock().unwrap().get(&slot).cloned())
    }

    async fn save_backfill_progress(&self, slot: u64) -> Result<()> {
        *self.backfill_progress.lock().unwrap() = Some(slot);
        Ok(())
    }

    async fn load_backfill_progress(&self) -> Result<Option<u64>> {
        Ok(*self.backfill_progress.lock().unwrap())
    }

    async fn mark_backfill_complete(&self) -> Result<()> {
        Ok(())
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
                        "accountKeys": ["11111111111111111111111111111111"],
                        "instructions": [],
                        "recentBlockhash": "11111111111111111111111111111111",
                        "header": {
                            "numRequiredSignatures": 1,
                            "numReadonlySignedAccounts": 0,
                            "numReadonlyUnsignedAccounts": 0
                        }
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
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), indexer.start()).await;

    // Verify transaction was marked as processed
    let is_processed = storage.is_processed(sig_str).await.unwrap();
    assert!(is_processed);
}

#[tokio::test]
async fn test_indexer_backfill() {
    let mock_server = MockServer::start().await;
    setup_rpc_mocks(&mock_server).await;

    let program_id = "11111111111111111111111111111111";
    let sig_str =
        "5j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia7";

    // Mock getBlock
    Mock::given(method("POST"))
        .and(body_string_contains("getBlock"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": {
                "blockhash": "headerhash",
                "previousBlockhash": "prevhash",
                "blockTime": 1678888888,
                "transactions": [
                    {
                        "transaction": {
                            "signatures": [sig_str],
                            "message": {
                                "accountKeys": [program_id, "other_account"],
                                "instructions": [],
                                "recentBlockhash": "headerhash",
                                "header": {
                                    "numRequiredSignatures": 1,
                                    "numReadonlySignedAccounts": 0,
                                    "numReadonlyUnsignedAccounts": 1
                                }
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
                    }
                ]
            },
            "id": 1
        })))
        .mount(&mock_server)
        .await;

    // Mock getTransaction (called by process_transaction_core if not mocking fetcher internals,
    // but BackfillEngine calls process_transaction_core which calls fetch_transaction.
    // fetch_transaction calls getTransaction.
    // So we need to mock getTransaction too even if we fetched block.
    // Wait, BackfillEngine logic fetches block, extracts signatures, then spawns task which calls process_transaction_core.
    // process_transaction_core calls fetcher.fetch_transaction.
    // So backfill re-fetches the transaction?
    // YES, currently process_transaction_core fetches it again.
    // This is inefficient (double fetch for backfill block based), but robust for reuse.
    // We should mock getTransaction as well.
    Mock::given(method("POST"))
        .and(body_string_contains("getTransaction"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": {
                "slot": 100,
                "blockTime": 1678888888,
                 "transaction": {
                    "signatures": [sig_str],
                    "message": {
                        "accountKeys": [program_id],
                        "instructions": [],
                        "recentBlockhash": "headerhash",
                        "header": {
                            "numRequiredSignatures": 1,
                            "numReadonlySignedAccounts": 0,
                            "numReadonlyUnsignedAccounts": 0
                        }
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

    use solana_indexer::config::BackfillConfig;

    let backfill_config = BackfillConfig {
        enabled: true,
        start_slot: Some(100),
        end_slot: Some(100),
        batch_size: 1,
        concurrency: 1,
        enable_reorg_handling: false, // simpler test
        finalization_check_interval: 1,
    };

    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(mock_server.uri())
        .with_database("postgresql://mock/db")
        .program_id(program_id)
        .with_backfill(backfill_config)
        .build()
        .unwrap();

    let storage = Arc::new(MockStorage::new());
    let indexer = SolanaIndexer::new_with_storage(config, storage.clone());

    // Run backfill
    indexer.start_backfill().await.unwrap();

    // Verify slot 100 was finalized
    {
        let slots = storage.finalized_slots.lock().unwrap();
        assert!(slots.contains(&100));
    }

    // Verify transaction processed and validated (in processed_signatures)
    // Note: process_transaction_core marks as finalized AND processed if is_finalized=true.
    {
        let sigs = storage.processed_signatures.lock().unwrap();
        assert!(sigs.contains(&sig_str.to_string()));
    }
}
