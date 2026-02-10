//! Jupiter Swap Indexer Example
//!
//! This example demonstrates the Solana Indexer SDK's "Plug-and-Play" architecture by
//! building a custom indexer for Jupiter v6 swaps. It showcases:
//!
//! 1. **Custom Event Types** - Defining a domain-specific `JupiterSwapEvent`
//! 2. **Custom Instruction Decoding** - Parsing Jupiter v6 program instructions
//! 3. **Custom DB Initialization** - Using `initialize_schema` for automated setup
//! 4. **Rate Limit Handling** - Using high polling intervals for Mainnet/Devnet
//!
//! ## Usage
//!
//! Set environment variables in `.env`:
//! ```env
//! RPC_URL=https://api.mainnet-beta.solana.com  # Or a private RPC
//! DATABASE_URL=postgresql://postgres:password@localhost/solana_indexer
//! ```
//!
//! Run the example:
//! ```bash
//! cargo run --example jupiter_swap_indexer
//! ```

use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_indexer::config::IndexingMode;
use solana_indexer::types::events::ParsedEvent;
use solana_indexer::types::traits::LogDecoder;
use solana_indexer::{
    EventDiscriminator, EventHandler, SolanaIndexerConfigBuilder, SolanaIndexerError,
    calculate_discriminator,
};
use sqlx::PgPool;

// ================================================================================================
// 1. Custom Event Definition
// ================================================================================================

/// Represents a Jupiter v6 Swap event.
///
/// Captured data includes the AMM used, input/output mints, and amounts.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct JupiterSwapEvent {
    pub amm: String,
    pub input_mint: String,
    pub output_mint: String,
    pub input_amount: u64,
    pub output_amount: u64,
}

impl EventDiscriminator for JupiterSwapEvent {
    fn discriminator() -> [u8; 8] {
        calculate_discriminator("JupiterSwapEvent")
    }
}

// ================================================================================================
// 2. Custom Instruction Decoder
// ================================================================================================

pub struct JupiterLogDecoder;

impl LogDecoder<JupiterSwapEvent> for JupiterLogDecoder {
    fn decode(&self, event: &ParsedEvent) -> Option<JupiterSwapEvent> {
        // Note: The SDK's stateless log parser doesn't attach the program ID
        // to "Program log:" lines (it only does for "Program invoke").
        // Thus, we skip the strict ID check inside the decoder for this example,
        // trusting the RPC filter handled the top-level filtering.

        // We are looking for lines that contain specific Jupiter instructions.

        // We are looking for "Program log: Instruction: Route" or similar
        // The event.data contains the log message string
        if let Some(log_message) = &event.data {
            println!("🔍 DEBUG Log: {}", log_message);
            // Simple string matching for demonstration
            if log_message.contains("Instruction: Route")
                || log_message.contains("Instruction: SharedAccountsRoute")
            {
                // In a real indexer, you would correlate this with the inner instructions (transfers)
                // to get exact amounts.
                // Here we return a "Detected" event to prove indexing works.
                return Some(JupiterSwapEvent {
                    amm: "Jupiter v6".to_string(),
                    input_mint: "See Transaction".to_string(), // Requires inner-instruction parsing
                    output_mint: "See Transaction".to_string(),
                    input_amount: 0,
                    output_amount: 0,
                });
            }
        }

        None
    }
}

// ================================================================================================
// 3. Custom Event Handler
// ================================================================================================

/// Handler for Jupiter Swap events.
///
/// Demonstrates automated schema initialization and idempotent data storage.
pub struct JupiterSwapHandler;

#[async_trait]
impl EventHandler<JupiterSwapEvent> for JupiterSwapHandler {
    /// Initializes the `jupiter_swaps` table automatically.
    async fn initialize_schema(&self, db: &PgPool) -> Result<(), SolanaIndexerError> {
        println!("📊 Ensuring jupiter_swaps table exists...");
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS jupiter_swaps (
                signature TEXT PRIMARY KEY,
                amm TEXT NOT NULL,
                input_mint TEXT NOT NULL,
                input_amount BIGINT NOT NULL,
                output_mint TEXT NOT NULL,
                output_amount BIGINT NOT NULL,
                indexed_at TIMESTAMPTZ DEFAULT NOW()
            )",
        )
        .execute(db)
        .await?;
        Ok(())
    }

    async fn handle(
        &self,
        event: JupiterSwapEvent,
        db: &PgPool,
        signature: &str,
    ) -> Result<(), SolanaIndexerError> {
        println!("🔥 Jupiter Swap Indexed! Signature: {}", signature);

        sqlx::query(
            "INSERT INTO jupiter_swaps (signature, amm, input_mint, input_amount, output_mint, output_amount)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (signature) DO NOTHING",
        )
        .bind(signature)
        .bind(event.amm)
        .bind(event.input_mint)
        .bind(event.input_amount as i64)
        .bind(event.output_mint)
        .bind(event.output_amount as i64)
        .execute(db)
        .await?;

        Ok(())
    }
}

// ================================================================================================
// 4. Main Application Entry Point
// ================================================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    println!("🚀 Starting Jupiter Swap Indexer Example...");
    println!("Respecting rate limits with 30s polling intervals.\n");

    let rpc_url = "https://api.mainnet-beta.solana.com".to_string();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set in .env");
    let jupiter_program_id = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";

    // Build configuration
    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(rpc_url)
        .with_database(database_url.clone())
        .program_id(jupiter_program_id)
        .with_poll_interval(35) // 35 second polling to be very safe with rate limits
        .with_batch_size(5) // Small batch size for stability
        .build()?;

    // Switch to LOG indexing mode
    let mut config = config;
    config.indexing_mode = IndexingMode::logs();

    // Initialize indexer
    let mut indexer = solana_indexer::SolanaIndexer::new(config).await?;

    // Register custom components
    let handler = JupiterSwapHandler;
    let db_pool = sqlx::PgPool::connect(&database_url).await?;

    // Explicitly run schema initialization (or let the SDK handle it if registered)
    handler.initialize_schema(&db_pool).await?;

    // Register the decoder for the Jupiter program ID
    // Note: The SDK matches by program name for parsed instructions.
    // For Jupiter v6, the parsed program name is often "jupiter-v6" or the program ID itself.
    indexer.log_decoder_registry_mut().register(
        jupiter_program_id.to_string(),
        Box::new(Box::new(JupiterLogDecoder) as Box<dyn LogDecoder<JupiterSwapEvent>>),
    );

    // Register the handler
    indexer.handler_registry_mut().register(
        JupiterSwapEvent::discriminator(),
        Box::new(Box::new(handler) as Box<dyn EventHandler<JupiterSwapEvent>>),
    );

    println!("✅ Jupiter Indexer configured and ready.");
    println!("Monitoring: {}\n", jupiter_program_id);

    // Start indexing
    indexer.start().await?;

    Ok(())
}
