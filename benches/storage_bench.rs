use criterion::{Criterion, criterion_group, criterion_main}; // Removed BenchmarkId
use solana_indexer::{Storage, utils::logging::log_section};
use tokio::runtime::Runtime;

fn bench_storage_operations(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    dotenvy::dotenv().ok();

    log_section("Starting Storage Benchmark");

    // Skip if DATABASE_URL not set
    let database_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("DATABASE_URL not set, skipping storage benchmarks");
            return;
        }
    };

    let storage = rt.block_on(async {
        let s = Storage::new(&database_url)
            .await
            .expect("Failed to connect");
        s.initialize().await.expect("Failed to initialize");
        std::sync::Arc::new(s)
    });

    let mut group = c.benchmark_group("storage");

    group.bench_function("is_processed", |b| {
        b.to_async(&rt).iter(|| async {
            storage
                .is_processed("bench_signature_123")
                .await
                .expect("Failed to check")
        })
    });

    group.bench_function("mark_processed", |b| {
        use std::sync::atomic::{AtomicU64, Ordering};
        let counter = AtomicU64::new(0);
        let storage = storage.clone();
        b.to_async(&rt).iter(|| async {
            let val = counter.fetch_add(1, Ordering::Relaxed);
            let sig = format!("bench_sig_{}", val);
            storage
                .mark_processed(&sig, 12345)
                .await
                .expect("Failed to mark")
        })
    });

    group.bench_function("get_last_processed_slot", |b| {
        b.to_async(&rt).iter(|| async {
            storage
                .get_last_processed_slot()
                .await
                .expect("Failed to get slot")
        })
    });

    group.finish();
}

criterion_group!(benches, bench_storage_operations);
criterion_main!(benches);
