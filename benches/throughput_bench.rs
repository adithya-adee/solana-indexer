use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use serde_json::json;
use solana_indexer::{SolanaIndexer, SolanaIndexerConfigBuilder, Storage};
use std::sync::Arc;
use tokio::runtime::Runtime;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

use solana_indexer::utils::logging::log_section;

fn bench_indexer_pipeline(c: &mut Criterion) {
    // SAFETY: This is a benchmark setup, no other threads should be accessing env vars concurrently.
    unsafe { std::env::set_var("SOLANA_INDEXER_SILENT", "1") };
    let rt = Runtime::new().unwrap();
    dotenvy::dotenv().ok();

    log_section("Starting Pipeline Throughput Benchmark");

    let database_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("DATABASE_URL not set, skipping throughput benchmarks");
            return;
        }
    };

    let mut group = c.benchmark_group("pipeline_throughput");
    group.throughput(Throughput::Elements(1));

    // Measure throughput with different batch sizes
    for batch_size in [1, 10, 50].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            batch_size,
            |b, &size| {
                b.to_async(&rt).iter(|| async {
                    let mock_server = MockServer::start().await;

                    // Setup mocks
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
                                "value": { "blockhash": "hash", "lastValidBlockHeight": 100 }
                            },
                            "id": 1
                        })))
                        .mount(&mock_server)
                        .await;

                    // Mock getSignaturesForAddress to return 'size' signatures
                    let mut signatures = Vec::new();
                    for i in 0..size {
                        signatures.push(json!({
                            "signature": format!("bench_sig_{}", i),
                            "slot": 100 + i,
                            "err": null,
                            "memo": null,
                            "blockTime": 1000
                        }));
                    }

                    Mock::given(method("POST"))
                        .and(body_string_contains("getSignaturesForAddress"))
                        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                            "jsonrpc": "2.0",
                            "result": signatures,
                            "id": 1
                        })))
                        .mount(&mock_server)
                        .await;

                    // Mock getTransaction batch
                    Mock::given(method("POST"))
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

                    let storage = Arc::new(Storage::new(&database_url).await.expect("DB"));
                    storage.initialize().await.expect("Init");

                    let config = SolanaIndexerConfigBuilder::new()
                        .with_rpc(mock_server.uri())
                        .with_database(&database_url)
                        .program_id("11111111111111111111111111111111")
                        .with_batch_size(size as usize)
                        .with_poll_interval(1)
                        .build()
                        .unwrap();

                    let indexer = SolanaIndexer::new_with_storage(config, storage);

                    // Run for short duration
                    let _ =
                        tokio::time::timeout(std::time::Duration::from_millis(100), indexer.start())
                            .await;
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_indexer_pipeline);
criterion_main!(benches);
