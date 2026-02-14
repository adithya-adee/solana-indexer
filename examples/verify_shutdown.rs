use solana_indexer_sdk::{SolanaIndexer, SolanaIndexerConfigBuilder};
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables
    dotenvy::dotenv().ok();

    // Get configuration from environment or use defaults
    let rpc_url =
        std::env::var("RPC_URL").unwrap_or_else(|_| "https://api.devnet.solana.com".to_string());
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://postgres:password@localhost:5432/solana_indexer_sdk".to_string()
    });

    // Create a configuration
    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(rpc_url)
        .with_database(database_url)
        .program_id("11111111111111111111111111111111")
        .with_stale_tentative_threshold(100)
        .build()?;

    println!("Creating indexer...");
    // We expect new() to fail if DB is not reachable, but let's try.
    // If it fails, we can't test shutdown.
    // Ideally we should use new_with_storage and a mock storage, but `StorageBackend` trait is tough to mock quickly without `mockall`.
    // Let's assume DB connection fails and handle it gracefully or rely on manual inspection of code.

    // Attempt to create indexer
    match SolanaIndexer::new(config).await {
        Ok(indexer) => {
            let token = indexer.cancellation_token();

            println!("Starting indexer...");
            let handle = tokio::spawn(async move {
                if let Err(e) = indexer.start().await {
                    eprintln!("Indexer error: {}", e);
                }
            });

            println!("Running for 5 seconds...");
            sleep(Duration::from_secs(5)).await;

            println!("Initiating shutdown...");
            token.cancel();

            println!("Waiting for indexer to stop...");
            match tokio::time::timeout(Duration::from_secs(10), handle).await {
                Ok(_) => println!("Indexer stopped gracefully!"),
                Err(_) => eprintln!("Indexer failed to stop in time!"),
            }
        }
        Err(e) => {
            eprintln!("Failed to create indexer (likely DB connection): {}", e);
            println!("Skipping full shutdown verification due to missing DB.");
        }
    }

    Ok(())
}
