# Solana Indexer Examples

This directory contains a set of examples that demonstrate how to use the `solana-indexer-sdk` for various indexing scenarios.

## Prerequisites

Most examples require a PostgreSQL database and access to a Solana RPC node. You can set these up using environment variables or a `.env` file in the project root.

```env
RPC_URL=https://api.mainnet-beta.solana.com
DATABASE_URL=postgresql://postgres:password@localhost/indexer
```

## Running Examples

To run an example, use the following command:

```bash
cargo run --example <name>
```

## Available Examples

| Example | Description | Command |
|---------|-------------|---------|
| **System Transfers** | Real-time indexing of native SOL transfers via RPC polling. | `cargo run --example rpc_system_transfer` |
| **SPL Token Transfers** | Indexing SPL token transfers via RPC polling. | `cargo run --example rpc_spl_token` |
| **WebSocket Transfers** | Real-time indexing of System Program transfers using WebSockets. | `cargo run --example ws_system_transfer` |
| **Helius Webhooks** | High-performance indexing using Helius as the data source. | `cargo run --example helius_system_transfer` |
| **Raydium Swaps** | Indexing token swaps on the Raydium DEX. | `cargo run --example raydium_indexer` |
| **Jupiter Swaps** | Indexing token swaps on the Jupiter aggregator (v6). | `cargo run --example jupiter_swap_indexer` |
| **Account Indexer** | Tracking account-level changes and history via transaction processing. | `cargo run --example account_indexer` |
| **Historical Backfill** | Indexing historical data using the backfill engine. | `cargo run --example backfill_indexer` |
| **Multi-Program** | Indexing multiple programs simultaneously in a single instance. | `cargo run --example multi_program_indexer` |
| **Transfer Generator** | A utility to generate test SPL token transfers for indexing. | `cargo run --example generator_spl_transfer` |
| **Verify Shutdown** | A utility to verify graceful shutdown of the indexer. | `cargo run --example verify_shutdown` |
