//! System Transfer Indexer Example
//!
//! This example demonstrates the Solana Indexer SDK's flexible architecture by building
//! a custom indexer for System Program transfers (native SOL transfers). It showcases:
//!
//! 1. **Custom Instruction Decoding** - Parsing System Program transfer instructions
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
//! PROGRAM_ID=11111111111111111111111111111111  # System Program
//! ```
//!
//! Run the example:
//! ```bash
//! cargo run --example system_transfer_indexer
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

/// Represents a System Program transfer event (native SOL transfer).
///
/// This struct captures the essential information from a System Program transfer:
/// - Source wallet (from)
/// - Destination wallet (to)
/// - Transfer amount (in lamports)
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct SystemTransferEvent {
    /// Source wallet public key
    pub from: Pubkey,
    /// Destination wallet public key
    pub to: Pubkey,
    /// Amount transferred in lamports (1 SOL = 1,000,000,000 lamports)
    pub amount: u64,
}

impl EventDiscriminator for SystemTransferEvent {
    /// Returns the 8-byte discriminator for this event type.
    ///
    /// The discriminator is a hash of the event name, used by the SDK to route
    /// events to the correct handler.
    fn discriminator() -> [u8; 8] {
        calculate_discriminator("SystemTransferEvent")
    }
}

// ================================================================================================
// Instruction Decoder
// ================================================================================================

/// Decoder for System Program transfer instructions.
///
/// This decoder parses raw Solana instructions and extracts System Program transfer data.
/// It handles the `transfer` instruction type from the System Program.
pub struct SystemTransferDecoder;

impl InstructionDecoder<SystemTransferEvent> for SystemTransferDecoder {
    /// Decodes a Solana instruction into a System transfer event.
    ///
    /// # Arguments
    ///
    /// * `instruction` - The UI instruction from the transaction
    ///
    /// # Returns
    ///
    /// * `Some(SystemTransferEvent)` - If the instruction is a valid System Program transfer
    /// * `None` - If the instruction is not a System Program transfer or parsing fails
    fn decode(&self, instruction: &UiInstruction) -> Option<SystemTransferEvent> {
        // Only process parsed instructions
        match instruction {
            UiInstruction::Parsed(UiParsedInstruction::Parsed(parsed)) => {
                // Verify this is a System Program instruction
                if parsed.program != "system" {
                    return None;
                }

                // Extract the instruction type and info
                let parsed_info = parsed.parsed.as_object()?;
                let instruction_type = parsed_info.get("type")?.as_str()?;

                // Only process transfer instructions
                if instruction_type != "transfer" {
                    return None;
                }

                // Extract transfer details from the instruction info
                let info = parsed_info.get("info")?.as_object()?;

                // Get source and destination wallet addresses
                let source = info.get("source")?.as_str()?;
                let destination = info.get("destination")?.as_str()?;

                // Get amount in lamports
                let lamports = info.get("lamports")?.as_u64()?;

                // Parse addresses and create event
                Some(SystemTransferEvent {
                    from: source.parse().ok()?,
                    to: destination.parse().ok()?,
                    amount: lamports,
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

/// Handler for System Program transfer events.
///
/// This handler:
/// 1. Creates the database schema (system_transfers table)
/// 2. Processes each transfer event by inserting it into the database
/// 3. Ensures idempotency using the transaction signature as primary key
pub struct SystemTransferHandler;

#[async_trait]
impl EventHandler<SystemTransferEvent> for SystemTransferHandler {
    /// Initializes the database schema for System transfers.
    ///
    /// Creates the `system_transfers` table if it doesn't exist. This method is called
    /// once during indexer startup.
    ///
    /// # Schema
    ///
    /// - `signature`: Transaction signature (primary key for idempotency)
    /// - `from_wallet`: Source wallet address
    /// - `to_wallet`: Destination wallet address
    /// - `amount_lamports`: Transfer amount in lamports
    /// - `amount_sol`: Transfer amount in SOL (computed)
    /// - `indexed_at`: Timestamp when the transfer was indexed
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

    /// Handles a System transfer event by storing it in the database.
    ///
    /// # Arguments
    ///
    /// * `event` - The decoded System transfer event
    /// * `db` - Database connection pool
    /// * `signature` - Transaction signature (for idempotency)
    ///
    /// # Idempotency
    ///
    /// Uses `ON CONFLICT DO NOTHING` to ensure the same transaction is not
    /// processed multiple times.
    async fn handle(
        &self,
        event: SystemTransferEvent,
        db: &PgPool,
        signature: &str,
    ) -> Result<(), SolanaIndexerError> {
        // Convert lamports to SOL for display
        let sol_amount = event.amount as f64 / 1_000_000_000.0;

        // Log the transfer for monitoring
        println!(
            "📝 SOL Transfer: {} → {} ({:.9} SOL / {} lamports) [{}]",
            event.from, event.to, sol_amount, event.amount, signature
        );

        // Insert into database with idempotency guarantee
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
    // Load environment variables from .env file
    dotenvy::dotenv().ok();

    println!("🚀 System Transfer Indexer (Native SOL Transfers)\n");
    println!("This indexer monitors and stores all native SOL transfers on Solana.\n");

    // ============================================================================================
    // Configuration
    // ============================================================================================

    let rpc_url = std::env::var("RPC_URL").expect("RPC_URL must be set in .env");
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set in .env");
    let program_id = std::env::var("PROGRAM_ID")
        .unwrap_or_else(|_| "11111111111111111111111111111111".to_string());

    println!("📋 Configuration:");
    println!("   RPC URL: {}", rpc_url);
    println!("   Database: {}", database_url);
    println!("   Program ID: {}", program_id);
    println!("   Program: System Program (Native SOL Transfers)\n");

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
    let handler = SystemTransferHandler;
    let db_pool = sqlx::PgPool::connect(&database_url).await?;
    handler.initialize_schema(&db_pool).await?;
    println!("✅ Database schema ready\n");

    // ============================================================================================
    // Register Decoder
    // ============================================================================================

    println!("🔧 Registering instruction decoder...");

    // Register the System Transfer decoder with the decoder registry
    // The decoder registry matches by program name (e.g., "system", "spl-token")
    // NOT by program ID (the pubkey)
    indexer.decoder_registry_mut().register(
        "system".to_string(), // Program name as it appears in parsed instructions
        Box::new(
            Box::new(SystemTransferDecoder) as Box<dyn InstructionDecoder<SystemTransferEvent>>
        ),
    );

    println!("✅ Decoder registered for 'system' program\n");

    // ============================================================================================
    // Register Handler
    // ============================================================================================

    println!("🔧 Registering event handler...");

    // Wrap the handler in a Box for dynamic dispatch
    let handler_box: Box<dyn EventHandler<SystemTransferEvent>> = Box::new(handler);

    // Register the handler with the indexer
    // The SDK will automatically route SystemTransferEvent instances to this handler
    indexer
        .handler_registry_mut()
        .register(SystemTransferEvent::discriminator(), Box::new(handler_box));

    println!("✅ Handler registered\n");

    // ============================================================================================
    // Start Indexing
    // ============================================================================================

    println!("🔄 Starting indexer...");
    println!("   Monitoring native SOL transfers on Solana");
    println!("   Compatible with spl-transfer-generator");
    println!("   Press Ctrl+C to stop\n");
    println!("{}", "=".repeat(80));

    // Start the indexer - this runs indefinitely until interrupted
    indexer.start().await?;

    Ok(())
}
