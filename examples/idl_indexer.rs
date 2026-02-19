//! IDL-Based Indexer Example
//!
//! This example demonstrates how to use the IDL parser to automatically generate
//! Rust types from a Solana program's IDL file and use them in an indexer.
//!
//! ## Setup
//!
//! 1. The `build.rs` handles type generation from `idl/my_program.json`.
//! 2. The generated types are exported to `idl/types.rs`.
//! 3. We include those types and use them in our decoders and handlers.

use async_trait::async_trait;
use solana_indexer_sdk::{
    EventHandler, InstructionDecoder, LogDecoder, SolanaIndexer, SolanaIndexerConfigBuilder,
    SolanaIndexerError, TxMetadata,
};
use solana_transaction_status::UiInstruction;
use sqlx::PgPool;

// Include the Rust types generated from the IDL by build.rs
// We use the exported version in the idl directory
#[path = "idl/types.rs"]
mod types;
use types::*;

// ================================================================================================
// Instruction Decoder
// ================================================================================================

/// Decoder that deserializes instruction arguments using types generated from the IDL.
pub struct IdlInstructionDecoder;

impl InstructionDecoder<InitializeArgs> for IdlInstructionDecoder {
    fn decode(&self, instruction: &UiInstruction) -> Option<InitializeArgs> {
        if let UiInstruction::Compiled(compiled) = instruction {
            let data = bs58::decode(&compiled.data).into_vec().ok()?;
            // In a production environment, we would check the discriminator first.
            // For Anchor, it's usually the first 8 bytes.
            if data.len() >= 8 {
                let (_disc, args_data) = data.split_at(8);
                return borsh::from_slice::<InitializeArgs>(args_data).ok();
            }
        }
        None
    }
}

// ================================================================================================
// Log Decoder
// ================================================================================================

/// Decoder that extracts events from program logs using types generated from the IDL.
pub struct IdlLogDecoder;

impl LogDecoder<UserInitialized> for IdlLogDecoder {
    fn decode(&self, log: &solana_indexer_sdk::ParsedEvent) -> Option<UserInitialized> {
        if let Some(data) = &log.data {
            // Anchor events are typically base64-encoded in logs
            // This is a placeholder for actual Anchor event parsing logic
            if data.contains("Program log: ") {
                // Parse base64 and deserialize UserInitialized...
            }
        }
        None
    }
}

// ================================================================================================
// Event Handler
// ================================================================================================

/// Handler for processing the `UserInitialized` event.
pub struct IdlEventHandler;

#[async_trait]
impl EventHandler<UserInitialized> for IdlEventHandler {
    async fn handle(
        &self,
        event: UserInitialized,
        context: &TxMetadata,
        _db: &PgPool,
    ) -> Result<(), SolanaIndexerError> {
        println!(
            "✅ Processed IDL-generated event 'UserInitialized' from transaction: {}",
            context.signature
        );
        println!("   User Pubkey: {}", event.user);
        println!("   User Name:   {}", event.name);

        Ok(())
    }
}

// ================================================================================================
// Instruction Handler
// ================================================================================================

/// Handler for processing the `Initialize` instruction arguments.
///
/// This demonstrates how you can index instruction inputs directly.
pub struct InitializeHandler;

#[async_trait]
impl EventHandler<InitializeArgs> for InitializeHandler {
    async fn handle(
        &self,
        args: InitializeArgs,
        context: &TxMetadata,
        _db: &PgPool,
    ) -> Result<(), SolanaIndexerError> {
        println!(
            "✅ Processed IDL-generated instruction 'Initialize' from transaction: {}",
            context.signature
        );
        println!("   Args - Name: {}, Age: {}", args.name, args.age);
        Ok(())
    }
}

// ================================================================================================
// Main Function
// ================================================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables
    dotenvy::dotenv().ok();

    let rpc_url =
        std::env::var("RPC_URL").unwrap_or_else(|_| "https://api.devnet.solana.com".to_string());
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost:5432/indexer".to_string());
    let program_id = std::env::var("PROGRAM_ID")
        .unwrap_or_else(|_| "11111111111111111111111111111111".to_string());

    println!("🚀 Starting IDL-based indexer...");
    println!("   RPC URL: {}", rpc_url);
    println!("   Program ID: {}", program_id);

    // Configure the indexer
    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(&rpc_url)
        .with_database(&database_url)
        .program_id(&program_id)
        .build()?;

    // Create the indexer
    let mut indexer = SolanaIndexer::new(config).await?;

    // Register decoders for types generated from IDL
    indexer.register_decoder("my_program", IdlInstructionDecoder)?;
    indexer.register_log_decoder("my_program", IdlLogDecoder)?;

    // Register handlers for both events and instructions
    indexer.register_handler(IdlEventHandler)?;
    indexer.register_handler(InitializeHandler)?;

    println!("▶️  Indexer configured. Press Ctrl+C to stop.");

    // Start the indexer
    indexer.start().await?;

    Ok(())
}
