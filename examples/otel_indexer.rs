//! OpenTelemetry Indexer Example
//!
//! Demonstrates how to instrument a Solana indexer with distributed tracing
//! using the `opentelemetry` feature flag.  Every transaction that reaches the
//! handler produces a structured OTel span, which any OTLP-compatible backend
//! (Jaeger, Grafana Tempo, Datadog, Honeycomb …) can store and visualise.
//!
//! # Prerequisites
//!
//! 1. A running OTLP collector.  The quickest option is Jaeger all-in-one:
//!
//!    ```sh
//!    docker run -d --name jaeger \
//!      -p 16686:16686 \   # Jaeger UI
//!      -p 4317:4317   \   # OTLP gRPC
//!      jaegertracing/all-in-one:latest
//!    ```
//!
//! 2. A PostgreSQL instance with `DATABASE_URL` set in your `.env`.
//!
//! 3. Build and run **with the `opentelemetry` feature**:
//!
//!    ```sh
//!    cargo run \
//!      --example otel_indexer \
//!      --features opentelemetry
//!    ```
//!
//! 4. Open `http://localhost:16686` in your browser and search for the
//!    service `"solana-system-transfer-otel"`.
//!
//! # What you will see
//!
//! - A root span called `"handle_transfer"` for every SOL transfer processed.
//! - Structured fields on that span: `tx.signature`, `tx.slot`,
//!   `transfer.from`, `transfer.to`, `transfer.lamports`.
//! - Child spans emitted by the database helper are automatically linked
//!   because they share the same `tracing` span context.

use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_indexer_sdk::{
    calculate_discriminator,
    telemetry::{init_telemetry_with_otel, OtelConfig, OtlpProtocol, TelemetryConfig},
    EventDiscriminator, EventHandler, InstructionDecoder, SolanaIndexer,
    SolanaIndexerConfigBuilder, SolanaIndexerError, TxMetadata,
};
use solana_sdk::pubkey::Pubkey;
use solana_transaction_status::{UiInstruction, UiParsedInstruction};
use sqlx::PgPool;
use tracing::instrument;

// ── Event type ────────────────────────────────────────────────────────────────

/// Represents a native SOL transfer between two wallets.
///
/// We derive `BorshSerialize` / `BorshDeserialize` because the SDK uses Borsh
/// for its internal event discriminator logic.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct SolTransfer {
    pub from: Pubkey,
    pub to: Pubkey,
    /// Transfer amount in lamports (1 SOL = 1_000_000_000 lamports).
    pub lamports: u64,
}

impl EventDiscriminator for SolTransfer {
    fn discriminator() -> [u8; 8] {
        calculate_discriminator("SolTransfer")
    }
}

// ── Decoder ───────────────────────────────────────────────────────────────────

/// Parses the System Program's `transfer` instruction into a typed
/// [`SolTransfer`] event.
///
/// Returns `None` for any instruction that is not a SOL transfer, so the SDK
/// will silently skip it.
pub struct SolTransferDecoder;

impl InstructionDecoder<SolTransfer> for SolTransferDecoder {
    fn decode(&self, instruction: &UiInstruction) -> Option<SolTransfer> {
        // The RPC returns parsed instructions as JSON when the program is known.
        let UiInstruction::Parsed(UiParsedInstruction::Parsed(parsed)) = instruction else {
            return None;
        };

        // Only handle the System Program `transfer` instruction type.
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

// ── Handler (with OTel instrumentation) ──────────────────────────────────────

pub struct SolTransferHandler;

#[async_trait]
impl EventHandler<SolTransfer> for SolTransferHandler {
    /// Create the `sol_transfers` table if it does not already exist.
    async fn initialize_schema(&self, db: &PgPool) -> Result<(), SolanaIndexerError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS sol_transfers (
                signature       TEXT        PRIMARY KEY,
                from_wallet     TEXT        NOT NULL,
                to_wallet       TEXT        NOT NULL,
                lamports        BIGINT      NOT NULL,
                slot            BIGINT      NOT NULL,
                indexed_at      TIMESTAMPTZ DEFAULT NOW()
            )",
        )
        .execute(db)
        .await?;

        Ok(())
    }

    /// Process a decoded SOL transfer event.
    ///
    /// The `#[instrument]` macro opens an OTel span named `"handle_transfer"`
    /// every time this method is called.  All structured fields listed in
    /// `fields(...)` are attached to the span so they appear in your tracing
    /// UI alongside timing and error information.
    ///
    /// Because we are inside an async function, `#[instrument]` automatically
    /// uses `tracing::Span::in_scope` for each `.await` point so the span
    /// context is never lost across task switches.
    #[instrument(
        name = "handle_transfer",
        // Attach key business fields as structured span attributes.
        // These show up as indexed columns in Jaeger / Grafana Tempo.
        fields(
            tx.signature  = %context.signature,
            tx.slot       = context.slot,
            transfer.from = %event.from,
            transfer.to   = %event.to,
            transfer.lamports = event.lamports,
        ),
        // Skip the db pool from auto-formatting — it has no useful Display.
        skip(self, db),
    )]
    async fn handle(
        &self,
        event: SolTransfer,
        context: &TxMetadata,
        db: &PgPool,
    ) -> Result<(), SolanaIndexerError> {
        // The span is already open here.  Any tracing events emitted inside
        // this function (including those from sqlx if you enable its tracing
        // integration) will automatically become children of this span.
        tracing::debug!("inserting transfer into database");

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

        // Emit a structured event that shows up as a log entry *inside* the
        // span — not as a separate top-level trace.
        tracing::info!(lamports = event.lamports, "✅ transfer saved");

        Ok(())
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load DATABASE_URL and OTLP_ENDPOINT from the project .env file.
    dotenvy::dotenv().ok();

    // ── Step 1: Initialise the telemetry pipeline ─────────────────────────────
    //
    // `init_telemetry_with_otel` installs a global tracing subscriber with two
    // layers:
    //   1. A console (fmt) layer  — structured, coloured, human-readable.
    //   2. An OTLP layer          — exports spans to the collector in real time.
    //
    // The returned `TelemetryGuard` is an RAII handle.  Dropping it triggers
    // `shutdown_telemetry`, which flushes the batch exporter so no spans are
    // lost on exit.  Keep `_guard` alive for the entire duration of `main`.
    let otlp_endpoint =
        std::env::var("OTLP_ENDPOINT").unwrap_or_else(|_| "http://localhost:4317".to_string());

    let _guard = init_telemetry_with_otel(TelemetryConfig {
        service_name: "solana-system-transfer-otel".into(),
        log_filter: "info,solana_indexer_sdk=debug".into(),
        enable_console_colors: true,
        show_target: true,
        show_thread_ids: false,
        otel: Some(OtelConfig {
            endpoint: otlp_endpoint.clone(),
            protocol: OtlpProtocol::Grpc,
        }),
    });

    tracing::info!(
        otlp_endpoint = %otlp_endpoint,
        "🔭 telemetry initialised — spans flowing to collector"
    );

    // ── Step 2: Configure the indexer ─────────────────────────────────────────

    let rpc_url = "https://api.devnet.solana.com";
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set (see examples/.env)");

    // System Program — watches every native SOL transfer on devnet.
    let system_program = "11111111111111111111111111111111";

    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(rpc_url)
        .with_database(database_url.clone())
        .program_id(system_program)
        // Poll every 5 seconds; devnet generates a new block every ~400 ms.
        .with_poll_interval(5)
        .with_batch_size(20)
        .with_worker_threads(4)
        .build()?;

    // ── Step 3: Wire up the indexer ───────────────────────────────────────────

    let mut indexer = SolanaIndexer::new(config).await?;

    // Initialise the database schema before starting.
    let db = PgPool::connect(&database_url).await?;
    SolTransferHandler.initialize_schema(&db).await?;

    // Register the decoder (enables instruction-input mode automatically).
    indexer.register_decoder("system", SolTransferDecoder)?;

    // Register the event handler. Every decoded SolTransfer triggers
    // `SolTransferHandler::handle`, which emits an OTel span.
    indexer.register_handler(SolTransferHandler)?;

    tracing::info!("▶️  starting indexer — press Ctrl+C to stop");

    // ── Step 4: Run until Ctrl+C ──────────────────────────────────────────────

    // Drive the indexer in a background task so the main thread can wait for
    // the shutdown signal without blocking the async runtime.
    let indexer_task = tokio::spawn(async move { indexer.start().await });

    tokio::signal::ctrl_c().await?;
    tracing::info!("🛑 shutdown signal received");

    // Wait for the indexer to wind down gracefully.
    if let Err(e) = indexer_task.await {
        tracing::error!(error = ?e, "indexer task panicked");
    }

    // `_guard` is dropped here, which calls `shutdown_telemetry` and flushes
    // any spans still queued in the batch exporter.
    tracing::info!("✅ all spans flushed — goodbye");

    Ok(())
}
