//! IDL-Based Indexer Example
//!
//! This example demonstrates how to use the IDL parser to automatically generate
//! Rust types from a Solana program's IDL file and use them in an indexer.
//!
//! ## Setup
//!
//! 1. Create a `build.rs` file in your project root:
//!    ```rust
//!    use std::env;
//!    use std::path::PathBuf;
//!
//!    fn main() {
//!        let idl_path = PathBuf::from("idl/my_program.json");
//!        let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
//!        let generated_path = out_dir.join("generated_types.rs");
//!
//!        solana_idl_parser::generate_sdk_types(&idl_path, &generated_path)
//!            .expect("Failed to generate types from IDL");
//!
//!        println!("cargo:rerun-if-changed={}", idl_path.display());
//!    }
//!    ```
//!
//! 2. Include the generated types in your code:
//!    ```rust
//!    include!(concat!(env!("OUT_DIR"), "/generated_types.rs"));
//!    ```
//!
//! 3. Use the generated types in your decoders and handlers.

use async_trait::async_trait;
use solana_indexer_sdk::{
    EventHandler, InstructionDecoder, LogDecoder, SolanaIndexer, SolanaIndexerConfigBuilder,
    SolanaIndexerError, TxMetadata,
};
use solana_transaction_status::UiInstruction;
use sqlx::PgPool;

// Include the generated types from IDL
// In a real project, this would be:
// include!(concat!(env!("OUT_DIR"), "/generated_types.rs"));
//
// For this example, we'll simulate what would be generated:
// - MyEvent struct with EventDiscriminator implementation
// - Other types and events from the IDL

// ================================================================================================
// Simulated Generated Types (normally from build.rs)
// ================================================================================================

// This is what would be generated from the IDL:
// #[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
// pub struct MyEvent {
//     pub data: u64,
// }
//
// impl MyEvent {
//     pub fn discriminator() -> [u8; 8] {
//         // Generated discriminator based on event name
//     }
// }
//
// impl EventDiscriminator for MyEvent {
//     fn discriminator() -> [u8; 8] {
//         Self::discriminator()
//     }
// }

// ================================================================================================
// Instruction Decoder for IDL-Generated Events
// ================================================================================================

/// Decoder that works with IDL-generated event types.
///
/// In a real implementation, you would decode instruction data using the
/// generated instruction argument types from the IDL.
pub struct IdlInstructionDecoder;

impl InstructionDecoder<()> for IdlInstructionDecoder {
    fn decode(&self, _instruction: &UiInstruction) -> Option<()> {
        // Decode instruction data based on IDL instruction definitions
        // This is a placeholder - in reality, you would:
        // 1. Check the instruction discriminator
        // 2. Deserialize instruction arguments using Borsh
        // 3. Return the appropriate event type
        None
    }
}

// ================================================================================================
// Log Decoder for IDL-Generated Events
// ================================================================================================

/// Decoder that extracts events from program logs.
///
/// This decoder looks for event discriminators in log messages and deserializes
/// the event data using the generated types.
pub struct IdlLogDecoder;

impl LogDecoder<()> for IdlLogDecoder {
    fn decode(&self, _log: &solana_indexer_sdk::ParsedEvent) -> Option<()> {
        // Decode log data based on IDL event definitions
        // This is a placeholder - in reality, you would:
        // 1. Check for event discriminator in log
        // 2. Extract event data
        // 3. Deserialize using BorshDeserialize
        // 4. Return the appropriate event type
        None
    }
}

// ================================================================================================
// Event Handler for IDL-Generated Events
// ================================================================================================

/// Handler for IDL-generated events.
///
/// This handler processes events that were generated from the IDL.
/// In a real implementation, you would handle specific event types
/// like `MyEvent`, `TransferEvent`, etc.
pub struct IdlEventHandler;

#[async_trait]
impl EventHandler<()> for IdlEventHandler {
    async fn handle(
        &self,
        _event: (),
        context: &TxMetadata,
        _db: &PgPool,
    ) -> Result<(), SolanaIndexerError> {
        println!(
            "✅ Processed IDL-generated event from transaction: {}",
            context.signature
        );
        // In a real application, you would:
        // 1. Extract event-specific data
        // 2. Insert into database
        // 3. Send notifications, etc.
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
    let indexer = SolanaIndexer::new(config).await?;

    // Register decoders
    // In a real implementation, you would register decoders for each
    // instruction type defined in your IDL
    // indexer.register_decoder("my_program", IdlInstructionDecoder)?;
    // indexer.register_log_decoder("my_program", IdlLogDecoder)?;

    // Register handlers
    // In a real implementation, you would register handlers for each
    // event type generated from your IDL
    // indexer.register_handler(IdlEventHandler)?;

    println!("▶️  Indexer configured. Press Ctrl+C to stop.");

    // Start the indexer
    indexer.start().await?;

    Ok(())
}
