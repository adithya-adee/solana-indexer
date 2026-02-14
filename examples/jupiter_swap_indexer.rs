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
//! DATABASE_URL=postgresql://postgres:password@localhost/solana_indexer_sdk
//! ```
//!
//! Run the example:
//! ```bash
//! cargo run --example jupiter_swap_indexer
//! ```

use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_indexer_sdk::config::IndexingMode;
use solana_indexer_sdk::types::events::ParsedEvent;
use solana_indexer_sdk::types::traits::LogDecoder;
use solana_indexer_sdk::{
    calculate_discriminator, EventDiscriminator, EventHandler, SolanaIndexerConfigBuilder,
    SolanaIndexerError,
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
    /// Decodes a Jupiter Swap event from program logs.
    ///
    /// Note: Program logs on Solana are unstructured strings. The SDK's log parser
    /// identifies "Program log:" and "Program data:" lines. For high-performance
    /// indexing, using logs can be faster than full transaction fetching if
    /// the program emits Anchor-style events.
    fn decode(&self, event: &ParsedEvent) -> Option<JupiterSwapEvent> {
        // The event.data contains the log message string.
        // In a production environment, you would use regex or a formal parser
        // to extract precise data from these strings.
        if let Some(log_message) = &event.data {
            // Check for Jupiter-specific instruction logs
            if log_message.contains("Instruction: Route")
                || log_message.contains("Instruction: SharedAccountsRoute")
            {
                // Note: Correlating logs with amounts often requires parsing inner-instructions.
                // This example serves as a "detection" indexer.
                return Some(JupiterSwapEvent {
                    amm: "Jupiter v6".to_string(),
                    input_mint: "See Transaction".to_string(),
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
        context: &solana_indexer_sdk::TxMetadata,
        db: &PgPool,
    ) -> Result<(), SolanaIndexerError> {
        let signature = &context.signature;
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
    let database_url = std::env::var("DATABASE_URL")?;
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
    let mut indexer = solana_indexer_sdk::SolanaIndexer::new(config).await?;

    // Register custom components
    let handler = JupiterSwapHandler;
    let db_pool = sqlx::PgPool::connect(&database_url).await?;

    // Explicitly run schema initialization (or let the SDK handle it if registered)
    handler.initialize_schema(&db_pool).await?;

    // Register the decoder for the Jupiter program ID
    // Note: The SDK matches by program name for parsed instructions.
    // For Jupiter v6, the parsed program name is often "jupiter-v6" or the program ID itself.
    indexer.log_decoder_registry_mut()?.register(
        jupiter_program_id.to_string(),
        Box::new(Box::new(JupiterLogDecoder) as Box<dyn LogDecoder<JupiterSwapEvent>>),
    )?;

    // Register the handler
    indexer.handler_registry_mut()?.register(
        JupiterSwapEvent::discriminator(),
        Box::new(Box::new(handler) as Box<dyn EventHandler<JupiterSwapEvent>>),
    )?;

    println!("✅ Jupiter Indexer configured and ready.");
    println!("Monitoring: {}", jupiter_program_id);
    println!("Press Ctrl+C to stop gracefully.\n");

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
