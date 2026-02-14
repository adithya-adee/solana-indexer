# Solana Indexer SDK

[![Crates.io](https://img.shields.io/crates/v/solana-indexer-sdk.svg)](https://crates.io/crates/solana-indexer-sdk)
[![Docs.rs](https://docs.rs/solana-indexer-sdk/badge.svg)](https://docs.rs/solana-indexer-sdk)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](#license)

A developer-centric, high-performance SDK for building custom Solana blockchain indexers.

## Installation

```bash
cargo add solana-indexer-sdk
```

## 30-Second Quickstart

```rust,no_run
use solana_indexer_sdk::{SolanaIndexer, SolanaIndexerConfigBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc("https://api.mainnet-beta.solana.com")
        .with_database("postgresql://postgres:password@localhost/indexer")
        .program_id("675k1q2wE7s6L3R29fs6tcMbtFD4vT759Wcx3CY6CSLg") // Raydium
        .build()?;

    let mut indexer = SolanaIndexer::new(config).await?;
    
    // Register your custom logic (InstructionDecoder and EventHandler)
    // indexer.decoder_registry_mut().register(...);
    // indexer.handler_registry_mut().register(...);

    indexer.start().await?;
    Ok(())
}
```

## Features

- **Multi-Program Indexing:** Index transactions from multiple programs simultaneously in a single instance.
- **Customizable Pipelines:** Decouple instruction decoding from event handling for maximum flexibility.
- **High-Performance:** Built on `tokio` for efficient asynchronous processing and high throughput.
- **Automatic Reorg Handling:** Built-in detection and recovery for chain reorganizations.
- **Backfill Engine:** Seamlessly index historical data alongside real-time updates.
- **Flexible Data Sources:** Support for RPC polling, WebSocket streams, and Helius Webhooks.
- **Type-Safe Events:** Strongly-typed event definitions using Borsh serialization.

## Documentation

- [Full API Reference (docs.rs)](https://docs.rs/solana-indexer-sdk)
- [Architecture Overview](https://github.com/adithya-adee/solana-indexer/blob/main/docs/ARCHITECTURE.md)
- [Examples Directory](https://github.com/adithya-adee/solana-indexer/tree/main/examples)

## License

Licensed under either of [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0) or [MIT license](https://opensource.org/licenses/MIT) at your option.
