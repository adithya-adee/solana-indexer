//! System Transfer Indexer Example - Advanced Showcase
//!
//! This example serves as a professional, production-ready template for building
//! a high-performance indexer. It demonstrates:
//!
//! 1. **Advanced Configuration**: Worker threads, registry limits, commitment levels.
//! 2. **Graceful Shutdown**: Programmatic shutdown using a `CancellationToken`.
//! 3. **Start Strategies**: How to resume from a checkpoint (`StartStrategy::Resume`).
//! 4. **Standard SDK Pattern**: The core Define->Decode->Handle pattern.
//!
//! ## Usage
//!
//! Set environment variables in `.env`:
//! ```env
//! RPC_URL=http://127.0.0.1:8899
//! DATABASE_URL=postgresql://postgres:password@localhost/solana_indexer_sdk
//! ```
//!
//! Run the example:
//! ```bash
//! cargo run --example rpc_system_transfer
//! ```

use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_indexer_sdk::{
    calculate_discriminator,
    config::{CommitmentLevel, RegistryConfig, StartStrategy},
    EventDiscriminator, EventHandler, InstructionDecoder, SolanaIndexer,
    SolanaIndexerConfigBuilder, SolanaIndexerError,
};
use solana_sdk::pubkey::Pubkey;
use solana_transaction_status::{UiInstruction, UiParsedInstruction};
use sqlx::PgPool;
use std::time::Duration;
use tokio::time::sleep;

// ================================================================================================
// Event Definition
// Events are typed data structures that represent the specific blockchain action we're interested in.
// ================================================================================================

/// Represents a System Program transfer event (native SOL transfer).
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct SystemTransferEvent {
    /// Source wallet public key
    pub from: Pubkey,
    /// Destination wallet public key
    pub to: Pubkey,
    /// Amount transferred in lamports
    pub amount: u64,
}

impl EventDiscriminator for SystemTransferEvent {
    /// Provides a unique identifier for the event type, allowing the SDK to route it to the correct handler.
    fn discriminator() -> [u8; 8] {
        calculate_discriminator("SystemTransferEvent")
    }
}

// ================================================================================================
// Instruction Decoder
// Decoders are responsible for parsing raw Solana instructions and extracting typed events.
// ================================================================================================

/// Decoder for System Program transfer instructions.
pub struct SystemTransferDecoder;

impl InstructionDecoder<SystemTransferEvent> for SystemTransferDecoder {
    /// Decodes a Solana instruction into a System transfer event.
    fn decode(&self, instruction: &UiInstruction) -> Option<SystemTransferEvent> {
        match instruction {
            UiInstruction::Parsed(UiParsedInstruction::Parsed(parsed)) => {
                // Ensure the instruction belongs to the expected program
                if parsed.program != "system" {
                    return None;
                }

                let parsed_info = parsed.parsed.as_object()?;
                let instruction_type = parsed_info.get("type")?.as_str()?;

                // Only process 'transfer' instructions
                if instruction_type != "transfer" {
                    return None;
                }

                let info = parsed_info.get("info")?.as_object()?;

                // Extract and parse the transfer details
                let source = info.get("source")?.as_str()?;
                let destination = info.get("destination")?.as_str()?;
                let lamports = info.get("lamports")?.as_u64()?;

                Some(SystemTransferEvent {
                    from: source.parse().ok()?,
                    to: destination.parse().ok()?,
                    amount: lamports,
                })
            }
            _ => None,
        }
    }
}

// ================================================================================================
// Event Handler
// Handlers implement the custom business logic for processing each extracted event.
// ================================================================================================

/// Handler for System Program transfer events.
pub struct SystemTransferHandler;

#[async_trait]
impl EventHandler<SystemTransferEvent> for SystemTransferHandler {
    /// Initializes the database schema. This is called once on startup.
    async fn initialize_schema(&self, db: &PgPool) -> Result<(), SolanaIndexerError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS system_transfers (
                signature TEXT PRIMARY KEY,
                from_wallet TEXT NOT NULL,
                to_wallet TEXT NOT NULL,
                amount_lamports BIGINT NOT NULL,
                amount_sol DECIMAL(20, 9) GENERATED ALWAYS AS (amount_lamports / 1000000000.0) STORED,
                indexed_at TIMESTAMPTZ DEFAULT NOW()
            )",
        )
        .execute(db)
        .await?;

        Ok(())
    }

    /// Processes a decoded event. This is where you perform database operations,
    /// trigger webhooks, or update caches.
    async fn handle(
        &self,
        event: SystemTransferEvent,
        context: &solana_indexer_sdk::TxMetadata,
        db: &PgPool,
    ) -> Result<(), SolanaIndexerError> {
        let signature = &context.signature;
        let sol_amount = event.amount as f64 / 1_000_000_000.0;

        println!(
            "📝 SOL Transfer: {} → {} ({:.9} SOL) [{}]",
            event.from, event.to, sol_amount, signature
        );

        // Store the event in the database using an idempotent query
        sqlx::query(
            "INSERT INTO system_transfers (signature, from_wallet, to_wallet, amount_lamports)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (signature) DO NOTHING",
        )
        .bind(signature)
        .bind(event.from.to_string())
        .bind(event.to.to_string())
        .bind(event.amount as i64)
        .execute(db)
        .await?;

        Ok(())
    }
}

// ================================================================================================
// Main Application
// ================================================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    println!("🚀 Advanced System Transfer Indexer Showcase\n");

    // ============================================================================================
    // 1. Advanced Configuration
    // ============================================================================================

    let rpc_url = std::env::var("RPC_URL")?;
    let database_url = std::env::var("DATABASE_URL")?;
    let program_id = std::env::var("PROGRAM_ID")
        .unwrap_or_else(|_| "11111111111111111111111111111111".to_string());

    // Configure registry limits to prevent unbounded memory growth in production.
    let registry_config = RegistryConfig {
        max_decoder_programs: 10,
        max_handlers: 10,
        enable_metrics: true, // Log registry stats periodically
        ..Default::default()
    };

    println!("📋 Configuration:");
    println!("   RPC URL: {}", rpc_url);
    println!("   Database: {}", database_url);
    println!("   Program ID: {}", program_id);

    // Build indexer configuration with advanced settings.
    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(rpc_url)
        .with_database(database_url.clone())
        .program_id(program_id)
        .with_poll_interval(2)
        .with_batch_size(10)
        .with_worker_threads(4) // Use 4 parallel threads for fetching transactions.
        .with_commitment(CommitmentLevel::Confirmed) // Index confirmed blocks.
        .with_start_strategy(StartStrategy::Resume) // Resume from the last processed signature.
        .with_registry_config(registry_config)
        .build()?;

    // ============================================================================================
    // 2. Indexer Setup
    // ============================================================================================

    let mut indexer = SolanaIndexer::new(config).await?;
    let token = indexer.cancellation_token(); // Get token for graceful shutdown.

    println!("   Worker Threads: {}", indexer.config().worker_threads);
    println!("   Commitment: {:?}", indexer.config().commitment_level);
    println!("   Start Strategy: {:?}", indexer.config().start_strategy);

    let handler = SystemTransferHandler;
    let db_pool = sqlx::PgPool::connect(&database_url).await?;
    handler.initialize_schema(&db_pool).await?;
    println!("\n✅ Database schema ready");

    // ============================================================================================
    // 3. Register Components
    // ============================================================================================

    indexer.decoder_registry_mut()?.register(
        "system".to_string(),
        Box::new(
            Box::new(SystemTransferDecoder) as Box<dyn InstructionDecoder<SystemTransferEvent>>
        ),
    )?;

    indexer.handler_registry_mut()?.register(
        SystemTransferEvent::discriminator(),
        Box::new(Box::new(handler) as Box<dyn EventHandler<SystemTransferEvent>>),
    )?;

    println!("✅ Decoder and Handler registered\n");

    // ============================================================================================
    // 4. Start Indexer with Graceful Shutdown
    // ============================================================================================
    println!("🔄 Starting indexer in background...");
    println!("   Will run for 15 seconds before graceful shutdown.\n");

    // Spawn the indexer to run in the background.
    let indexer_handle = tokio::spawn(async move { indexer.start().await });

    // In a real application, you would wait for a signal (e.g., another Ctrl+C handler,
    // a shutdown API call, or a parent task completing).
    sleep(Duration::from_secs(15)).await;

    // Initiate graceful shutdown.
    println!("\n⏰ 15 seconds elapsed. Initiating graceful shutdown...");
    token.cancel();

    // Wait for the indexer to finish its current work and stop.
    match tokio::time::timeout(Duration::from_secs(10), indexer_handle).await {
        Ok(Ok(Ok(_))) => println!("✅ Indexer shut down gracefully."),
        Ok(Ok(Err(e))) => eprintln!("❌ Indexer task exited with an error: {}", e),
        Ok(Err(e)) => eprintln!("❌ Indexer task failed to join: {}", e),
        Err(_) => eprintln!("❌ Indexer failed to shut down within 10 seconds."),
    }

    Ok(())
}
