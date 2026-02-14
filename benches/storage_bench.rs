use criterion::{criterion_group, criterion_main, Criterion};
use solana_indexer_sdk::{Storage, StorageBackend};
use std::sync::Arc;
use tokio::runtime::Runtime;

async fn setup_storage() -> Option<Arc<dyn StorageBackend>> {
    // Try to load .env from SDK directory
    let _ = dotenvy::from_path("../solana-indexer-sdk/.env");

    let database_url = std::env::var("DATABASE_URL").ok()?;

    let storage = Storage::new(&database_url)
        .await
        .expect("Failed to connect");
    storage.initialize().await.expect("Failed to initialize");
    Some(Arc::new(storage))
}

fn storage_benchmark(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    // Initialize storage outside the benchmark loop
    let storage = rt.block_on(setup_storage());

    if storage.is_none() {
        eprintln!("DATABASE_URL not set, skipping storage benchmarks");
        return;
    }
    let storage = storage.unwrap();

    let mut group = c.benchmark_group("storage");

    // Benchmark writing (mark_processed)
    group.bench_function("mark_processed", |b| {
        b.to_async(&rt).iter_custom(|iters| {
            let storage = storage.clone();
            async move {
                let start = std::time::Instant::now();
                for i in 0..iters {
                    let sig = format!(
                        "bench_write_{}_{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_nanos(),
                        i
                    );
                    storage.mark_processed(&sig, 12345).await.unwrap();
                }
                start.elapsed()
            }
        })
    });

    // Benchmark reading (is_processed)
    // Pre-populate a record to read
    let read_sig = "bench_read_static_signature";
    rt.block_on(async {
        storage.mark_processed(read_sig, 12345).await.unwrap();
    });

    group.bench_function("is_processed", |b| {
        b.to_async(&rt).iter(|| async {
            storage.is_processed(read_sig).await.unwrap();
        })
    });

    group.finish();
}

criterion_group!(benches, storage_benchmark);
criterion_main!(benches);
