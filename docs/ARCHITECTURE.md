# SolStream Architecture

**Build Philosophy:** SolStream is designed as a developer-centric SDK for Solana data indexing. Our philosophy is to provide a highly extensible, reliable, and secure platform that prioritizes a seamless developer experience (DX) while offering a clear path from local development to production-grade deployments. We start with foundational reliability on localnet and strategically integrate production-ready features.

## 1. Core System Architecture: Event-Driven Pipeline

SolStream operates on an event-driven pipeline model, designed for flexibility in data acquisition and processing. This architecture ensures modularity, allowing developers to choose input sources and define custom handling logic.

### Data Flow Overview

The core pipeline consists of the following stages:

1.  **Input Source (Poller / Subscriber):** Responsible for acquiring raw transaction data.
    *   **Poller:** Periodically queries Solana RPC endpoints for new transaction signatures (e.g., `getSignaturesForAddress`). Ideal for localnet and lower-throughput scenarios.
    *   **Subscriber:** Listens for real-time transaction events via WebSocket connections (e.g., `programSubscribe`). Essential for high-throughput, low-latency production environments.
2.  **Fetcher:** For each identified new transaction signature, it retrieves the full transaction details (e.g., `getTransaction`). This includes instruction data, logs, and metadata.
3.  **Decoder:** Parses the raw transaction data.
    *   Utilizes automatically generated Rust structs from the program's IDL (Interface Definition Language) to interpret instruction data and emitted events into strongly-typed Rust objects. This stage ensures type safety and reduces boilerplate for developers.
    *   Handles common Solana transaction types (e.g., SPL Token Program instructions) even without a specific program IDL, allowing for general-purpose indexing.
4.  **Idempotency Tracker:** Before processing, checks a persistent store (`_solstream_processed` table) to prevent re-processing already handled transactions. This ensures data consistency and reliability.
5.  **Handler:** Executes user-defined business logic.
    *   This is where the developer's custom code integrates. For each decoded event or instruction, the SDK invokes a registered `EventHandler` implementation.
    *   Typical operations include database inserts, updates, external API calls, or triggering further processing.
6.  **Confirmation & Persistence:** Marks the transaction signature as processed in the idempotency tracker.

### Simplified Flow Diagram (Evolving)

```
┌───────────────────────┐
│ Solana Network        │
│ RPC / WebSocket       │
└───────────┬───────────┘
            │
            ▼
┌──────────────────────────────────────┐
│ SolStream Core Loop                  │
│ (Potentially Multi-threaded/Async)   │
│                                      │
│  1. Input Source (Poller/Subscriber) │
│  2. Fetch transaction data           │
│  3. Decode via IDL/Known Structures  │
│  4. Check idempotency table          │
│  5. Call user-defined EventHandler   │
│  6. Mark signature as processed      │
└───────────┬──────────────────────────┘
            │
            ▼
┌───────────────────────┐
│ External Services     │
│                       │
│ • Database (Postgres/ │
│   Supabase)           │
│ • Other APIs          │
│ • Analytics Platforms │
└───────────────────────┘
```

## 2. Configuration Management

SolStream provides a flexible configuration system, blending ease-of-use with programmatic control, crucial for both local development and production deployments.

### `SolStreamConfig` and Builder Pattern

Configuration is managed via a `SolStreamConfig` struct, built using a fluent builder pattern. This approach offers:
*   **Type Safety:** All configuration parameters are strongly typed.
*   **Discoverability:** IDE auto-completion guides developers through available options.
*   **Flexibility:** Easily integrate configuration from various sources (environment variables, files, hardcoded defaults).

**Example:**
```rust
SolStream::new()
    .with_rpc("http://127.0.0.1:8899")
    .with_database(env::var("DATABASE_URL")?)
    .program_id(env::var("PROGRAM_ID")?)
    .register_handler::<TransferEvent>(TransferHandler)
    .start() // Or .start_polling(), .start_websocket()
    .await
```

### Environment Variable Integration

For convenience, especially in local development, SolStream seamlessly integrates with environment variables:
*   **Automatic `env::var` Lookup:** Configuration methods (e.g., `.with_database()`) can directly consume values from `std::env::var`.
*   **`.env` File Support:** Developers can use popular crates like `dotenv` to load variables from a `.env` file into the environment. SolStream expects these to be loaded *before* its configuration methods are called.

**Best Practice:** Always `.gitignore` your `.env` files to prevent sensitive information from being committed to version control. For production, rely on explicit environment variables or secrets management systems rather than `.env` files.

## 3. Indexer Types and Selection

SolStream supports multiple indexing strategies, allowing developers to select the best fit for their use case and performance requirements.

*   **Program-Specific Polling (Current Default):**
    *   **Mechanism:** Periodically queries an RPC node for transactions related to a specific `PROGRAM_ID`.
    *   **Use Case:** Ideal for local development, testing, and applications with moderate throughput requirements on a single program.
    *   **Activation:** Configured via `.program_id(...)` and initiated with `.start_polling()`.

*   **Program-Specific WebSocket Subscriptions (Production Roadmap):**
    *   **Mechanism:** Maintains a persistent WebSocket connection to an RPC node, receiving real-time notifications for transactions involving a specific `PROGRAM_ID`.
    *   **Use Case:** Critical for high-throughput, low-latency production applications requiring immediate data processing.
    *   **Activation:** Initiated with `.start_websocket()`.

*   **General/Multi-Program Indexing (Production Roadmap):**
    *   **Mechanism:** Capabilities to index events or instructions from multiple programs, or to process broad categories of on-chain activity (e.g., all SPL token transfers) without being tied to a single `PROGRAM_ID`. This often involves integrating with specialized data providers (e.g., Helius, Blockdaemon).
    *   **Use Case:** Building comprehensive blockchain analytics, cross-program interactions, or aggregated data services.

## 4. IDL Processing & Type Generation (Developer Experience Highlight)

A cornerstone of SolStream's DX is its ability to transform Solana program IDLs into strongly-typed Rust code.

*   **IDL-Driven Development:** Developers place their Solana program's `idl.json` file(s) in a designated `idl/` directory within their project.
*   **Procedural Macro Compilation:** During `cargo build`, a SolStream procedural macro reads and parses these `idl.json` files.
*   **Automatic Code Generation:** The macro automatically generates:
    *   Rust `struct` definitions for all defined accounts and **emitted events**.
    *   Necessary traits (e.g., `AnchorDeserialize`, `BorshDeserialize`) for deserializing raw on-chain data into these generated types.
    *   An example generated type: `use solstream::generated::TransferEvent;`.
*   **Type Safety and Reduced Boilerplate:** This generation eliminates manual struct definition and deserialization logic, providing compile-time type checking and significantly reducing development effort and potential errors.
*   **Event Discriminators:** The SDK's `Decoder` uses unique 8-byte discriminators (automatically derived or specified in the IDL) to identify and correctly deserialize emitted events from transaction logs into their corresponding generated Rust types.

### General Indexing Without IDL

For scenarios where a specific program IDL isn't available or desired (e.g., indexing generic SPL Token transfers), SolStream's `Decoder` can also interpret common instruction types by directly parsing their known byte layouts. This provides flexibility but requires the SDK to maintain knowledge of these specific instruction formats.

## 5. Extensibility: The `EventHandler` Trait (Custom Logic Injection)

The `EventHandler` trait is SolStream's primary extension point, enabling developers to inject their custom business logic cleanly and efficiently.

```rust
#[async_trait]
pub trait EventHandler<T>: Send + Sync + 'static {
    async fn handle(&self, event: T, db: &PgPool, signature: &str)
        -> Result<(), SolStreamError>;
}
```

*   **Generic for Any Event `T`:** The trait is generic over `T`, meaning developers can implement `EventHandler` for *any* event `struct` (typically a generated event type from an IDL) they wish to process.
*   **`handle` Method:** This `async` method is where all custom logic resides.
    *   `event: T`: The strongly-typed, deserialized event object from the blockchain.
    *   `db: &PgPool`: A database connection pool (e.g., `sqlx::PgPool`), providing a convenient way to interact with a persistent store.
    *   `signature: &str`: The transaction signature, useful for logging, linking, or debugging.
*   **SDK Invocation:** The SolStream core loop, after decoding a transaction and identifying an event of type `T`, automatically finds the developer-registered `EventHandler<T>` instance and calls its `handle` method. This abstraction means developers only focus on *what* to do with the data, not *how* to acquire or decode it.

## 6. Reliability & Error Handling

Reliability is paramount for an indexing solution. SolStream implements robust mechanisms to ensure data integrity and system stability.

*   **Idempotency:** The `_solstream_processed` table (managed by the `Tracker`) guarantees that each unique transaction is processed only once, preventing duplicate entries or side effects even if the indexer restarts or encounters temporary failures.
*   **Structured `SolStreamError`:**
    ```rust
    pub enum SolStreamError {
        DatabaseError(sqlx::Error), // Errors during database operations
        DecodingError(String),      // Errors parsing transaction data or events
        RpcError(String),           // Errors interacting with Solana RPC
        // ... potentially more specific error types for network, config, etc.
    }
    ```
    Errors are explicitly typed, allowing for precise handling and debugging.
*   **Explicit Error Propagation:** All errors propagate up the call stack, ensuring no silent failures.
*   **Logging:** Detailed logging (e.g., using `tracing` crate) is integrated throughout the SDK to provide visibility into the pipeline's operation, aiding in diagnostics and monitoring.
*   **Retry Logic (Production Roadmap):** For transient failures (e.g., RPC timeouts), a production-ready SolStream will implement configurable retry mechanisms with exponential backoff.
*   **Database Transactions:** For handlers performing multiple database operations, developers are strongly encouraged to wrap their logic in database transactions to ensure atomicity (all or nothing) and data consistency.

## 7. Security Considerations

Security is built into SolStream's design, protecting both the indexed data and the developer's application.

*   **No Hardcoded Secrets:** Configuration encourages the use of environment variables, preventing sensitive information (database credentials, API keys) from being hardcoded or committed to version control.
*   **Input Validation (Implicit):** IDL-driven decoding inherently provides a form of input validation. Data not conforming to the defined IDL structs will fail deserialization, preventing malformed on-chain data from being incorrectly processed.
*   **Safe Database Interactions:** By promoting `sqlx` and parameterized queries, SolStream guides developers towards preventing common database vulnerabilities like SQL injection.
*   **Minimal Privileges:** Encourage running the indexer with the minimum necessary permissions for both network and database access.
*   **Open Source Auditability:** As an open-source SDK, the codebase is transparent and auditable by the community, fostering trust and allowing for early detection of potential vulnerabilities.

## 8. Directory Structure

```
solstream-sdk/
├── Cargo.toml
├── idl/
│   └── your_program.json          // Developer drops IDL here (e.g., from Anchor)
├── src/
│   ├── lib.rs                     // Public API and main entry point
│   ├── config.rs                  // SolStreamConfig and builder logic
│   ├── input_sources/             // Poller and future Subscriber implementations
│   │   ├── poller.rs
│   │   └── websocket.rs           // (Future)
│   ├── fetcher.rs                 // Logic for fetching full transaction data
│   ├── decoder.rs                 // IDL-driven and generic data parsing
│   ├── tracker.rs                 // Idempotency tracking logic
│   ├── traits.rs                  // EventHandler trait definition
│   ├── error.rs                   // SolStreamError enum definition
│   ├── storage.rs                 // Database interaction utilities (e.g., SQLx)
│   └── macros/                    // Procedural macro for IDL compilation
└── examples/
    └── token_transfer_indexer.rs  // Demo implementation of an EventHandler
```

### Build Flow:
1.  **Define Program:** Developer writes their Solana program (e.g., using Anchor) and generates its `idl.json`.
2.  **Integrate IDL:** Developer places `idl/your_program.json` into their SolStream project.
3.  **Generate Types:** Runs `cargo build` which triggers the SolStream Proc-macro to generate Rust `structs` from the IDL.
4.  **Import Generated Types:** Developer imports generated types: `use solstream::generated::TransferEvent;`.
5.  **Implement Handler:** Implements the `EventHandler<TransferEvent>` trait for their custom logic.
6.  **Configure & Run:** Configures SolStream using the builder pattern and runs the indexer with `cargo run`.

## 9. Developer Quickstart

### Step 1: Setup Configuration

```bash
# .env file (for local development, add to .gitignore)
DATABASE_URL=postgresql://user:pass@localhost:5432/mydb
RPC_URL=http://127.0.0.1:8899
PROGRAM_ID=YourProgramPublicKey111111111111111111111
```

For production, these should be explicit environment variables.

### Step 2: Implement Your EventHandler

```rust
use solstream::{EventHandler, SolStreamError};
use solstream::generated::TransferEvent; // Assuming your IDL defines a TransferEvent
use sqlx::PgPool;
use async_trait::async_trait;

pub struct MyTransferHandler;

#[async_trait]
impl EventHandler<TransferEvent> for MyTransferHandler {
    async fn handle(&self, event: TransferEvent, db: &PgPool, signature: &str)
        -> Result<(), SolStreamError> {
        println!("Processing Transfer: Sig={}, From={}, To={}, Amount={}",
                 signature, event.from, event.to, event.amount);

        // Example: Persist to database
        sqlx::query!(
            "INSERT INTO transfers (signature, from_wallet, to_wallet, amount)
             VALUES ($1, $2, $3, $4)",
            signature, event.from.to_string(), event.to.to_string(), event.amount as i64
        )
        .execute(db)
        .await
        .map_err(SolStreamError::DatabaseError)?;
        
        Ok(())
    }
}
```

### Step 3: Initialize and Run SolStream

```rust
use solstream::SolStream;
use std::env;
use anyhow::Result; // Using anyhow for simplified error handling in main

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file for local development (optional, but good practice)
    dotenv::dotenv().ok(); 

    // Initialize SolStream with configuration and register handlers
    SolStream::new()
        // Configure RPC endpoint
        .with_rpc(&env::var("RPC_URL")?)
        // Configure database connection
        .with_database(&env::var("DATABASE_URL")?)
        // Specify the program ID to index
        .program_id(&env::var("PROGRAM_ID")?)
        // Register your custom event handler
        .register_handler::<TransferEvent>(MyTransferHandler)
        // Start the indexing process (e.g., polling for now)
        .start_polling() 
        .await?;

    Ok(())
}
```
**That's it! Your indexer is now configured to process `TransferEvent`s for your specified program.**

## 10. Production Roadmap & Future Enhancements

SolStream is on a continuous path of improvement, with a clear roadmap to enhance its capabilities for production environments.

*   **Phase 1 (Current):** Foundational localnet polling, robust idempotency, `sqlx` integration, IDL-driven type generation.
*   **Phase 2: Real-time & Resilient Indexing:**
    *   WebSocket subscriptions for low-latency event processing.
    *   Configurable retry logic with exponential backoff for transient RPC failures.
    *   Rate limiting to respect RPC provider quotas.
*   **Phase 3: Data Integrity & Provider Integration:**
    *   Helius / other specialized RPC provider integrations for enhanced reliability, historical data, and gap-filling backfill.
    *   Monitoring and observability dashboards (e.g., Prometheus/Grafana integration).
    *   Dead-letter queue support for unprocessable events.
*   **Phase 4: Advanced Indexing Patterns:**
    *   Multi-program indexing capabilities from a single SolStream instance.
    *   Custom RPC provider support for greater flexibility.
    *   Performance benchmarks and optimizations against existing indexing solutions.
    *   Support for on-chain state change tracking (e.g., account data changes).