//! Backfill Indexer Example
//!
//! This example demonstrates how to configure and run the backfill engine to index
//! historical data. It showcases:
//!
//! 1. **Backfill Configuration**: Setting slot ranges and concurrency.
//! 2. **Reorg Handling**: Enabling automatic reorg detection and recovery.
//! 3. **Progress Tracking**: Resuming from saved checkpoints.
//!
//! ## Usage
//!
//! ```bash
//! RPC_URL=https://api.mainnet-beta.solana.com \
//! DATABASE_URL=postgres://... \
//! PROGRAM_ID=... \
//! cargo run --example backfill_indexer
//! ```
use solana_indexer::config::BackfillConfig;
use solana_indexer::{SolanaIndexer, SolanaIndexerConfigBuilder};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables
    dotenvy::dotenv().ok();

    // Default configuration from env
    let rpc_url =
        env::var("RPC_URL").unwrap_or_else(|_| "https://api.devnet.solana.com".to_string());
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let program_id = env::var("PROGRAM_ID").expect("PROGRAM_ID must be set");

    // Configure backfill settings
    let backfill_config = BackfillConfig {
        enabled: true,
        // Start from a specific slot (set to None to start from the earliest available or 0).
        // If resumption is enabled, this will be overridden by the last saved progress if it's higher.
        start_slot: Some(250_000_000),

        // End at a specific slot (set to None to continue until the latest finalized block).
        end_slot: Some(250_001_000),

        // Number of transactions to fetch per batch.
        batch_size: 10,

        // Number of concurrent RPC requests. Adjust based on your RPC provider's rate limits.
        concurrency: 5,

        // Enable reorg handling to detect and rollback invalidated blocks.
        enable_reorg_handling: true,

        // Interval (in slots) to check for block finalization updates.
        finalization_check_interval: 100,
    };

    println!("Configuring backfill indexer for program: {}", program_id);
    println!(
        "Backfill range: {:?} to {:?}",
        backfill_config.start_slot, backfill_config.end_slot
    );

    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(rpc_url)
        .with_database(db_url)
        .program_id(program_id)
        .with_backfill(backfill_config)
        .build()?;

    // Create indexer
    let indexer = SolanaIndexer::new(config).await?;

    // Initialize database
    indexer.storage().initialize().await?;

    // Start backfill process
    println!("Starting backfill...");
    if let Err(e) = indexer.start_backfill().await {
        eprintln!("Backfill failed: {}", e);
        // Depending on strategy, you might want to exit or continue to real-time indexing
    } else {
        println!("Backfill completed successfully!");
    }

    // Start real-time indexing
    // println!("Starting real-time indexing...");
    // indexer.start().await?;

    Ok(())
}
