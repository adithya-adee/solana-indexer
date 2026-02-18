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
- **IDL-Based Type Generation:** Automatically generate Rust types from Solana program IDL files.

## IDL-Based Type Generation

The SDK includes support for automatically generating Rust types from Solana program IDL files. This allows you to work with type-safe event and instruction types without manually writing boilerplate code.

### Quick Start

1. **Add the IDL parser feature** to your `Cargo.toml`:

```toml
[dependencies]
solana-indexer-sdk = { path = "../solana-indexer-sdk", features = ["idl-parser"] }

[build-dependencies]
solana-idl-parser = { path = "../solana-idl-parser" }
```

2. **Create a `build.rs` file** in your project root:

```rust
use std::env;
use std::path::PathBuf;

fn main() {
    let idl_path = PathBuf::from("idl/my_program.json");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let generated_path = out_dir.join("generated_types.rs");

    solana_idl_parser::generate_sdk_types(&idl_path, &generated_path)
        .expect("Failed to generate types from IDL");

    println!("cargo:rerun-if-changed={}", idl_path.display());
}
```

3. **Include the generated types** in your code:

```rust
// src/lib.rs or src/main.rs
include!(concat!(env!("OUT_DIR"), "/generated_types.rs"));
```

4. **Use the generated types** in your decoders and handlers:

```rust
use solana_indexer_sdk::{EventHandler, EventDiscriminator};
use sqlx::PgPool;

// MyEvent is generated from your IDL
impl EventHandler<MyEvent> for MyEventHandler {
    async fn handle(
        &self,
        event: MyEvent,
        context: &TxMetadata,
        db: &PgPool,
    ) -> Result<(), SolanaIndexerError> {
        // Use the generated event type
        println!("Received event: {:?}", event);
        Ok(())
    }
}
```

### Generated Types

The IDL parser generates:

- **Event Structs**: With `BorshSerialize`, `BorshDeserialize`, and `EventDiscriminator` implementations
- **Type Structs**: For custom types defined in your IDL
- **Error Enums**: For program error codes

All generated types are compatible with the SDK's decoder and handler system.

### Example

See [`examples/idl_indexer.rs`](../examples/idl_indexer.rs) for a complete example of using IDL-generated types in an indexer.

## Documentation

- [Full API Reference (docs.rs)](https://docs.rs/solana-indexer-sdk)
- [Architecture Overview](https://github.com/adithya-adee/solana-indexer/blob/main/docs/ARCHITECTURE.md)
- [Examples Directory](https://github.com/adithya-adee/solana-indexer/tree/main/examples)

## License

Licensed under either of [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0) or [MIT license](https://opensource.org/licenses/MIT) at your option.
