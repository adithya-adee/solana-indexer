use criterion::{criterion_group, criterion_main, Criterion};
use serde_json::json;
use solana_indexer_sdk::{SolanaIndexer, SolanaIndexerConfigBuilder, Storage, StorageBackend};
use std::sync::Arc;
use tokio::runtime::Runtime;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn setup_mock_rpc() -> (MockServer, String) {
    let mock_server = MockServer::start().await;

    // Mock getVersion
    Mock::given(method("POST"))
        .and(path("/"))
        .and(body_string_contains("getVersion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": { "solana-core": "1.16.7", "feature-set": 0 },
            "id": 1
        })))
        .mount(&mock_server)
        .await;

    // Mock getLatestBlockhash
    Mock::given(method("POST"))
        .and(path("/"))
        .and(body_string_contains("getLatestBlockhash"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": {
                "context": { "slot": 1 },
                "value": { "blockhash": "hash", "lastValidBlockHeight": 100 }
            },
            "id": 1
        })))
        .mount(&mock_server)
        .await;

    // Mock getSignaturesForAddress
    let signatures: Vec<_> = (0..50)
        .map(|i| {
            json!({
                "signature": format!("bench_sig_{}", i),
                "slot": 100 + i,
                "err": null,
                "memo": null,
                "blockTime": 1000
            })
        })
        .collect();

    Mock::given(method("POST"))
        .and(path("/"))
        .and(body_string_contains("getSignaturesForAddress"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": signatures,
            "id": 1
        })))
        .mount(&mock_server)
        .await;

    // Mock getTransaction
    Mock::given(method("POST"))
        .and(path("/"))
        .and(body_string_contains("getTransaction"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": {
                "slot": 100,
                "transaction": {
                    "signatures": ["bench_sig_0"],
                    "message": { "accountKeys": [], "instructions": [], "recentBlockhash": "hash" }
                },
                "meta": { "err": null, "status": { "Ok": null }, "fee": 0, "preBalances": [], "postBalances": [], "innerInstructions": [] }
            },
            "id": 1
        })))
        .mount(&mock_server)
        .await;

    let uri = mock_server.uri();
    (mock_server, uri)
}

fn throughput_benchmark(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    // SAFETY: This is a benchmark setup
    unsafe { std::env::set_var("SOLANA_INDEXER_SILENT", "1") };

    // Try to load .env
    let _ = dotenvy::from_path("../solana-indexer-sdk/.env");

    let database_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("DATABASE_URL not set, skipping fuzzing");
            return;
        }
    };

    let (storage, _mock_server, rpc_url) = rt.block_on(async {
        let storage = Arc::new(
            Storage::new(&database_url)
                .await
                .expect("DB connection failed"),
        );
        storage
            .initialize()
            .await
            .expect("DB initialization failed");

        let (mock_server, rpc_url) = setup_mock_rpc().await;

        (storage, mock_server, rpc_url)
    });

    let storage_backend: Arc<dyn StorageBackend> = storage;

    let mut group = c.benchmark_group("throughput");
    group.sample_size(10); // Throughput tests are slow

    group.bench_function("indexer_pipeline_run", |b| {
        b.to_async(&rt).iter(|| {
            let config = SolanaIndexerConfigBuilder::new()
                .with_rpc(&rpc_url)
                .with_database(&database_url)
                .program_id("11111111111111111111111111111111")
                .with_batch_size(50)
                .with_poll_interval(1)
                // Use StartStrategy::Latest which will fetch getSignaturesForAddress
                .build()
                .unwrap();

            let indexer = SolanaIndexer::new_with_storage(config, storage_backend.clone());

            async move {
                // Run for a very short duration as one "iteration"
                let _ =
                    tokio::time::timeout(std::time::Duration::from_millis(100), indexer.start())
                        .await;
            }
        })
    });

    group.finish();
}

criterion_group!(benches, throughput_benchmark);
criterion_main!(benches);
