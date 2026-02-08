//! System Transfer Indexer Example (WebSocket)
//!
//! This example demonstrates real-time indexing of System Program transfers using WebSocket.
//! It's identical to `system_transfer_indexer.rs` but uses WebSocket instead of polling.
//!
//! ## Usage
//!
//! Set environment variables in `.env`:
//! ```env
//! WS_URL=ws://127.0.0.1:8900
//! RPC_URL=http://127.0.0.1:8899
//! DATABASE_URL=postgresql://postgres:password@localhost/solana_indexer
//! PROGRAM_ID=11111111111111111111111111111111  # System Program
//! ```
//!
//! Run the example:
//! ```bash
//! cargo run --example system_transfer_indexer_ws
//! ```

use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_indexer::{
    EventDiscriminator, EventHandler, InstructionDecoder, SolanaIndexer,
    SolanaIndexerConfigBuilder, SolanaIndexerError, calculate_discriminator,
};
use solana_sdk::pubkey::Pubkey;
use solana_transaction_status::{UiInstruction, UiParsedInstruction};
use sqlx::PgPool;

// ================================================================================================
// Event Definition
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
// Instruction Decoder
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
// Event Handler
// ================================================================================================

pub struct SystemTransferHandler;

#[async_trait]
impl EventHandler<SystemTransferEvent> for SystemTransferHandler {
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

    async fn handle(
        &self,
        event: SystemTransferEvent,
        db: &PgPool,
        signature: &str,
    ) -> Result<(), SolanaIndexerError> {
        let sol_amount = event.amount as f64 / 1_000_000_000.0;

        println!(
            "📝 SOL Transfer: {} → {} ({:.9} SOL / {} lamports) [{}]",
            event.from, event.to, sol_amount, event.amount, signature
        );

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

    println!("🚀 System Transfer Indexer (WebSocket)\n");

    // ============================================================================================
    // Configuration
    // ============================================================================================

    let ws_url = std::env::var("WS_URL").expect("WS_URL must be set in .env");
    let rpc_url = std::env::var("RPC_URL").expect("RPC_URL must be set in .env");
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set in .env");
    let program_id = std::env::var("PROGRAM_ID")
        .unwrap_or_else(|_| "11111111111111111111111111111111".to_string());

    println!("📋 Configuration:");
    println!("   WebSocket URL: {}", ws_url);
    println!("   RPC URL: {}", rpc_url);
    println!("   Database: {}", database_url);
    println!("   Program ID: {}\n", program_id);

    // ============================================================================================
    // Create Indexer
    // ============================================================================================

    let config = SolanaIndexerConfigBuilder::new()
        .with_ws(ws_url, rpc_url)
        .with_database(database_url)
        .program_id(&program_id)
        .build()?;

    let mut indexer = SolanaIndexer::new(config).await?;

    // ============================================================================================
    // Register Decoder
    // ============================================================================================

    indexer.decoder_registry_mut().register(
        "system".to_string(),
        Box::new(
            Box::new(SystemTransferDecoder) as Box<dyn InstructionDecoder<SystemTransferEvent>>
        ),
    );

    // ============================================================================================
    // Register Handler
    // ============================================================================================

    let handler: Box<dyn EventHandler<SystemTransferEvent>> = Box::new(SystemTransferHandler);
    indexer
        .handler_registry_mut()
        .register(SystemTransferEvent::discriminator(), Box::new(handler));

    // ============================================================================================
    // Start Indexing
    // ============================================================================================

    println!("🔄 Starting real-time indexer with WebSocket...\n");
    indexer.start().await?;

    Ok(())
}
