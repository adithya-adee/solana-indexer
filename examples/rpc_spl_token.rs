//! SPL Token Transfer Indexer Example
//!
//! This example demonstrates the Solana Indexer SDK's flexible architecture by building
//! a custom indexer for SPL token transfers. It showcases:
//!
//! 1. **Custom Instruction Decoding** - Parsing SPL token transfer instructions
//! 2. **Custom Event Handling** - Processing and storing transfer data in PostgreSQL
//! 3. **Multi-Program Support** - Using the DecoderRegistry pattern
//!
//! ## Architecture
//!
//! ```text
//! Transaction → InstructionDecoder<T> → Option<T> → EventHandler<T> → Database
//!               (Parse instruction)     (Typed      (Process event)
//!                                        Event)
//! ```
//!
//! ## Usage
//!
//! Set environment variables in `.env`:
//! ```env
//! RPC_URL=http://127.0.0.1:8899
//! DATABASE_URL=postgresql://postgres:password@localhost/solana_indexer
//! ```
//!
//! Run the example:
//! ```bash
//! cargo run --example spl_token_indexer
//! ```

use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_indexer::{
    EventDiscriminator, EventHandler, InstructionDecoder, SolanaIndexerConfigBuilder,
    SolanaIndexerError, calculate_discriminator,
};
use solana_sdk::pubkey::Pubkey;
use solana_transaction_status::{UiInstruction, UiParsedInstruction};
use sqlx::PgPool;

// ================================================================================================
// Event Definition
// ================================================================================================

/// Represents an SPL token transfer event.
///
/// This struct captures the essential information from an SPL token transfer:
/// - Source wallet (from)
/// - Destination wallet (to)
/// - Transfer amount (in token's smallest unit)
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct SplTransferEvent {
    /// Source wallet public key
    pub from: Pubkey,
    /// Destination wallet public key
    pub to: Pubkey,
    /// Amount transferred (in token's smallest unit, e.g., lamports for SOL)
    pub amount: u64,
}

impl EventDiscriminator for SplTransferEvent {
    /// Returns the 8-byte discriminator for this event type.
    ///
    /// The discriminator is a hash of the event name, used by the SDK to route
    /// events to the correct handler.
    fn discriminator() -> [u8; 8] {
        calculate_discriminator("SplTransferEvent")
    }
}

// ================================================================================================
// Instruction Decoder
// ================================================================================================

/// Decoder for SPL token transfer instructions.
///
/// This decoder parses raw Solana instructions and extracts SPL token transfer data.
/// It handles both `transfer` and `transferChecked` instruction types.
pub struct SplTransferDecoder;

impl InstructionDecoder<SplTransferEvent> for SplTransferDecoder {
    /// Decodes a Solana instruction into an SPL transfer event.
    ///
    /// # Arguments
    ///
    /// * `instruction` - The UI instruction from the transaction
    ///
    /// # Returns
    ///
    /// * `Some(SplTransferEvent)` - If the instruction is a valid SPL token transfer
    /// * `None` - If the instruction is not an SPL token transfer or parsing fails
    fn decode(&self, instruction: &UiInstruction) -> Option<SplTransferEvent> {
        // Only process parsed instructions
        match instruction {
            UiInstruction::Parsed(UiParsedInstruction::Parsed(parsed)) => {
                // Verify this is an SPL token program instruction
                if parsed.program != "spl-token" {
                    return None;
                }

                // Extract the instruction type and info
                let parsed_info = parsed.parsed.as_object()?;
                let instruction_type = parsed_info.get("type")?.as_str()?;

                // Only process transfer and transferChecked instructions
                if instruction_type != "transfer" && instruction_type != "transferChecked" {
                    return None;
                }

                // Extract transfer details from the instruction info
                let info = parsed_info.get("info")?.as_object()?;

                // Get source and destination wallet addresses
                let source = info.get("source")?.as_str()?;
                let destination = info.get("destination")?.as_str()?;

                // Extract amount - handle both transfer types
                // - transfer: amount is a direct field
                // - transferChecked: amount is nested in tokenAmount object
                let amount = if instruction_type == "transferChecked" {
                    let token_amount = info.get("tokenAmount")?.as_object()?;
                    token_amount.get("amount")?.as_str()?.parse::<u64>().ok()?
                } else {
                    info.get("amount")?.as_str()?.parse::<u64>().ok()?
                };

                // Parse addresses and create event
                Some(SplTransferEvent {
                    from: source.parse().ok()?,
                    to: destination.parse().ok()?,
                    amount,
                })
            }
            // Ignore non-parsed instructions
            _ => None,
        }
    }
}

// ================================================================================================
// Event Handler
// ================================================================================================

/// Handler for SPL token transfer events.
///
/// This handler:
/// 1. Creates the database schema (spl_transfers table)
/// 2. Processes each transfer event by inserting it into the database
/// 3. Ensures idempotency using the transaction signature as primary key
pub struct SplTransferHandler;

#[async_trait]
impl EventHandler<SplTransferEvent> for SplTransferHandler {
    /// Initializes the database schema for SPL transfers.
    ///
    /// Creates the `spl_transfers` table if it doesn't exist. This method is called
    /// once during indexer startup.
    ///
    /// # Schema
    ///
    /// - `signature`: Transaction signature (primary key for idempotency)
    /// - `from_wallet`: Source wallet address
    /// - `to_wallet`: Destination wallet address
    /// - `amount`: Transfer amount in token's smallest unit
    /// - `indexed_at`: Timestamp when the transfer was indexed
    async fn initialize_schema(&self, db: &PgPool) -> Result<(), SolanaIndexerError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS spl_transfers (
                signature TEXT PRIMARY KEY,
                from_wallet TEXT NOT NULL,
                to_wallet TEXT NOT NULL,
                amount BIGINT NOT NULL,
                indexed_at TIMESTAMPTZ DEFAULT NOW()
            )",
        )
        .execute(db)
        .await?;

        Ok(())
    }

    /// Handles an SPL transfer event by storing it in the database.
    ///
    /// # Arguments
    ///
    /// * `event` - The decoded SPL transfer event
    /// * `db` - Database connection pool
    /// * `signature` - Transaction signature (for idempotency)
    ///
    /// # Idempotency
    ///
    /// Uses `ON CONFLICT DO NOTHING` to ensure the same transaction is not
    /// processed multiple times.
    async fn handle(
        &self,
        event: SplTransferEvent,
        db: &PgPool,
        signature: &str,
    ) -> Result<(), SolanaIndexerError> {
        // Log the transfer for monitoring
        println!(
            "📝 SPL Transfer: {} → {} ({} tokens) [{}]",
            event.from, event.to, event.amount, signature
        );

        // Insert into database with idempotency guarantee
        sqlx::query(
            "INSERT INTO spl_transfers (signature, from_wallet, to_wallet, amount)
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
    // Load environment variables from .env file
    dotenvy::dotenv().ok();

    println!("🚀 SPL Token Transfer Indexer\n");
    println!("This indexer monitors and stores all SPL token transfers on Solana.\n");

    // ============================================================================================
    // Configuration
    // ============================================================================================

    let rpc_url = std::env::var("RPC_URL").expect("RPC_URL must be set in .env");
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set in .env");
    let program_id = std::env::var("PROGRAM_ID").expect("PROGRAM_ID must be set in .env");

    println!("📋 Configuration:");
    println!("   RPC URL: {}", rpc_url);
    println!("   Database: {}", database_url);
    println!("   Program ID: {}", program_id);
    println!("   Program: SPL Token Program\n");

    // Build indexer configuration
    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(rpc_url)
        .with_database(database_url.clone())
        .program_id(program_id)
        .with_poll_interval(2) // Poll every 2 seconds for new transactions
        .with_batch_size(10) // Fetch up to 10 transactions per batch
        .build()?;

    // ============================================================================================
    // Indexer Setup
    // ============================================================================================

    // Create the indexer instance
    let mut indexer = solana_indexer::SolanaIndexer::new(config).await?;

    // Initialize database schema
    println!("📊 Initializing database schema...");
    let handler = SplTransferHandler;
    let db_pool = sqlx::PgPool::connect(&database_url).await?;
    handler.initialize_schema(&db_pool).await?;
    println!("✅ Database schema ready\n");

    // ============================================================================================
    // Register Decoder
    // ============================================================================================

    println!("🔧 Registering instruction decoder...");

    // Register the SPL Transfer decoder with the decoder registry
    // The decoder registry matches by program name (e.g., "spl-token", "system")
    // NOT by program ID (the pubkey)
    indexer.decoder_registry_mut().register(
        "spl-token".to_string(), // Program name as it appears in parsed instructions
        Box::new(Box::new(SplTransferDecoder) as Box<dyn InstructionDecoder<SplTransferEvent>>),
    )?;

    println!("✅ Decoder registered for 'spl-token' program\n");

    // ============================================================================================
    // Register Handler
    // ============================================================================================

    println!("🔧 Registering event handler...");

    // Wrap the handler in a Box for dynamic dispatch
    let handler_box: Box<dyn EventHandler<SplTransferEvent>> = Box::new(handler);

    // Register the handler with the indexer
    // The SDK will automatically route SplTransferEvent instances to this handler
    indexer
        .handler_registry_mut()
        .register(SplTransferEvent::discriminator(), Box::new(handler_box))?;

    println!("✅ Handler registered\n");

    // ============================================================================================
    // Start Indexing
    // ============================================================================================

    println!("🔄 Starting indexer...");
    println!("   Monitoring SPL token transfers on Solana");
    println!("   Press Ctrl+C to stop\n");
    println!("{}", "=".repeat(80));

    // Start the indexer - this runs indefinitely until interrupted
    indexer.start().await?;

    Ok(())
}
