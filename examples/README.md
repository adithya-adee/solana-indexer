# Solana Indexer Examples

This directory contains a suite of professional examples demonstrating how to use the `solana-indexer-sdk` for various blockchain indexing scenarios.

## Core Concepts (LLM & Developer Guide)

To build a custom indexer with this SDK, you follow a three-pillar pattern: **Define, Decode, and Handle**.

### 1. Define the Event
Create a `Borsh`-serializable struct representing the data you want to extract. Implement `EventDiscriminator` to provide a unique 8-byte ID (typically using `calculate_discriminator("YourEventName")`).

```rust
#[derive(BorshSerialize, BorshDeserialize)]
pub struct MyTransferEvent { pub from: Pubkey, pub amount: u64 }

impl EventDiscriminator for MyTransferEvent {
    fn discriminator() -> [u8; 8] { calculate_discriminator("MyTransferEvent") }
}
```

### 2. Implement the Decoder
Implement `InstructionDecoder<T>` (for transaction data) or `LogDecoder<T>` (for program logs). The `decode` method takes raw Solana data and returns `Some(YourEvent)` if it matches your criteria.

*   **RPC Polling/WS:** Uses `InstructionDecoder` on `UiInstruction`.
*   **Helius/Logs:** Uses `LogDecoder` on `ParsedEvent`.

### 3. Implement the Handler
Implement `EventHandler<T>`. This is where your business logic lives (e.g., database writes, notifications). It is asynchronous and provided with a `PgPool` and transaction metadata.

### 4. Orchestration
Initialize the `SolanaIndexer` with a `SolanaIndexerConfig` (built via `SolanaIndexerConfigBuilder`), register your decoders and handlers, and call `indexer.start()`.

---

## Example Directory Reference

| Category | Example | Focus |
|:---|:---|:---|
| ⭐ **Showcase** | [rpc_system_transfer.rs](./rpc_system_transfer.rs) | **Production-ready template.** Demonstrates advanced configuration, graceful shutdown, and professional practices. **Start here.** |
| **Basics** | [rpc_spl_token.rs](./rpc_spl_token.rs) | SPL Token transfers (e.g., USDC) with complex instruction parsing. |
| **Real-time** | [ws_system_transfer.rs](./ws_system_transfer.rs) | Sub-second latency using WebSocket subscriptions. |
| | [helius_system_transfer.rs](./helius_system_transfer.rs) | Using Helius as a high-performance data source. |
| **Advanced** | [account_indexer.rs](./account_indexer.rs) | Indexing account state changes (Crank-based) instead of instructions. |
| | [backfill_indexer.rs](./backfill_indexer.rs) | Historical data indexing with reorg protection. |
| | [multi_program_indexer.rs](./multi_program_indexer.rs) | Running multiple indexing pipelines in one process with shared storage. |
| **Observability** | [otel_indexer.rs](./otel_indexer.rs) | Tracing context and generating performance spans using OpenTelemetry, Tempo, Prometheus, and Grafana (see `observability/`). |
| **DEX/DeFi** | [raydium_indexer.rs](./raydium_indexer.rs) | Parsing partially decoded instructions from the Raydium AMM. |
| | [jupiter_swap_indexer.rs](./jupiter_swap_indexer.rs) | Event-based indexing for Jupiter aggregator swaps. |
| **Utilities** | [generator_spl_transfer.rs](./generator_spl_transfer.rs) | Test data generator for local development. |
| | [verify_shutdown.rs](./verify_shutdown.rs) | Demonstrates graceful shutdown and resource cleanup. |

## Running Examples

1. **Environment Setup**: Create a `.env` file in the project root:
   ```env
   RPC_URL=https://api.mainnet-beta.solana.com
   DATABASE_URL=postgresql://postgres:password@localhost/indexer
   PROGRAM_ID=... # Optional depending on the example
   ```
2. **Execute**:
   ```bash
   cargo run --example <name>
   ```
