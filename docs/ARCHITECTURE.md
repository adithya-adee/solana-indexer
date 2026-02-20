# Solana Indexer SDK — Architecture

**Build Philosophy:** `solana-indexer-sdk` is a developer-centric, production-grade Rust SDK for Solana data indexing. Our philosophy is to provide a highly extensible, reliable, and performant platform that prioritizes a seamless developer experience (DX) while offering a clear path from local development to production-grade deployments. We solve the "boring problems" (polling loops, RPC connections, signature fetching, idempotency, reorg handling, backfill) so developers can focus on their unique business logic.

---

## 1. Core System Architecture: Flexible Event-Driven Pipeline

SolanaIndexer operates on a flexible event-driven pipeline model with **three key extension points** for developers:

1. **`InstructionDecoder<T>`**: Custom instruction parsing (raw bytes → typed events)
2. **`LogDecoder<T>`**: Custom log parsing (program logs → typed events)
3. **`EventHandler<T>`**: Custom event processing (typed events → business logic)

This separation of concerns allows developers to build indexers for any use case — from general SPL token transfers to complex custom program logic like Jupiter swaps and Raydium AMM pools — while the SDK handles all infrastructure concerns.

### Data Flow Overview

```
┌──────────────────────────────────────────────────────────────────┐
│                      Input Sources                               │
│  ┌──────────┐   ┌──────────────┐   ┌──────────────────────────┐ │
│  │  Poller  │   │  WebSocket   │   │  Hybrid (WS + Poller)    │ │
│  │  (RPC)   │   │  Subscriber  │   │  Gap-filling backstop    │ │
│  └────┬─────┘   └──────┬───────┘   └───────────┬──────────────┘ │
│       │                │                        │                │
│       │                │                        │                │
│       │                │          ┌─────────────┴─────────────┐  │
│       │                │          │        Laserstream        │  │
│       │                │          │      (Yellowstone gRPC)   │  │
│       │                │          └─────────────┬─────────────┘  │
│       │                │                        │                │
└───────┴────────────────┴────────────────────────┴────────────────┘
                         │                                          
                         ▼                                          
              ┌──────────────────────┐                              
              │   Parallel Fetcher   │  (Concurrent tx retrieval)   
              │   + Backfill Engine  │  (Historical data indexing)  
              └──────────┬───────────┘                              
                         │                                          
                         ▼                                          
              ┌──────────────────────┐                              
              │  Idempotency Check   │  (_processed / _tentative)   
              └──────────┬───────────┘                              
                         │                                          
              ┌──────────┴───────────┐                              
              │                      │                              
              ▼                      ▼                              
  ┌───────────────────┐  ┌───────────────────┐                     
  │ Decoder Registry  │  │ Log Decoder Reg.  │                     
  │ (Instructions)    │  │ (Program Logs)    │                     
  └─────────┬─────────┘  └─────────┬─────────┘                     
            │                      │                                
            └──────────┬───────────┘                                
                       ▼                                            
            ┌──────────────────────┐                                
            │  Handler Registry    │  (Business logic dispatch)     
            └──────────┬───────────┘                                
                       │                                            
                       ▼                                            
            ┌──────────────────────┐                                
            │  Confirmation &      │                                
            │  Persistence         │                                
            └──────────────────────┘                                
```

### Pipeline Stages

1. **Input Source (Poller / Subscriber / Hybrid):** Acquires transaction signatures.
   - **Poller:** Periodically queries Solana RPC for new transaction signatures. Ideal for localnet and moderate throughput.
   - **WebSocket Subscriber:** Real-time notifications via `programSubscribe`. Essential for low-latency production environments.
   - **Hybrid (Dual-Stream):** Combines WebSocket speed with RPC polling reliability. Uses WS for real-time events and a background poller for gap-filling.
   - **Helius Enhanced RPC:** Integration with Helius RPC endpoints for enhanced reliability, historical data, and optimized polling.
   - **Laserstream (Yellowstone gRPC):** High-throughput, low-latency streaming of transactions and blocks directly from validator Geyser plugins. Ideal for heavy backfill and real-time ingestion.
2. **Parallel Fetcher:** Retrieves full transaction details concurrently using a bounded worker pool (`tokio::spawn` + semaphore).
3. **Backfill Engine:** Manages historical data indexing with configurable depth, batch sizes, and slot-based tracking.
4. **Idempotency Tracker:** Checks `_solana_indexer_sdk_processed` and `_solana_indexer_sdk_tentative` tables to prevent re-processing.
5. **Decoder Registry (Extension Point #1):** Routes instructions to registered `InstructionDecoder<T>` implementations.
6. **Log Decoder Registry (Extension Point #2):** Routes program logs to registered `LogDecoder<T>` implementations.
7. **Handler Registry (Extension Point #3):** Dispatches decoded events to registered `EventHandler<T>` implementations.
8. **Confirmation & Persistence:** Marks the transaction as processed in the idempotency tracker.

**Key Insight:** Developers only implement the extension points. The SDK handles everything else.

---

## 2. Configuration Management

### `SolanaIndexerConfig` and Builder Pattern

Configuration is managed via a `SolanaIndexerConfig` struct, built using a fluent builder pattern:

```rust
let config = SolanaIndexerConfigBuilder::new()
    .with_rpc("http://127.0.0.1:8899")       // or .with_ws() or .with_hybrid()
    .with_database(&env::var("DATABASE_URL")?)
    .program_id(&env::var("PROGRAM_ID")?)
    .with_batch_size(50)
    .with_poll_interval(5)
    .with_backfill(BackfillConfig {
        enabled: true,
        max_depth: 1000,
        batch_size: 100,
    })
    .build()?;

let indexer = SolanaIndexer::new(config).await?;
indexer.start().await?;
```

**Features:**
- **Type Safety:** All configuration parameters are strongly typed.
- **Discoverability:** IDE auto-completion guides developers through available options.
- **Environment Variable Integration:** Seamlessly integrates with `std::env::var` and `.env` files.
- **Helius Support:** `.with_helius(api_key, network)` for Helius-enhanced RPC.

---

## 3. Indexer Types and Selection

| Mode | Method | Use Case | Activation |
|:---|:---|:---|:---|
| **RPC Polling** | Periodic `getSignaturesForAddress` | Local dev, moderate throughput | `.with_rpc(...)` |
| **WebSocket** | Real-time `programSubscribe` | Low-latency production | `.with_ws(...)` |
| **Hybrid** | WS + background RPC gap-filling | Production (speed + completeness) | `.with_hybrid(...)` |
| **Helius Enhanced** | Helius RPC APIs | Enhanced reliability + historical data | `.with_helius(...)` |
| **Laserstream** | Yellowstone gRPC | High throughput & backfill | `.with_laserstream(...)` |

---

## 4. IDL Processing & Type Generation

- **IDL-Driven Development:** Place `idl.json` files in the `idl/` directory.
- **Procedural Macro Compilation:** During `cargo build`, a proc macro generates:
  - Rust `struct` definitions for accounts and events.
  - `BorshDeserialize` trait implementations.
  - Event discriminator constants.
- **General Indexing Without IDL:** The SDK can parse common instruction types (e.g., SPL Token transfers) by directly parsing known byte layouts.

---

## 5. Performance & Reliability

### Benchmark Results (v0.1.0 Baseline)

| Component | Metric | Value | Throughput |
|:---|:---|:---|:---|
| **Decoder** | Single Instruction | `62 ns` | **~16M ops/sec** |
| **Decoder** | Batch (100 instr) | `6.4 µs` | **~157K batches/sec** |
| **Storage** | Write (`mark_processed`) | `2.8 ms` | ~357 ops/sec |
| **Storage** | Read (`is_processed`) | `92 µs` | ~10.8K ops/sec |
| **Pipeline** | End-to-End (50 tx batch) | `101 ms` | **~494 TPS** |

Benchmarks use the `criterion` library for statistical rigor. Run with:
```bash
cargo bench
# View report: target/criterion/report/index.html
```

### Parallel Fetch Pipeline
- **Worker Pool:** Configurable concurrent workers via `tokio::spawn`.
- **Semaphore Bounding:** Prevents overwhelming RPC providers.
- **Non-Blocking:** Fully async/await architecture.

### Reorg Detection & Finality Management
- **Commitment Levels:** Configurable (`Processed`, `Confirmed`, `Finalized`).
- **Tentative Transaction Tracking:** Transactions initially stored in `_solana_indexer_sdk_tentative`.
- **FinalityMonitor:** Background task that:
  - Fetches latest finalized slot.
  - Compares block hashes for tentative transactions.
  - Detects reorgs and invokes `on_rollback()` handlers.
  - Promotes confirmed transactions to finalized status.
- **Graceful Shutdown:** Supports cancellation tokens for clean teardown.

### Backfill Engine
- **Historical Indexing:** Automatically backfills missed transactions.
- **Configurable Depth:** Control how far back to search.
- **Slot-Based Tracking:** Efficient batch processing by slot ranges.

---

## 6. Extensibility: Developer Extension Points

### Extension Point #1: `InstructionDecoder<T>`

```rust
pub trait InstructionDecoder<T>: Send + Sync {
    fn decode(&self, instruction: &UiInstruction) -> Option<T>;
}
```

### Extension Point #2: `LogDecoder<T>` (Log-Based Indexing)

```rust
pub trait LogDecoder<T>: Send + Sync {
    fn decode_log(&self, log_line: &str) -> Option<T>;
}
```

### Extension Point #3: `EventHandler<T>`

```rust
#[async_trait]
pub trait EventHandler<T>: Send + Sync + 'static {
    async fn initialize_schema(&self, _db: &PgPool) -> Result<()> { Ok(()) }
    async fn handle(&self, event: T, context: &TxMetadata, db: &PgPool) -> Result<()>;
    async fn on_rollback(&self, _context: &TxMetadata, _db: &PgPool) -> Result<()> { Ok(()) }
}
```

### How They Work Together

```
Transaction → InstructionDecoder<T> → Option<T> ─┐
                                                   ├→ EventHandler<T> → Database/API
Program Logs → LogDecoder<T> → Option<T> ─────────┘
```

---

## 7. Reliability & Error Handling

- **Idempotency:** `_solana_indexer_sdk_processed` table prevents duplicate processing.
- **Structured Errors:** `SolanaIndexerError` enum with `thiserror` provides clear, actionable errors:
  - `DatabaseError`, `DecodingError`, `RpcError`, `ConfigError`, `WebSocketError`
- **Contextual Logging:** Built-in structured logging via `tracing` crate.
- **Graceful Shutdown:** All async tasks honor cancellation tokens.
- **Database Transactions:** Handlers can wrap operations in DB transactions for atomicity.

---

## 8. Security Considerations

- **No Hardcoded Secrets:** Environment variable-driven configuration.
- **Input Validation:** IDL-driven decoding provides implicit validation.
- **SQL Injection Prevention:** `sqlx` with parameterized queries throughout.
- **Minimal Privileges:** Designed for least-privilege operation.
- **Open Source Auditability:** Full codebase transparency.

---

## 9. Directory Structure

```
solana-indexer/
├── Cargo.toml                          # Workspace definition
├── docs/
│   └── ARCHITECTURE.md                 # This file
│
├── solana-indexer-sdk/                  # Core SDK crate
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs                      # Public API and re-exports
│   │   ├── config/
│   │   │   └── mod.rs                  # SolanaIndexerConfig + Builder
│   │   ├── core/
│   │   │   ├── indexer.rs              # Main orchestrator (start, process_*)
│   │   │   ├── fetcher.rs              # Parallel transaction fetching
│   │   │   ├── decoder.rs              # IDL-driven + generic data parsing
│   │   │   ├── registry.rs             # Decoder registry
│   │   │   ├── registry_metrics.rs     # Registry performance metrics
│   │   │   ├── log_registry.rs         # Log decoder registry
│   │   │   ├── account_registry.rs     # Account decoder registry
│   │   │   ├── backfill.rs             # Historical backfill engine
│   │   │   ├── backfill_defaults.rs    # Backfill default configurations
│   │   │   └── reorg.rs               # Finality monitoring & reorg detection
│   │   ├── streams/
│   │   │   ├── poller.rs               # RPC polling implementation
│   │   │   ├── websocket.rs            # WebSocket subscription
│   │   │   ├── hybrid.rs              # Dual-stream (WS + RPC)
│   │   │   └── helius.rs              # Helius-enhanced RPC
│   │   ├── storage/
│   │   │   └── mod.rs                  # PostgreSQL persistence + idempotency
│   │   ├── types/
│   │   │   ├── events.rs               # Event structures + discriminators
│   │   │   └── traits.rs              # EventHandler, InstructionDecoder, etc.
│   │   └── utils/
│   │       ├── error.rs                # SolanaIndexerError enum
│   │       ├── logging.rs              # Structured logging
│   │       └── macros.rs              # Procedural macro for IDL compilation
│   └── tests/                          # Integration tests
│       ├── handler_integration_test.rs
│       ├── multi_program_test.rs
│       └── rpc_integration_test.rs
│
├── benches/                            # Performance benchmarks (criterion)
│   ├── Cargo.toml
│   ├── BENCHMARK_HISTORY.md
│   ├── decoder_bench.rs                # Decoder throughput
│   ├── storage_bench.rs                # Database read/write latency
│   └── throughput_bench.rs             # End-to-end pipeline throughput
│
└── examples/                           # Ready-to-run examples
    ├── rpc_system_transfer.rs          # RPC-based System Transfer indexer
    ├── rpc_spl_token.rs                # RPC-based SPL Token indexer
    ├── ws_system_transfer.rs           # WebSocket-based indexer
    ├── helius_system_transfer.rs       # Helius-enhanced indexer
    ├── jupiter_swap_indexer.rs         # Jupiter DEX swap tracking
    ├── raydium_indexer.rs              # Raydium AMM indexer
    ├── multi_program_indexer.rs        # Multi-program indexing
    ├── account_indexer.rs              # Account state indexing
    ├── backfill_indexer.rs             # Historical backfill
    ├── verify_shutdown.rs              # Graceful shutdown verification
    └── generator_spl_transfer.rs       # Test transaction generator
```

---

## 10. Developer Quickstart

### Step 1: Add Dependency
```toml
[dependencies]
solana-indexer-sdk = "0.2"
```

### Step 2: Define Event + Decoder + Handler

```rust
use solana_indexer_sdk::*;

#[derive(Debug, Clone)]
pub struct TransferEvent {
    pub from: String,
    pub to: String,
    pub amount: u64,
}

pub struct MyDecoder;
impl InstructionDecoder<TransferEvent> for MyDecoder {
    fn decode(&self, instruction: &UiInstruction) -> Option<TransferEvent> {
        // Parse instruction data
        todo!()
    }
}

pub struct MyHandler;
#[async_trait]
impl EventHandler<TransferEvent> for MyHandler {
    async fn handle(&self, event: TransferEvent, ctx: &TxMetadata, db: &PgPool) -> Result<()> {
        sqlx::query("INSERT INTO transfers (sig, from_addr, to_addr, amount) VALUES ($1,$2,$3,$4)")
            .bind(&ctx.signature)
            .bind(&event.from)
            .bind(&event.to)
            .bind(event.amount as i64)
            .execute(db).await?;
        Ok(())
    }
}
```

### Step 3: Run

```rust
#[tokio::main]
async fn main() -> Result<()> {
    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc("http://127.0.0.1:8899")
        .with_database("postgresql://user:pass@localhost/mydb")
        .program_id("11111111111111111111111111111111")
        .build()?;

    let mut indexer = SolanaIndexer::new(config).await?;
    indexer.register_decoder("system", MyDecoder)?;
    indexer.register_handler(MyHandler)?;
    indexer.start().await?;
    Ok(())
}
```

---

## 11. Completed Features & Roadmap

### ✅ Completed (v0.2.0)

| Feature | Status |
|:---|:---|
| RPC Polling (localnet + mainnet) | ✅ |
| WebSocket Subscriptions | ✅ |
| Hybrid Dual-Stream (WS + RPC) | ✅ |
| Helius Enhanced RPC | ✅ |
| Parallel Transaction Fetching | ✅ |
| Idempotency Tracking | ✅ |
| Multi-Program Indexing | ✅ |
| Instruction Decoder Registry | ✅ |
| Log Decoder Registry | ✅ |
| Account Decoder Registry | ✅ |
| IDL-Driven Type Generation (proc macros) | ✅ |
| Historical Backfill Engine | ✅ |
| Reorg Detection & Finality Monitor | ✅ |
| `EventHandler::on_rollback()` | ✅ |
| Builder Pattern Configuration | ✅ |
| Criterion Benchmarks (decoder, storage, throughput) | ✅ |
| Integration Test Suite | ✅ |
| 11 Working Examples (Jupiter, Raydium, SPL, etc.) | ✅ |
| **Yellowstone gRPC Streaming (Laserstream)** | ✅ |

### 🚀 Roadmap (v0.3.0+)

| Feature | Priority | Description |
|:---|:---|:---|
| **crates.io Release** | 🔴 High | Publish to crates.io for ecosystem adoption |
| **Configurable Retry Logic** | 🟡 Medium | Exponential backoff for transient RPC failures |
| **Dead-Letter Queue** | 🟡 Medium | Capture events that fail after all retries |
| **Prometheus/Grafana Metrics** | 🟡 Medium | Observability dashboard integration |
| **Custom Database Backends** | 🟡 Medium | SQLite, ClickHouse, MongoDB support |
| **Rate Limiting** | 🟢 Low | Respect RPC provider quotas automatically |
| **GraphQL Query Layer** | 🟢 Low | Auto-generated query API from indexed data |