//! Production-grade SOL transfer indexer with OpenTelemetry observability.
//!
//! Indexes every native SOL transfer routed through the System Program,
//! persists them to PostgreSQL, and emits distributed traces to any
//! OTLP-compatible backend (Jaeger, Grafana Tempo, Datadog, Honeycomb, …).
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
//! RPC_URL=https://api.devnet.solana.com      # use a private endpoint in production
//! OTLP_ENDPOINT=http://localhost:4317
//! ```
//!
//! **3. Run**
//! ```sh
//! cargo run --example otel_indexer --features opentelemetry
//! ```
//!
//! **4. View traces** at `http://localhost:3000` → Explore → Tempo → Service: `solana-sol-transfer-indexer`
//!
//! # Architecture
//!
//! ```text
//! Devnet RPC ──poll──► SolTransferDecoder ──► SolTransferHandler ──► PostgreSQL
//!                │                                   │
//!           BackfillManager                    OTel Span (handle_transfer)
//!          (fills history)                          │
//!                                           OTLP gRPC exporter ──► Tempo
//!                                                                    │
//!                                                                 Grafana
//! ```
//!
//! # Rate Limit Notes
//!
//! Public Solana RPC nodes enforce ~10 req/s. This example is tuned conservatively:
//! - **Poll interval**: 5 s (12 new-block polls/min)
//! - **Batch size**: 20 signatures per poll
//! - **Backfill concurrency**: 3 parallel RPC calls
//!
//! For production throughput, replace `RPC_URL` with a private Helius /
//! QuickNode / Alchemy endpoint that allows higher request rates.

use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_indexer_sdk::{
    calculate_discriminator,
    config::{BackfillConfig, IndexingMode},
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

/// Decodes System Program `transfer` instructions into [`SolTransfer`] events.
///
/// The RPC returns instructions in JSON-Parsed format when the program is
/// registered. Any non-transfer instruction yields `None` and is silently
/// ignored by the SDK.
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

/// Persists decoded SOL transfers to PostgreSQL.
///
/// Each invocation opens an OpenTelemetry span (`handle_transfer`) containing
/// structured fields that are indexable in Jaeger, Grafana Tempo, or any
/// other OTLP backend.
pub struct SolTransferHandler;

#[async_trait]
impl EventHandler<SolTransfer> for SolTransferHandler {
    /// Creates the `sol_transfers` table and supporting indexes on first run.
    async fn initialize_schema(&self, db: &PgPool) -> Result<(), SolanaIndexerError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS sol_transfers (
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
            "CREATE INDEX IF NOT EXISTS idx_sol_transfers_from ON sol_transfers (from_wallet)",
        )
        .execute(db)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_sol_transfers_to ON sol_transfers (to_wallet)")
            .execute(db)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_sol_transfers_slot ON sol_transfers (slot DESC)",
        )
        .execute(db)
        .await?;

        Ok(())
    }

    /// Persists a transfer and records a structured OTel span.
    ///
    /// The `#[instrument]` macro attaches the listed fields to the active span
    /// so every trace entry in Jaeger carries the full transfer context.
    /// `ON CONFLICT DO NOTHING` guarantees idempotency across restarts and
    /// backfill re-runs.
    #[instrument(
        name = "handle_transfer",
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
            "INSERT INTO sol_transfers (signature, from_wallet, to_wallet, lamports, slot)
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
        name = "handle_backfill_transfer",
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
        // Backfill and real-time events share the exact same database insertion logic
        self.handle(event, context, db).await
    }
}

// ── Backfill Trigger ──────────────────────────────────────────────────────────

/// A custom trigger that ensures we only backfill exactly what we asked for,
/// even across restarts, and prevents the indexer from looping infinitely
/// over the same range once it is finished.
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
        // If the database has already progressed to or past our end_slot, we are done.
        // Returning None stops the backfiller from fetching this range again.
        if let Some(last) = ctx.last_backfilled_slot {
            if last >= self.end_slot {
                return Ok(None);
            }
        }

        // Determine actual start slot.
        // We prioritize our requested start_slot, but if the database has explicitly
        // made some progress WITHIN our requested range, we resume from there.
        let mut actual_start = self.start_slot;
        if let Some(last) = ctx.last_backfilled_slot {
            if last >= self.start_slot && last < self.end_slot {
                actual_start = last + 1;
            }
        }

        if actual_start > self.end_slot {
            return Ok(None);
        }

        // Chunk sizes to avoid overloading memory and RPC
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
    // Load .env before reading any environment variables.
    dotenvy::dotenv().ok();

    // ── Telemetry ─────────────────────────────────────────────────────────────
    //
    // Installs a dual-layer tracing subscriber:
    //   • fmt  — coloured, human-readable console output
    //   • OTLP — batched gRPC export to the configured collector
    //
    // The `TelemetryGuard` flushes the batch exporter on drop, ensuring no
    // spans are lost during graceful shutdown.

    let otel_config = std::env::var("OTLP_ENDPOINT")
        .ok()
        .map(|endpoint| OtelConfig {
            endpoint,
            protocol: OtlpProtocol::Grpc,
        });

    let _guard = init_telemetry_with_otel(TelemetryConfig {
        service_name: "solana-sol-transfer-indexer".into(),
        log_filter: "warn,solana_indexer_sdk=info,otel_indexer=debug".into(),
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

    let rpc_url =
        std::env::var("RPC_URL").unwrap_or_else(|_| "https://api.devnet.solana.com".to_string());
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    // Discover a safe, recent block to begin historical backfill
    // (Public RPCs drop genesis blocks, so slot 0 backfills will fail).
    let rpc_client = solana_client::rpc_client::RpcClient::new(&rpc_url);
    let latest_slot = rpc_client.get_slot().unwrap_or(443_000_000);
    let backfill_start = latest_slot.saturating_sub(100);

    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(&rpc_url)
        .with_database(&database_url)
        // Index only System Program transactions (native SOL transfers).
        .program_id(SYSTEM_PROGRAM_ID)
        // Decode instruction inputs only; logs and account diffing are not needed here.
        .with_indexing_mode(IndexingMode::inputs())
        // Real-time polling: conservative interval to stay within public RPC rate limits.
        .with_poll_interval(5)
        .with_batch_size(20)
        .with_worker_threads(4)
        // Backfill: concurrently fills historical gaps, capped at 3 parallel
        // requests to avoid hitting the 10 req/s limit on public RPC nodes.
        .with_backfill(BackfillConfig {
            enabled: true,
            batch_size: 50,
            concurrency: 3,
            start_slot: Some(backfill_start),
            end_slot: Some(latest_slot),
            poll_interval_secs: 2,
            ..Default::default()
        })
        .build()?;

    // ── Wiring ────────────────────────────────────────────────────────────────

    tracing::info!(rpc_url = %rpc_url, program = SYSTEM_PROGRAM_ID, "configuring indexer");

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

    // _guard drops here → shutdown_telemetry() flushes the OTLP batch exporter.
    tracing::info!("shutdown complete");

    Ok(())
}
