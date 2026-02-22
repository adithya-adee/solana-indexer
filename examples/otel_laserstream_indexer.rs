//! OpenTelemetry + Laserstream Indexer Example
//!
//! Demonstrates how to build an indexer that receives real-time transaction
//! data via Laserstream (Yellowstone gRPC) and instruments the processing
//! pipeline with OpenTelemetry tracing.
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
//! 3. Yellowstone gRPC / Laserstream credentials. Add to your `.env`:
//!    ```dotenv
//!    LASERSTREAM_GRPC_URL=https://grpc.location.tld
//!    LASERSTREAM_X_TOKEN=your_auth_token_here
//!    ```
//!
//! 4. Build and run **with both features enabled**:
//!
//!    ```sh
//!    cargo run \
//!      --example otel_laserstream_indexer \
//!      --features "opentelemetry laserstream"
//!    ```

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

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct SolTransfer {
    pub from: Pubkey,
    pub to: Pubkey,
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

// ── Handler (with OTel instrumentation) ──────────────────────────────────────

pub struct SolTransferHandler;

#[async_trait]
impl EventHandler<SolTransfer> for SolTransferHandler {
    async fn initialize_schema(&self, db: &PgPool) -> Result<(), SolanaIndexerError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS sol_transfers_ls (
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

    #[instrument(
        name = "handle_laserstream_transfer",
        fields(
            tx.signature  = %context.signature,
            tx.slot       = context.slot,
            transfer.from = %event.from,
            transfer.to   = %event.to,
            transfer.lamports = event.lamports,
            source = "laserstream"
        ),
        skip(self, db),
    )]
    async fn handle(
        &self,
        event: SolTransfer,
        context: &TxMetadata,
        db: &PgPool,
    ) -> Result<(), SolanaIndexerError> {
        tracing::debug!("inserting transfer into database (Laserstream)");

        sqlx::query(
            "INSERT INTO sol_transfers_ls (signature, from_wallet, to_wallet, lamports, slot)
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

        tracing::info!(lamports = event.lamports, "✅ laserstream transfer saved");

        Ok(())
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    // Initialise OpenTelemetry
    let otlp_endpoint =
        std::env::var("OTLP_ENDPOINT").unwrap_or_else(|_| "http://localhost:4317".to_string());

    let _guard = init_telemetry_with_otel(TelemetryConfig {
        service_name: "solana-laserstream-otel".into(),
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

    // Laserstream configuration
    let grpc_url = std::env::var("LASERSTREAM_GRPC_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:10000".to_string());
    let x_token = std::env::var("LASERSTREAM_X_TOKEN").ok();

    // Database URL and standard RPC config
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set (see examples/.env)");
    let rpc_url = "https://api.devnet.solana.com";

    // Watch the System Program
    let system_program = "11111111111111111111111111111111";

    let config = SolanaIndexerConfigBuilder::new()
        // Standard RPC for transaction gap filling + startup state
        .with_rpc(rpc_url)
        // Configure Laserstream (Yellowstone) gRPC as the primary realtime feed
        .with_laserstream(grpc_url, x_token)
        .with_database(database_url.clone())
        .program_id(system_program)
        .with_worker_threads(4)
        .build()?;

    let mut indexer = SolanaIndexer::new(config).await?;

    let db = PgPool::connect(&database_url).await?;
    SolTransferHandler.initialize_schema(&db).await?;

    indexer.register_decoder("system", SolTransferDecoder)?;
    indexer.register_handler(SolTransferHandler)?;

    tracing::info!("▶️  starting laserstream indexer — press Ctrl+C to stop");

    let indexer_task = tokio::spawn(async move { indexer.start().await });

    tokio::signal::ctrl_c().await?;
    tracing::info!("🛑 shutdown signal received");

    if let Err(e) = indexer_task.await {
        tracing::error!(error = ?e, "indexer task panicked");
    }

    tracing::info!("✅ all spans flushed — goodbye");

    Ok(())
}
