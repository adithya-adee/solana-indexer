//! System Transfer Indexer Example - Full Feature Showcase
//!
//! This example demonstrates a production-ready indexer with multiple features:
//! 1. **Transaction & Account Tracking**: Captures transfer events and tracks accounts involved.
//! 2. **Dynamic Backfill**: Automatically indexes historical data if the indexer falls behind.
//! 3. **Graceful Shutdown**: Uses a standard Ctrl+C handler for safe shutdown.
//! 4. **Advanced Configuration**: Sets worker threads and commitment levels.

use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_indexer_sdk::{
    calculate_discriminator,
    config::{BackfillConfig, CommitmentLevel, StartStrategy},
    init_telemetry, BackfillContext, BackfillHandler, BackfillRange, BackfillTrigger,
    EventDiscriminator, EventHandler, InstructionDecoder, RetryConfig, SolanaIndexer,
    SolanaIndexerConfigBuilder, SolanaIndexerError, StorageBackend, TelemetryConfig, TxMetadata,
};
use solana_sdk::pubkey::Pubkey;
use solana_transaction_status::{UiInstruction, UiParsedInstruction};
use sqlx::PgPool;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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
        if let UiInstruction::Parsed(UiParsedInstruction::Parsed(parsed)) = instruction {
            if parsed.program == "system" && parsed.parsed.get("type")?.as_str()? == "transfer" {
                let info = parsed.parsed.get("info")?.as_object()?;
                return Some(SystemTransferEvent {
                    from: info.get("source")?.as_str()?.parse().ok()?,
                    to: info.get("destination")?.as_str()?.parse().ok()?,
                    amount: info.get("lamports")?.as_u64()?,
                });
            }
        }
        None
    }
}

// ================================================================================================
// Event Handler
// ================================================================================================

pub struct SystemTransferHandler;

#[async_trait]
impl EventHandler<SystemTransferEvent> for SystemTransferHandler {
    /// Initializes the database schema for both transactions and accounts.
    async fn initialize_schema(&self, db: &PgPool) -> Result<(), SolanaIndexerError> {
        // Create table for transactions
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS system_transactions (
                signature TEXT PRIMARY KEY,
                from_wallet TEXT NOT NULL,
                to_wallet TEXT NOT NULL,
                amount_lamports BIGINT NOT NULL,
                indexed_at TIMESTAMPTZ DEFAULT NOW()
            )",
        )
        .execute(db)
        .await?;

        // Create table for backfilled transactions
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS backfill_transactions (
                signature TEXT PRIMARY KEY,
                from_wallet TEXT NOT NULL,
                to_wallet TEXT NOT NULL,
                amount_lamports BIGINT NOT NULL,
                indexed_at TIMESTAMPTZ DEFAULT NOW()
            )",
        )
        .execute(db)
        .await?;

        // Create table to track accounts involved in transfers
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS system_accounts (
                pubkey TEXT PRIMARY KEY,
                last_seen_at TIMESTAMPTZ NOT NULL
            )",
        )
        .execute(db)
        .await?;

        Ok(())
    }

    /// Processes a decoded event, updating both transactions and accounts tables.
    async fn handle(
        &self,
        event: SystemTransferEvent,
        context: &TxMetadata,
        db: &PgPool,
    ) -> Result<(), SolanaIndexerError> {
        let signature = &context.signature;
        let from_wallet = event.from.to_string();
        let to_wallet = event.to.to_string();

        let mut tx = db.begin().await?;

        // Insert the transaction
        sqlx::query(
            "INSERT INTO system_transactions (signature, from_wallet, to_wallet, amount_lamports)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (signature) DO NOTHING",
        )
        .bind(signature)
        .bind(&from_wallet)
        .bind(&to_wallet)
        .bind(event.amount as i64)
        .execute(&mut *tx)
        .await?;

        // Update the accounts involved
        sqlx::query(
            "INSERT INTO system_accounts (pubkey, last_seen_at)
             VALUES ($1, NOW())
             ON CONFLICT (pubkey) DO UPDATE SET last_seen_at = NOW()",
        )
        .bind(&from_wallet)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO system_accounts (pubkey, last_seen_at)
             VALUES ($1, NOW())
             ON CONFLICT (pubkey) DO UPDATE SET last_seen_at = NOW()",
        )
        .bind(&to_wallet)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(())
    }
}

#[async_trait]
impl BackfillHandler<SystemTransferEvent> for SystemTransferHandler {
    async fn handle_backfill(
        &self,
        event: SystemTransferEvent,
        context: &TxMetadata,
        db: &PgPool,
    ) -> Result<(), SolanaIndexerError> {
        let signature = &context.signature;
        let from_wallet = event.from.to_string();
        let to_wallet = event.to.to_string();

        let mut tx = db.begin().await?;

        // Insert the transaction into the dedicated backfill table
        sqlx::query(
            "INSERT INTO backfill_transactions (signature, from_wallet, to_wallet, amount_lamports)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (signature) DO NOTHING",
        )
        .bind(signature)
        .bind(&from_wallet)
        .bind(&to_wallet)
        .bind(event.amount as i64)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(())
    }
}

// ================================================================================================
// Custom Backfill Trigger (runs exactly once)
// ================================================================================================

/// Simple backfill trigger that schedules a single range once and then disables itself.
///
/// This is useful in examples or integrations where you just want to verify that
/// backfill works end-to-end without running continuously.
#[derive(Debug, Default)]
pub struct OnceBackfillTrigger {
    has_run: AtomicBool,
}

impl OnceBackfillTrigger {
    pub fn new() -> Self {
        Self {
            has_run: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl BackfillTrigger for OnceBackfillTrigger {
    async fn next_range(
        &self,
        ctx: &BackfillContext,
        _storage: &dyn StorageBackend,
    ) -> solana_indexer_sdk::Result<Option<BackfillRange>> {
        // Ensure we only schedule a single backfill range for the lifetime of this trigger.
        if self.has_run.swap(true, Ordering::SeqCst) {
            return Ok(None);
        }

        // For devnet compatibility, always base the range on the current devnet
        // finalized tip instead of any previously stored progress (which might
        // have come from a different network like localnet).
        let latest = ctx.latest_finalized_slot;
        if latest == 0 {
            return Ok(None);
        }

        // Cap the range size to at most 10_000 slots to avoid huge jobs.
        let max_range: u64 = 10_000;
        let start_slot = latest.saturating_sub(max_range);
        let end_slot = latest;

        Ok(Some(BackfillRange::new(start_slot, end_slot)))
    }
}

// ================================================================================================
// Main Application
// ================================================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    // Enable SDK's native console logging (via tracing)
    let _guard = init_telemetry(TelemetryConfig {
        service_name: "system-transfer-example".to_string(),
        log_filter: "info".to_string(),
        enable_console_colors: true,
        ..Default::default()
    });

    println!("🚀 System Transfer Indexer starting...");

    let rpc_url = "https://api.devnet.solana.com".to_string();
    let database_url = std::env::var("DATABASE_URL")?;
    let program_id = "11111111111111111111111111111111".to_string();

    // Enable dynamic backfill. The indexer will automatically process historical data
    // if it's more than 1000 slots behind the chain tip.
    let backfill_config = BackfillConfig {
        enabled: true,
        poll_interval_secs: 10,
        desired_lag_slots: Some(1000),
        ..Default::default()
    };

    let retry_config = RetryConfig {
        max_retries: 5,
        initial_backoff_ms: 500, // Wait 500ms before first retry
        backoff_multiplier: 2.0, // Exponential backoff (500ms, 1s, 2s...)
        max_backoff_ms: 10_000,  // Cap backoff at 10s
        jitter: true,            // Add random jitter to prevent thundering herd
    };

    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(rpc_url)
        .with_database(database_url.clone())
        .program_id(program_id)
        .with_poll_interval(10)
        .with_batch_size(10)
        .with_worker_threads(5)
        .with_commitment(CommitmentLevel::Confirmed)
        .with_start_strategy(StartStrategy::Latest)
        .with_backfill(backfill_config)
        .with_retry(retry_config)
        .build()?;

    let mut indexer = SolanaIndexer::new(config).await?;

    // Initialize database schema
    let handler = SystemTransferHandler;
    let db_pool = sqlx::PgPool::connect(&database_url).await?;
    handler.initialize_schema(&db_pool).await?;
    println!("✅ Database schema ready");

    // Register decoder, live handler, and backfill handler using high-level APIs so that
    // instruction-input indexing is enabled for both live and backfill processing.
    indexer.register_decoder("system", SystemTransferDecoder)?;
    indexer.register_handler(SystemTransferHandler)?;
    indexer.register_backfill_handler(SystemTransferHandler)?;

    // Install a custom backfill trigger that only schedules a single range once.
    // This keeps the example behavior simple while still exercising the backfill pipeline.
    let once_trigger = Arc::new(OnceBackfillTrigger::new());
    indexer.with_backfill_trigger(once_trigger)?;

    println!("✅ Decoder, handler, backfill handler, and custom trigger registered");

    // Start indexer and wait for Ctrl+C
    println!("▶️  Starting indexer... Press Ctrl+C to shut down.");
    let indexer_handle = tokio::spawn(async move { indexer.start().await });

    if let Err(e) = tokio::signal::ctrl_c().await {
        eprintln!("Error waiting for shutdown signal: {}", e);
    }

    // Wait for the indexer task to complete. The `start` method will automatically
    // handle the cancellation signal and shut down gracefully.
    if let Err(e) = indexer_handle.await {
        eprintln!("Indexer task failed: {:?}", e);
    }

    println!("✅ Indexer shut down.");
    Ok(())
}
