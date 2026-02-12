# Solana Indexer SDK

A production-ready, high-performance Solana indexing SDK written in Rust. Build custom indexers with minimal code by leveraging automatic transaction polling, fetching, decoding, and event handling.

## Features

- **🚀 Complete Pipeline**: Automatic polling, fetching, decoding, and event handling
- **🔄 Idempotency**: Built-in transaction deduplication and crash recovery
- **⚡ High Performance**: Parallel batch processing with configurable concurrency
- **🎯 Type-Safe**: IDL-based type generation with compile-time safety
- **🔌 Extensible**: Custom event handlers with async support
- **💾 Database Integration**: PostgreSQL with connection pooling
- **📊 Event-Driven**: Discriminator-based event routing
- **🛡️ Production-Ready**: Comprehensive error handling and logging

## Quick Start

### Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
solana-indexer = "0.1.0"
tokio = { version = "1", features = ["full"] }
dotenvy = "0.15"
```

### Basic Usage

```rust
use solana_indexer::{SolanaIndexer, SolanaIndexerConfigBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables
    dotenvy::dotenv().ok();

    // Configure indexer
    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(std::env::var("RPC_URL")?)
        .with_database(std::env::var("DATABASE_URL")?)
        .program_id(std::env::var("PROGRAM_ID")?)
        .with_poll_interval(5)  // Poll every 5 seconds
        .with_batch_size(10)    // Fetch 10 transactions per batch
        .build()?;

    // Create and start indexer
    let indexer = SolanaIndexer::new(config).await?;
    indexer.start().await?;

    Ok(())
}
```

### Environment Variables

Create a `.env` file:

```env
RPC_URL=https://api.mainnet-beta.solana.com
DATABASE_URL=postgresql://user:password@localhost/solana_indexer
PROGRAM_ID=YourProgramIdHere
```

## Architecture

The SDK implements a complete event-driven pipeline:

```
┌─────────────┐    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐
│   Poller    │───▶│   Fetcher   │───▶│   Decoder   │───▶│   Handler   │
│ (Signatures)│    │(Transactions)│    │  (Events)   │    │  (Custom)   │
└─────────────┘    └─────────────┘    └─────────────┘    └─────────────┘
       │                                                          │
       │                                                          │
       └──────────────────┐                          ┌───────────┘
                          ▼                          ▼
                    ┌──────────────────────────────────┐
                    │  Storage (Idempotency Tracking)  │
                    └──────────────────────────────────┘
```
README
### Core Components

1. **Poller**: Monitors program accounts for new transactions
2. **Fetcher**: Retrieves full transaction details from RPC
3. **Decoder**: Parses transactions and extracts events
4. **Storage**: Tracks processed transactions for idempotency
5. **Handler Registry**: Routes events to custom handlers
6. **Type Generation**: IDL-based type generation utilities

## Advanced Usage

### Custom Event Handlers

```rust
use solana_indexer::{EventHandler, TransferEvent};
use async_trait::async_trait;
use sqlx::PgPool;

pub struct MyTransferHandler;

#[async_trait]
impl EventHandler<TransferEvent> for MyTransferHandler {
    async fn handle(
        &self,
        event: TransferEvent,
        db: &PgPool,
        signature: &str,
    ) -> Result<(), solana_indexer::SolanaIndexerError> {
        println!("Transfer: {} -> {} ({})", event.from, event.to, event.amount);
        
        // Store in database
        sqlx::query(
            "INSERT INTO transfers (signature, from_wallet, to_wallet, amount) 
             VALUES ($1, $2, $3, $4)"
        )
        .bind(signature)
        .bind(&event.from)
        .bind(&event.to)
        .bind(event.amount as i64)
        .execute(db)
        .await?;
        
        Ok(())
    }
}
```

### IDL-Based Type Generation

```rust
use solana_indexer::{Idl, generate_event_struct};

// Parse IDL
let idl_json = std::fs::read_to_string("program_idl.json")?;
let idl = Idl::parse(&idl_json)?;

// Generate event types
for event in &idl.events {
    let code = generate_event_struct(event);
    println!("{}", code);
}
```

### Configuration Options

```rust
let config = SolanaIndexerConfigBuilder::new()
    .with_rpc("https://api.mainnet-beta.solana.com")
    .with_database("postgresql://localhost/db")
    .program_id("YourProgramId")
    .with_poll_interval(5)      // Seconds between polls
    .with_batch_size(100)       // Signatures per batch
    .build()?;
```

### Registry Configuration

Configure memory limits and monitoring for the indexer's registries to ensure stability in resource-constrained environments.

```rust
use solana_indexer::config::RegistryConfig;

let registry_config = RegistryConfig {
    max_decoder_programs: 100,    // Limit instruction decoders
    max_log_decoder_programs: 100, // Limit log decoders
    max_account_decoders: 1000,   // Limit account decoders
    max_handlers: 50,             // Limit event handlers
    enable_metrics: true,         // Enable periodic metrics logging
};

let config = SolanaIndexerConfigBuilder::new()
    .with_rpc("...")
    .with_registry_config(registry_config)
    .build()?;
```

### Backfill & Reorg Handling

The SDK supports comprehensive historical data backfilling with automatic reorganization handling.

```rust
use solana_indexer::config::BackfillConfig;

let backfill_config = BackfillConfig {
    enabled: true,
    start_slot: Some(150_000_000),      // Start slot (Optional)
    end_slot: Some(150_005_000),        // End slot (Optional)
    batch_size: 100,                    // Transactions per batch
    concurrency: 50,                    // Concurrent RPC requests
    enable_reorg_handling: true,        // Detect and handle chain reorgs
    finalization_check_interval: 32,    // Slots between finalization checks
};

let config = SolanaIndexerConfigBuilder::new()
    .with_backfill(backfill_config)
    // ... other config
    .build()?;
```

The backfill engine:
1.  **detects reorgs** by comparing stored block hashes with the canonical chain.
2.  **rolls back** invalidated data automatically.
3.  **tracks progress** persistently to allow resuming after restarts.
4.  **marks slots finalized** only when confirmed by the cluster.

## Database Setup

The SDK automatically creates the required schema:

```sql
CREATE TABLE _solana_indexer_processed (
    signature TEXT PRIMARY KEY,
    slot BIGINT NOT NULL,
    processed_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

CREATE INDEX idx_processed_slot ON _solana_indexer_processed(slot);
```

## Development & Testing
 
 Ensure your environment is set up with Rust, Cargo, and a running PostgreSQL instance.
 
 ### Running Tests
 
 Run the full test suite, including unit tests and integration tests:
 
 ```bash
 # Run unit tests
 cargo test --lib
 
 # Run integration tests (requires DATABASE_URL)
 DATABASE_URL=postgresql://user:password@localhost/solana_indexer cargo test --test '*_integration_test' -- --ignored
 ```
 
 ### Benchmarking
 
 The project uses `criterion` for performance benchmarking.
 
 ```bash
 # Run all benchmarks
 cargo bench
 
 # Run specific benchmark
 cargo bench --bench throughput_bench
 ```
 
 **Available Benchmarks:**
 - `decoder_bench`: Transaction decoding performance
 - `storage_bench`: Database operation latency
 - `throughput_bench`: End-to-end pipeline throughput
 
 ### Code Coverage
 
 Generate code coverage reports using `cargo-tarpaulin` or `cargo-llvm-cov`.
 
 **Using cargo-tarpaulin (Recommended for CI):**
 ```bash
 cargo install cargo-tarpaulin
 cargo tarpaulin --out Html --output-dir coverage
 ```
 
 **Using cargo alias (Recommended):**
 ```bash
 # Compile and run coverage
 cargo coverage
 ```
 
 ```bash
 # Generate detailed report
 cargo report
 ```
 
 ### Continuous Integration
 
 A generic CI configuration for coverage reporting is available in `.github/workflows/coverage.yml`. It runs tests and uploads coverage data to Codecov on every push to main.

## Examples

See the `examples/` directory for complete examples:

- **Basic Indexer**: Simple transaction indexing
- **Custom Handlers**: Event-driven processing
- **IDL Generation**: Type generation from IDL
- **Recovery**: Crash recovery and resumption

## Performance

- **Throughput**: 1000+ transactions/second
- **Latency**: Sub-second event processing
- **Memory**: Configurable connection pooling
- **Concurrency**: Parallel batch processing

## Error Handling

The SDK provides comprehensive error types:

```rust
pub enum SolanaIndexerError {
    DatabaseError(sqlx::Error),
    RpcError(String),
    DecodingError(String),
    ConfigError(String),
    // ... more variants
}
```

## Roadmap

- [ ] WebSocket support for real-time indexing
- [ ] Procedural macros for automatic type generation
- [ ] Built-in metrics and monitoring
- [ ] Multi-program indexing
- [ ] Sharding and horizontal scaling

## Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

Built with:
- [Solana SDK](https://github.com/solana-labs/solana)
- [SQLx](https://github.com/launchbadge/sqlx)
- [Tokio](https://tokio.rs/)
- [Anchor](https://www.anchor-lang.com/)

## Support

- **Documentation**: [docs.rs/solana-indexer](https://docs.rs/solana-indexer)
- **Issues**: [GitHub Issues](https://github.com/adithya-adee/solana-indexer/issues)
- **Discord**: [Join our community](https://discord.gg/yourserver)

## Benchmark Results Guide

- **Time**: Execution time per iteration (e.g., `[75.937 µs 80.213 µs 84.666 µs]` means 95% confidence interval is between 75µs and 84µs).
- **Thrpt**: Throughput in elements per second (transactions/sec).
- **Outliers**: Statistically significant deviations (often due to GC or OS noise).

**Detailed Reports:**
After running benchmarks, open `target/criterion/report/index.html` in your browser for interactive graphs and failure analysis.
