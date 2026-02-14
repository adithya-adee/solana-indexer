//! Helius System Transfer Indexer Example
//!
//! This example limits the `rpc_system_transfer.rs` example but uses Helius
//! as the high-performance data source.
//!
//! ## Usage
//!
//! Set environment variables in `.env`:
//! ```env
//! HELIUS_API_KEY=your_api_key
//! DATABASE_URL=postgresql://postgres:password@localhost/solana_indexer_sdk
//! ```
//!
//! Run the example:
//! ```bash
//! cargo run --example helius_system_transfer
//! ```

use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_indexer_sdk::{
    calculate_discriminator, config::HeliusNetwork, EventDiscriminator, EventHandler,
    InstructionDecoder, SolanaIndexerConfigBuilder, SolanaIndexerError,
};
use solana_sdk::pubkey::Pubkey;
use solana_transaction_status::{UiInstruction, UiParsedInstruction};
use sqlx::PgPool;

// ================================================================================================
// Event Definition (Same as rpc_system_transfer.rs)
// ================================================================================================

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct SystemTransferEvent {
    pub from: Pubkey,
    pub to: Pubkey,
    pub amount: u64,
}

impl EventDiscriminator for SystemTransferEvent {
    fn discriminator() -> [u8; 8] {
        calculate_discriminator("SystemTransferEvent")
    }
}

// ================================================================================================
// Instruction Decoder (Same as rpc_system_transfer.rs)
// ================================================================================================

pub struct SystemTransferDecoder;

impl InstructionDecoder<SystemTransferEvent> for SystemTransferDecoder {
    fn decode(&self, instruction: &UiInstruction) -> Option<SystemTransferEvent> {
        match instruction {
            UiInstruction::Parsed(UiParsedInstruction::Parsed(parsed)) => {
                if parsed.program != "system" {
                    return None;
                }
                let parsed_info = parsed.parsed.as_object()?;
                let instruction_type = parsed_info.get("type")?.as_str()?;
                if instruction_type != "transfer" {
                    return None;
                }
                let info = parsed_info.get("info")?.as_object()?;
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
// Event Handler (Same as rpc_system_transfer.rs)
// ================================================================================================

pub struct SystemTransferHandler;

#[async_trait]
impl EventHandler<SystemTransferEvent> for SystemTransferHandler {
    async fn initialize_schema(&self, db: &PgPool) -> Result<(), SolanaIndexerError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS helius_system_transfers (
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

    async fn handle(
        &self,
        event: SystemTransferEvent,
        context: &solana_indexer_sdk::TxMetadata,
        db: &PgPool,
    ) -> Result<(), SolanaIndexerError> {
        let signature = &context.signature;
        let sol_amount = event.amount as f64 / 1_000_000_000.0;
        println!(
            "⚡ Helius Transfer: {} → {} ({:.9} SOL) [{}]",
            event.from, event.to, sol_amount, signature
        );

        sqlx::query(
            "INSERT INTO helius_system_transfers (signature, from_wallet, to_wallet, amount_lamports)
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

    println!("🚀 Helius System Transfer Indexer\n");

    let api_key = std::env::var("HELIUS_API_KEY")?;
    let database_url = std::env::var("DATABASE_URL")?;
    let program_id = std::env::var("PROGRAM_ID")
        .unwrap_or_else(|_| "11111111111111111111111111111111".to_string());

    let network_str = std::env::var("HELIUS_NETWORK").unwrap_or_else(|_| "mainnet".to_string());
    let network = match network_str.to_lowercase().as_str() {
        "mainnet" => HeliusNetwork::Mainnet,
        "devnet" => HeliusNetwork::Devnet,
        _ => return Err("Invalid HELIUS_NETWORK: must be 'mainnet' or 'devnet'".into()),
    };

    println!("📋 Configuration:");
    println!("   Source: Helius ({:?})", network);
    println!("   Database: {}", database_url);
    println!("   Program: {}\n", program_id);
    println!("ℹ️  Note: Helius Free Tier has rate limits (e.g. 10 TPS).");
    println!("    If you encounter 429 errors, consider upgrading or implementing a retry queue.");

    // Default to WebSocket enabled (true). Pass false to use RPC polling only.
    let use_websocket = false;

    if !use_websocket {
        println!("ℹ️  Mode: RPC Polling Only (WebSocket disabled)");
    } else {
        println!("ℹ️  Mode: WebSocket + RPC (Default)");
    }

    let config = SolanaIndexerConfigBuilder::new()
        .with_helius_network(api_key, network, use_websocket)
        .with_database(database_url.clone())
        .program_id(program_id)
        .build()?;

    let mut indexer = solana_indexer_sdk::SolanaIndexer::new(config).await?;

    // Initialize schema
    let handler = SystemTransferHandler;
    let db_pool = sqlx::PgPool::connect(&database_url).await?;
    handler.initialize_schema(&db_pool).await?;

    // Register Decoder
    indexer.decoder_registry_mut()?.register(
        "system".to_string(),
        Box::new(
            Box::new(SystemTransferDecoder) as Box<dyn InstructionDecoder<SystemTransferEvent>>
        ),
    )?;

    // Register Handler
    indexer.handler_registry_mut()?.register(
        SystemTransferEvent::discriminator(),
        Box::new(Box::new(handler) as Box<dyn EventHandler<SystemTransferEvent>>),
    )?;

    println!("🔄 Starting Helius indexer...");

    // Get cancellation token before moving indexer
    let token = indexer.cancellation_token();

    // Spawn the indexer in a background task
    let indexer_handle = tokio::spawn(async move { indexer.start().await });

    // Wait for Ctrl+C
    tokio::select! {
        result = indexer_handle => {
            match result {
                Ok(Ok(())) => println!("✅ Indexer completed successfully."),
                Ok(Err(e)) => eprintln!("❌ Indexer error: {}", e),
                Err(e) => eprintln!("❌ Indexer task panicked: {}", e),
            }
        }
        _ = tokio::signal::ctrl_c() => {
            println!("\n⏰ Ctrl+C received. Initiating graceful shutdown...");
            token.cancel();
            println!("✅ Shutdown signal sent. Indexer will stop gracefully.");
        }
    }

    Ok(())
}
