//! Production-grade SOL transfer indexer with OpenTelemetry observability using Helius.
//!
//! Indexes every native SOL transfer routed through the System Program,
//! persists them to PostgreSQL, and emits distributed traces to any
//! OTLP-compatible backend (Jaeger, Grafana Tempo, Datadog, Honeycomb, …).
//! This uses Helius as a high-performance data source, replacing public RPCs.
//!
//! # Quick Start
//!
//! **1. Start Grafana Tempo (OTLP collector + UI)**
//! ```sh
//! cd examples/observability
//! docker-compose up -d
//! ```
//! This will spin up:
//! - **Grafana** at `http://localhost:3000`
//! - **Tempo** tracing backend listening on `4317`
//!
//! **2. Configure environment** (`examples/.env`)
//! ```dotenv
//! DATABASE_URL=postgres://user:pass@localhost:5432/solana_indexer
//! HELIUS_API_KEY=your_helius_api_key
//! OTLP_ENDPOINT=http://localhost:4317
//! ```
//!
//! **3. Run**
//! ```sh
//! cargo run --example otel_helius_indexer --features "opentelemetry helius"
//! ```
//!
//! **4. View traces** at `http://localhost:3000` → Explore → Tempo → Service: `solana-helius-indexer`

use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_indexer_sdk::{
    calculate_discriminator,
    config::{BackfillConfig, HeliusNetwork, IndexingMode},
    telemetry::{init_telemetry_with_otel, OtelConfig, OtlpProtocol, TelemetryConfig},
    EventDiscriminator, EventHandler, InstructionDecoder, SolanaIndexer,
    SolanaIndexerConfigBuilder, SolanaIndexerError, TxMetadata,
};
use solana_sdk::pubkey::Pubkey;
use solana_transaction_status::{UiInstruction, UiParsedInstruction};
use sqlx::PgPool;
use tracing::instrument;

// The Solana System Program handles all native SOL transfers.
const SYSTEM_PROGRAM_ID: &str = "11111111111111111111111111111111";

// ── Event ─────────────────────────────────────────────────────────────────────

/// A decoded native SOL transfer between two wallets.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct SolTransfer {
    pub from: Pubkey,
    pub to: Pubkey,
    /// Amount transferred, in lamports (1 SOL = 1_000_000_000 lamports).
    pub lamports: u64,
}

impl EventDiscriminator for SolTransfer {
    fn discriminator() -> [u8; 8] {
        calculate_discriminator("SolTransfer")
    }
}

// ── Decoder ───────────────────────────────────────────────────────────────────

pub struct SolTransferDecoder;

impl InstructionDecoder<SolTransfer> for SolTransferDecoder {
    fn decode(&self, instruction: &UiInstruction) -> Option<SolTransfer> {
        let UiInstruction::Parsed(UiParsedInstruction::Parsed(parsed)) = instruction else {
            return None;
        };

        if parsed.program != "system" || parsed.parsed.get("type")?.as_str()? != "transfer" {
            return None;
        }

        let info = parsed.parsed.get("info")?.as_object()?;

        Some(SolTransfer {
            from: info.get("source")?.as_str()?.parse().ok()?,
            to: info.get("destination")?.as_str()?.parse().ok()?,
            lamports: info.get("lamports")?.as_u64()?,
        })
    }
}

// ── Handler ───────────────────────────────────────────────────────────────────

pub struct SolTransferHandler;

#[async_trait]
impl EventHandler<SolTransfer> for SolTransferHandler {
    async fn initialize_schema(&self, db: &PgPool) -> Result<(), SolanaIndexerError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS helius_sol_transfers (
                signature    TEXT        PRIMARY KEY,
                from_wallet  TEXT        NOT NULL,
                to_wallet    TEXT        NOT NULL,
                lamports     BIGINT      NOT NULL,
                slot         BIGINT      NOT NULL,
                indexed_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(db)
        .await?;

        // Indexes for common query patterns (analytics, wallet history).
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_helius_transfers_from ON helius_sol_transfers (from_wallet)",
        )
        .execute(db)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_helius_transfers_to ON helius_sol_transfers (to_wallet)"
        )
        .execute(db)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_helius_transfers_slot ON helius_sol_transfers (slot DESC)",
        )
        .execute(db)
        .await?;

        Ok(())
    }

    #[instrument(
        name = "handle_helius_transfer",
        fields(
            tx.signature      = %context.signature,
            tx.slot           = context.slot,
            transfer.from     = %event.from,
            transfer.to       = %event.to,
            transfer.lamports = event.lamports,
        ),
        skip(self, db),
    )]
    async fn handle(
        &self,
        event: SolTransfer,
        context: &TxMetadata,
        db: &PgPool,
    ) -> Result<(), SolanaIndexerError> {
        sqlx::query(
            "INSERT INTO helius_sol_transfers (signature, from_wallet, to_wallet, lamports, slot)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (signature) DO NOTHING",
        )
        .bind(&context.signature)
        .bind(event.from.to_string())
        .bind(event.to.to_string())
        .bind(event.lamports as i64)
        .bind(context.slot as i64)
        .execute(db)
        .await?;

        tracing::info!(
            lamports = event.lamports,
            sol = event.lamports as f64 / 1_000_000_000.0,
            "transfer indexed"
        );

        Ok(())
    }
}

#[async_trait]
impl solana_indexer_sdk::types::backfill_traits::BackfillHandler<SolTransfer>
    for SolTransferHandler
{
    #[instrument(
        name = "handle_backfill_helius_transfer",
        fields(
            tx.signature      = %context.signature,
            tx.slot           = context.slot,
            transfer.from     = %event.from,
            transfer.to       = %event.to,
            transfer.lamports = event.lamports,
            backfill          = true,
        ),
        skip(self, db),
    )]
    async fn handle_backfill(
        &self,
        event: SolTransfer,
        context: &TxMetadata,
        db: &PgPool,
    ) -> Result<(), SolanaIndexerError> {
        self.handle(event, context, db).await
    }
}

// ── Backfill Trigger ──────────────────────────────────────────────────────────

pub struct CustomBackfillTrigger {
    pub start_slot: u64,
    pub end_slot: u64,
}

#[async_trait]
impl solana_indexer_sdk::types::backfill_traits::BackfillTrigger for CustomBackfillTrigger {
    async fn next_range(
        &self,
        ctx: &solana_indexer_sdk::types::backfill_traits::BackfillContext,
        _storage: &dyn solana_indexer_sdk::storage::StorageBackend,
    ) -> Result<Option<solana_indexer_sdk::types::backfill_traits::BackfillRange>, SolanaIndexerError>
    {
        if let Some(last) = ctx.last_backfilled_slot {
            if last >= self.end_slot {
                return Ok(None);
            }
        }

        let mut actual_start = self.start_slot;
        if let Some(last) = ctx.last_backfilled_slot {
            if last >= self.start_slot && last < self.end_slot {
                actual_start = last + 1;
            }
        }

        if actual_start > self.end_slot {
            return Ok(None);
        }

        let mut actual_end = self.end_slot;
        let chunk_size = 10_000;
        if actual_end - actual_start > chunk_size {
            actual_end = actual_start + chunk_size;
        }

        Ok(Some(
            solana_indexer_sdk::types::backfill_traits::BackfillRange::new(
                actual_start,
                actual_end,
            ),
        ))
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    // ── Telemetry ─────────────────────────────────────────────────────────────

    let otel_config = std::env::var("OTLP_ENDPOINT")
        .ok()
        .map(|endpoint| OtelConfig {
            endpoint,
            protocol: OtlpProtocol::Grpc,
        });

    let _guard = init_telemetry_with_otel(TelemetryConfig {
        service_name: "solana-helius-indexer".into(),
        log_filter: "warn,solana_indexer_sdk=info,otel_helius_indexer=debug".into(),
        enable_console_colors: true,
        show_target: false,
        show_thread_ids: false,
        otel: otel_config,
    });

    if let Ok(endpoint) = std::env::var("OTLP_ENDPOINT") {
        tracing::info!(otlp_endpoint = %endpoint, "telemetry initialised with otlp");
    } else {
        tracing::info!("telemetry initialised (console only)");
    }

    // ── Configuration ─────────────────────────────────────────────────────────

    let api_key = std::env::var("HELIUS_API_KEY").expect("HELIUS_API_KEY must be set");
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    // We use mainnet by default.
    let network = HeliusNetwork::Mainnet;

    // Discover a safe, recent block to begin historical backfill
    let rpc_url = format!("https://mainnet.helius-rpc.com/?api-key={}", api_key);
    let rpc_client = solana_client::rpc_client::RpcClient::new(&rpc_url);

    // Fall back to a known recent mainnet slot if getting the current slot fails
    let latest_slot = rpc_client.get_slot().unwrap_or(320_000_000);
    // Start backfill a few hundred slots behind latest
    let backfill_start = latest_slot.saturating_sub(500);

    let config = SolanaIndexerConfigBuilder::new()
        // `true` for websockets real-time updates through Helius
        .with_helius_network(api_key, network, true)
        .with_database(&database_url)
        .program_id(SYSTEM_PROGRAM_ID)
        .with_indexing_mode(IndexingMode::inputs())
        // Backfill configuration through Helius RPC. We can use higher concurrency
        // because Helius limits are more permissive.
        .with_backfill(BackfillConfig {
            enabled: true,
            batch_size: 100,
            concurrency: 5,
            start_slot: Some(backfill_start),
            end_slot: Some(latest_slot),
            poll_interval_secs: 1,
            ..Default::default()
        })
        .build()?;

    // ── Wiring ────────────────────────────────────────────────────────────────

    tracing::info!(
        program = SYSTEM_PROGRAM_ID,
        network = "Mainnet",
        "configuring helius indexer"
    );

    let mut indexer = SolanaIndexer::new(config).await?;

    // Ensure schema exists before any data arrives.
    let db = PgPool::connect(&database_url).await?;
    SolTransferHandler.initialize_schema(&db).await?;
    tracing::info!("schema ready");

    indexer.register_decoder("system", SolTransferDecoder)?;
    indexer.register_handler(SolTransferHandler)?;
    indexer.register_backfill_handler(SolTransferHandler)?;
    indexer.with_backfill_trigger(std::sync::Arc::new(CustomBackfillTrigger {
        start_slot: backfill_start,
        end_slot: latest_slot,
    }))?;

    // ── Run ───────────────────────────────────────────────────────────────────

    tracing::info!("indexer started — press Ctrl+C to stop");

    let indexer_task = tokio::spawn(async move { indexer.start().await });

    tokio::signal::ctrl_c().await?;
    tracing::info!("shutdown signal received, flushing spans…");

    if let Err(e) = indexer_task.await {
        tracing::error!(error = ?e, "indexer task panicked");
    }

    tracing::info!("shutdown complete");

    Ok(())
}
