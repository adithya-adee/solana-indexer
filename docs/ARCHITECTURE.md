# Solana Indexer Architecture

**Build Philosophy:** SolanaIndexer is designed as a developer-centric SDK for Solana data indexing. Our philosophy is to provide a highly extensible, reliable, and secure platform that prioritizes a seamless developer experience (DX) while offering a clear path from local development to production-grade deployments. We solve the "boring problems" (polling loops, RPC connections, signature fetching, idempotency) so developers can focus on their unique business logic.

## 1. Core System Architecture: Flexible Event-Driven Pipeline

SolanaIndexer operates on a flexible event-driven pipeline model with **two key extension points** for developers:

1. **`InstructionDecoder<T>`**: Custom instruction parsing (raw bytes → typed events)
2. **`EventHandler<T>`**: Custom event processing (typed events → business logic)

This separation of concerns allows developers to build indexers for any use case—from general SPL token transfers to complex custom program logic—while the SDK handles all infrastructure concerns.

### Data Flow Overview

The core pipeline consists of the following stages:

1.  **Input Source (Poller / Subscriber):** Responsible for acquiring raw transaction data.
    *   **Poller:** Periodically queries Solana RPC endpoints for new transaction signatures (e.g., `getSignaturesForAddress`). Ideal for localnet and lower-throughput scenarios.
    *   **Subscriber:** Listens for real-time transaction events via WebSocket connections (e.g., `programSubscribe`). Essential for high-throughput, low-latency production environments.
2.  **Fetcher:** For each identified new transaction signature, it retrieves the full transaction details (e.g., `getTransaction`). This includes instruction data, logs, and metadata.
3.  **Idempotency Tracker:** Before processing, checks a persistent store (`_solana_indexer_processed` table) to prevent re-processing already handled transactions. This ensures data consistency and reliability.
4.  **Decoder Registry (Developer Extension Point #1):** Routes instructions to registered `InstructionDecoder<T>` implementations.
    *   Developers implement `InstructionDecoder<T>` to parse raw instruction data into strongly-typed event structs.
    *   Multiple decoders can be registered for different program IDs, enabling multi-program indexing.
    *   Decoders can use IDL-generated types (via proc macros) or custom parsing logic.
    *   Returns `Option<T>` where `T` is the typed event struct.
5.  **Handler Registry (Developer Extension Point #2):** Dispatches decoded events to registered `EventHandler<T>` implementations.
    *   Developers implement `EventHandler<T>` to define custom business logic for processing events.
    *   Typical operations include database inserts, updates, external API calls, or triggering further processing.
    *   Handlers can define custom database schemas via the `initialize_schema()` method.
6.  **Confirmation & Persistence:** Marks the transaction signature as processed in the idempotency tracker.

### Architecture Flow Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                    SDK Infrastructure                       │
│                   (We Handle This)                          │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌───────────────────────┐
│ Solana Network        │  ← RPC / WebSocket / Helius
│ (RPC Endpoint)        │
└───────────┬───────────┘
            │
            ▼
┌──────────────────────────────────────┐
│ 1. Input Source                      │  ← Poller / Subscriber
│    • Fetch new signatures            │
│    • Handle retries & rate limits    │
└───────────┬──────────────────────────┘
            │
            ▼
┌──────────────────────────────────────┐
│ 2. Transaction Fetcher               │
│    • Fetch full transaction details  │
│    • Extract instructions            │
└───────────┬──────────────────────────┘
            │
            ▼
┌──────────────────────────────────────┐
│ 3. Idempotency Check                 │
│    • Skip if already processed       │
└───────────┬──────────────────────────┘
            │
            ▼
┌─────────────────────────────────────────────────────────────┐
│              Developer Extension Point #1                    │
│                                                              │
│ 4. Decoder Registry                                          │
│    ┌──────────────────────────────────────────────┐        │
│    │ InstructionDecoder<T>                        │        │
│    │ • Developer implements custom parsing        │        │
│    │ • UiInstruction → Option<T>                  │        │
│    │ • Can use IDL-generated types                │        │
│    └──────────────────────────────────────────────┘        │
│                                                              │
│    Output: Vec<(discriminator, event_data)>                 │
└─────────────────────────┬───────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────────┐
│              Developer Extension Point #2                    │
│                                                              │
│ 5. Handler Registry                                          │
│    ┌──────────────────────────────────────────────┐        │
│    │ EventHandler<T>                              │        │
│    │ • Developer implements business logic        │        │
│    │ • T → Database/API/Custom Actions            │        │
│    │ • initialize_schema() for DB setup           │        │
│    └──────────────────────────────────────────────┘        │
└─────────────────────────┬───────────────────────────────────┘
                          │
                          ▼
┌──────────────────────────────────────┐
│ 6. Mark as Processed                 │
│    • Update idempotency tracker      │
└───────────┬──────────────────────────┘
            │
            ▼
┌───────────────────────┐
│ External Services     │
│ • PostgreSQL          │
│ • Supabase            │
│ • Custom APIs         │
│ • Analytics           │
└───────────────────────┘
```

**Key Insight:** Developers only implement the two extension points (`InstructionDecoder<T>` and `EventHandler<T>`). The SDK handles everything else.

## 2. Configuration Management

SolanaIndexer provides a flexible configuration system, blending ease-of-use with programmatic control, crucial for both local development and production deployments.

### `SolanaIndexerConfig` and Builder Pattern

Configuration is managed via a `SolanaIndexerConfig` struct, built using a fluent builder pattern. This approach offers:
*   **Type Safety:** All configuration parameters are strongly typed.
*   **Discoverability:** IDE auto-completion guides developers through available options.
*   **Flexibility:** Easily integrate configuration from various sources (environment variables, files, hardcoded defaults).

**Example:**
```rust
SolanaIndexerConfigBuilder::new()
    .with_rpc("http://127.0.0.1:8899") // or .with_ws("ws://...", "http://...")
    .with_database(env::var("DATABASE_URL")?)
    .program_id(env::var("PROGRAM_ID")?)
    .build()?;

let indexer = SolanaIndexer::new(config).await?;
indexer.start().await?;
```

### Environment Variable Integration

For convenience, especially in local development, SolanaIndexer seamlessly integrates with environment variables:
*   **Automatic `env::var` Lookup:** Configuration methods (e.g., `.with_database()`) can directly consume values from `std::env::var`.
*   **`.env` File Support:** Developers can use popular crates like `dotenv` to load variables from a `.env` file into the environment. SolanaIndexer expects these to be loaded *before* its configuration methods are called.

**Best Practice:** Always `.gitignore` your `.env` files to prevent sensitive information from being committed to version control. For production, rely on explicit environment variables or secrets management systems rather than `.env` files.

## 3. Indexer Types and Selection

SolanaIndexer supports multiple indexing strategies, allowing developers to select the best fit for their use case and performance requirements.

*   **Program-Specific Polling (Default via `with_rpc`):**
    *   **Mechanism:** Periodically queries an RPC node for transactions related to a specific `PROGRAM_ID`.
    *   **Use Case:** Ideal for local development, testing, and applications with moderate throughput requirements on a single program.
    *   **Activation:** Configured via `.with_rpc(...)`.

*   **Program-Specific WebSocket Subscriptions (Real-time via `with_ws`):**
    *   **Mechanism:** Maintains a persistent WebSocket connection to an RPC node, receiving real-time notifications for transactions involving a specific `PROGRAM_ID`.
    *   **Use Case:** Critical for high-throughput, low-latency production applications requiring immediate data processing.
    *   **Activation:** Configured via `.with_ws(...)`.

*   **General/Multi-Program Indexing (Production Roadmap):**
    *   **Mechanism:** Capabilities to index events or instructions from multiple programs, or to process broad categories of on-chain activity (e.g., all SPL token transfers) without being tied to a single `PROGRAM_ID`. This often involves integrating with specialized data providers (e.g., Helius, Blockdaemon).
    *   **Use Case:** Building comprehensive blockchain analytics, cross-program interactions, or aggregated data services.

## 4. IDL Processing & Type Generation (Developer Experience Highlight)

A cornerstone of SolanaIndexer's DX is its ability to transform Solana program IDLs into strongly-typed Rust code.

*   **IDL-Driven Development:** Developers place their Solana program's `idl.json` file(s) in a designated `idl/` directory within their project.
*   **Procedural Macro Compilation:** During `cargo build`, a SolanaIndexer procedural macro reads and parses these `idl.json` files.
*   **Automatic Code Generation:** The macro automatically generates:
    *   Rust `struct` definitions for all defined accounts and **emitted events**.
    *   Necessary traits (e.g., `AnchorDeserialize`, `BorshDeserialize`) for deserializing raw on-chain data into these generated types.
    *   An example generated type: `use solana_indexer::generated::TransferEvent;`.
*   **Type Safety and Reduced Boilerplate:** This generation eliminates manual struct definition and deserialization logic, providing compile-time type checking and significantly reducing development effort and potential errors.
*   **Event Discriminators:** The SDK's `Decoder` uses unique 8-byte discriminators (automatically derived or specified in the IDL) to identify and correctly deserialize emitted events from transaction logs into their corresponding generated Rust types.

### General Indexing Without IDL

For scenarios where a specific program IDL isn't available or desired (e.g., indexing generic SPL Token transfers), SolanaIndexer's `Decoder` can also interpret common instruction types by directly parsing their known byte layouts. This provides flexibility but requires the SDK to maintain knowledge of these specific instruction formats.

## 5. Extensibility: Developer Extension Points

SolanaIndexer provides two primary extension points that enable developers to inject custom logic cleanly and efficiently:

### Extension Point #1: `InstructionDecoder<T>` (Custom Instruction Parsing)

The `InstructionDecoder<T>` trait allows developers to define how raw Solana instructions are parsed into typed event structures.

```rust
/// Generic instruction decoder trait for custom parsing logic.
///
/// Developers implement this trait to define how raw Solana instructions
/// are parsed into typed event structures.
pub trait InstructionDecoder<T>: Send + Sync {
    /// Decodes a Solana instruction into a typed event.
    ///
    /// # Arguments
    /// * `instruction` - The UI instruction from the transaction
    ///
    /// # Returns
    /// * `Some(T)` - Successfully decoded event
    /// * `None` - Instruction doesn't match or failed to decode
    fn decode(&self, instruction: &UiInstruction) -> Option<T>;
}
```

**Key Features:**
- **Generic over Event Type `T`**: Developers can implement decoders for any event struct they define
- **Flexible Parsing**: Can use IDL-generated types, Anchor deserialization, or custom byte parsing
- **Multi-Program Support**: Register different decoders for different program IDs
- **Type Safety**: Returns `Option<T>` ensuring compile-time correctness

**Example: SPL Token Transfer Decoder**
```rust
pub struct SplTransferDecoder;

impl InstructionDecoder<TransferEvent> for SplTransferDecoder {
    fn decode(&self, instruction: &UiInstruction) -> Option<TransferEvent> {
        // 1. Check if instruction is for SPL Token program
        // 2. Parse instruction data to identify transfer type
        // 3. Extract accounts and amounts
        // 4. Return TransferEvent if successful
        
        match instruction {
            UiInstruction::Parsed(parsed) => {
                if parsed.program == "spl-token" && parsed.parsed["type"] == "transfer" {
                    Some(TransferEvent {
                        from: parse_pubkey(&parsed.parsed["info"]["source"]),
                        to: parse_pubkey(&parsed.parsed["info"]["destination"]),
                        amount: parsed.parsed["info"]["amount"].as_u64()?,
                        mint: parse_pubkey(&parsed.parsed["info"]["mint"]),
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}
```

**Example: Custom Program Decoder with IDL**
```rust
// Using IDL-generated types
pub struct MyProgramDecoder;

impl InstructionDecoder<DepositEvent> for MyProgramDecoder {
    fn decode(&self, instruction: &UiInstruction) -> Option<DepositEvent> {
        // Use anchor_lang to deserialize instruction data
        let data = instruction.data()?;
        
        // Check instruction discriminator (first 8 bytes)
        if &data[..8] == DEPOSIT_DISCRIMINATOR {
            // Deserialize using Borsh
            DepositEvent::try_from_slice(&data[8..]).ok()
        } else {
            None
        }
    }
}
```

**Registration:**
```rust
SolanaIndexer::new()
    .register_decoder(SPL_TOKEN_PROGRAM_ID, Box::new(SplTransferDecoder))
    .register_decoder(MY_PROGRAM_ID, Box::new(MyProgramDecoder))
    // ... rest of configuration
```

### Extension Point #2: `EventHandler<T>` (Custom Event Processing)

The `EventHandler<T>` trait is where developers define their business logic for processing decoded events.

```rust
#[async_trait]
pub trait EventHandler<T>: Send + Sync + 'static {
    /// Initializes custom database schema for this handler.
    ///
    /// Override this method to create custom tables. Called once
    /// during indexer startup.
    ///
    /// # Default Implementation
    /// Does nothing (no-op).
    async fn initialize_schema(&self, _db: &PgPool) -> Result<()> {
        Ok(())
    }

    /// Handles a decoded event.
    ///
    /// # Arguments
    /// * `event` - The typed event to process
    /// * `db` - Database connection pool
    /// * `signature` - Transaction signature for tracking
    async fn handle(&self, event: T, db: &PgPool, signature: &str) -> Result<()>;
}
```

**Key Features:**
- **Generic over Event Type `T`**: Type-safe handling of specific event structures
- **Schema Initialization**: Optional `initialize_schema()` method for custom database setup
- **Async Processing**: Full async/await support for database operations and API calls
- **Database Access**: Provided `PgPool` for persistence operations
- **Transaction Context**: Access to transaction signature for logging and tracking

**Example: Transfer Event Handler**
```rust
pub struct TransferHandler;

#[async_trait]
impl EventHandler<TransferEvent> for TransferHandler {
    /// Create the transfers table on startup
    async fn initialize_schema(&self, db: &PgPool) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS spl_transfers (
                signature TEXT PRIMARY KEY,
                from_wallet TEXT NOT NULL,
                to_wallet TEXT NOT NULL,
                amount BIGINT NOT NULL,
                mint TEXT NOT NULL,
                indexed_at TIMESTAMPTZ DEFAULT NOW()
            )"
        )
        .execute(db)
        .await?;
        
        Ok(())
    }

    /// Process each transfer event
    async fn handle(
        &self,
        event: TransferEvent,
        db: &PgPool,
        signature: &str,
    ) -> Result<()> {
        println!(
            "Processing Transfer: {} -> {} ({} tokens)",
            event.from, event.to, event.amount
        );

        sqlx::query!(
            "INSERT INTO spl_transfers (signature, from_wallet, to_wallet, amount, mint)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (signature) DO NOTHING",
            signature,
            event.from.to_string(),
            event.to.to_string(),
            event.amount as i64,
            event.mint.to_string()
        )
        .execute(db)
        .await?;

        Ok(())
    }
}
```

**Registration:**
```rust
SolanaIndexer::new()
    .register_handler::<TransferEvent>(TransferHandler)
    .register_handler::<DepositEvent>(DepositHandler)
    // ... rest of configuration
```

### How They Work Together

```
Transaction → InstructionDecoder<T> → Option<T> → EventHandler<T> → Database/API
              (Developer defines      (Typed     (Developer defines
               parsing logic)          Event)     business logic)
```

1.  **SDK fetches transaction** from Solana network
2.  **Decoder Registry routes** instructions to appropriate `InstructionDecoder<T>`
3.  **Developer's decoder** parses raw bytes into typed event `T`
4.  **Handler Registry dispatches** event to appropriate `EventHandler<T>`
5.  **Developer's handler** processes event (database writes, API calls, etc.)
6.  **SDK marks transaction** as processed in idempotency tracker

This separation allows developers to:
- **Reuse decoders** across multiple handlers
- **Test parsing logic** independently from business logic
- **Mix and match** different decoders and handlers
- **Support multiple programs** in a single indexer instance

## 6. Reliability & Error Handling

Reliability is paramount for an indexing solution. SolanaIndexer implements robust mechanisms to ensure data integrity and system stability.

*   **Idempotency:** The `_solana_indexer_processed` table (managed by the `Tracker`) guarantees that each unique transaction is processed only once, preventing duplicate entries or side effects even if the indexer restarts or encounters temporary failures.
*   **Structured `SolanaIndexerError`:**
    SolanaIndexer defines a custom error enumeration, `SolanaIndexerError`, to provide clear, actionable error reporting throughout the SDK. This error type implements `std::error::Error` and `std::fmt::Display`, allowing for seamless integration with Rust's error handling ecosystem and easy human-readable output. For professional Rust libraries, the `thiserror` crate is a highly recommended tool for defining custom error types with rich context and automatic `From` conversions.

    ```rust
    // Assuming 'thiserror' is a dependency in your Cargo.toml
    use thiserror::Error;

    /// Custom error type for SolanaIndexer operations.
    #[derive(Debug, Error)]
    pub enum SolanaIndexerError {
        /// Errors encountered during database operations.
        #[error("Database error: {0}")]
        DatabaseError(#[from] sqlx::Error),
        /// Errors during transaction data or event decoding.
        #[error("Decoding error: {0}")]
        DecodingError(String),
        /// Errors interacting with the Solana RPC.
        #[error("RPC error: {0}")]
        RpcError(String),
        /// Errors related to configuration, e.g., missing environment variables.
        #[error("Configuration error: {0}")]
        ConfigError(#[from] std::env::VarError),
        // Add more specific error types as the SDK evolves.
    }
    ```
*   **Contextual Errors:** When errors occur, SolanaIndexer aims to provide rich contextual information, including the specific operation, input data, and any underlying causes. This significantly aids in debugging and issue resolution, enabling developers to quickly pinpoint and address issues.
*   **Explicit Error Propagation:** All errors propagate up the call stack, ensuring no silent failures. Developers are guided to handle errors appropriately, either by logging, retrying, or gracefully exiting.
*   **Logging:** Detailed logging (e.g., using `tracing` crate) is integrated throughout the SDK to provide visibility into the pipeline's operation, aiding in diagnostics and monitoring.
*   **Retry Logic (Production Roadmap):** For transient failures (e.g., RPC timeouts, temporary network glitches), a production-ready SolanaIndexer will implement configurable retry mechanisms with exponential backoff. This ensures resilience against intermittent issues without requiring constant developer intervention.
*   **Dead-Letter Queues (Production Roadmap):** For events that consistently fail processing after all retries (e.g., due to malformed data or unrecoverable application errors), a dead-letter queue mechanism will be introduced. This prevents such problematic events from blocking the main processing pipeline and allows for manual inspection and reprocessing.
*   **Database Transactions:** For handlers performing multiple database operations, developers are strongly encouraged to wrap their logic in database transactions to ensure atomicity (all or nothing) and data consistency.

## 7. Security Considerations

Security is built into SolanaIndexer's design, protecting both the indexed data and the developer's application.

*   **No Hardcoded Secrets:** Configuration encourages the use of environment variables, preventing sensitive information (database credentials, API keys) from being hardcoded or committed to version control.
*   **Input Validation (Implicit):** IDL-driven decoding inherently provides a form of input validation. Data not conforming to the defined IDL structs will fail deserialization, preventing malformed on-chain data from being incorrectly processed.
*   **Safe Database Interactions:** By promoting `sqlx` and parameterized queries, SolanaIndexer guides developers towards preventing common database vulnerabilities like SQL injection.
*   **Minimal Privileges:** Encourage running the indexer with the minimum necessary permissions for both network and database access.
*   **Open Source Auditability:** As an open-source SDK, the codebase is transparent and auditable by the community, fostering trust and allowing for early detection of potential vulnerabilities.

## 8. Directory Structure

```
solana-indexer/
├── Cargo.toml
├── idl/
│   └── your_program.json          // Developer drops IDL here (e.g., from Anchor)
├── src/
│   ├── lib.rs                     // Public API and main entry point
│   ├── common/                    // Shared utilities and core types
│   │   ├── config.rs              // SolanaIndexerConfig
│   │   ├── error.rs               // SolanaIndexerError enum
│   │   ├── macros.rs              // Procedural macro for IDL compilation
│   │   ├── traits.rs              // EventHandler and SchemaInitializer traits
│   │   ├── types.rs               // Core event types
│   │   └── mod.rs                 // Module exports
│   ├── sources/                   // Input source implementations
│   │   ├── mod.rs
│   │   └── poller.rs              // Poller implementation
│   │   └── websocket.rs           // WebSocket implementation
│   ├── fetcher.rs                 // Logic for fetching full transaction data
│   ├── decoder.rs                 // IDL-driven and generic data parsing
│   ├── indexer.rs                 // Main orchestrator logic
│   ├── storage.rs                 // Database interaction and idempotency
│   └── generator/                 // Transaction generator for testing
└── examples/
    └── token_transfer_indexer.rs  // Demo implementation of an EventHandler
```

### Build Flow:
1.  **Define Program:** Developer writes their Solana program (e.g., using Anchor) and generates its `idl.json`.
2.  **Integrate IDL:** Developer places `idl/your_program.json` into their SolanaIndexer project.
3.  **Generate Types:** Runs `cargo build` → Proc-macro generates structs
4.  **Import Generated Types:** Developer imports generated types: `use solana_indexer::generated::TransferEvent;`
5.  **Implement Handler:** Implements `EventHandler<TransferEvent>` trait
6.  **Configure & Run:** Configures SolanaIndexer using the builder pattern and runs the indexer with `cargo run`

## 9. Developer Quickstart

### Step 1: Setup Configuration

```bash
# .env file (for local development, add to .gitignore)
DATABASE_URL=postgresql://user:pass@localhost:5432/mydb
RPC_URL=http://127.0.0.1:8899
PROGRAM_ID=YourProgramPublicKey111111111111111111111
```

For production, these should be explicit environment variables.

### Step 2: Define Your Event Structure

```rust
use borsh::{BorshSerialize, BorshDeserialize};
use solana_indexer::{EventDiscriminator, calculate_discriminator};

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct TransferEvent {
    pub from: String,
    pub to: String,
    pub amount: u64,
}

impl EventDiscriminator for TransferEvent {
    fn discriminator() -> [u8; 8] {
        calculate_discriminator("TransferEvent")
    }
}
```

### Step 3: Implement Your InstructionDecoder

```rust
use solana_indexer::InstructionDecoder;
use solana_transaction_status::UiInstruction;

pub struct MyDecoder;

impl InstructionDecoder<TransferEvent> for MyDecoder {
    fn decode(&self, instruction: &UiInstruction) -> Option<TransferEvent> {
        // Parse instruction data and return TransferEvent if successful
        // Return None if instruction doesn't match
        todo!("Implement your custom parsing logic")
    }
}
```

### Step 4: Implement Your EventHandler

```rust
use solana_indexer::{EventHandler, SolanaIndexerError};
use solana_indexer::generated::TransferEvent; // Assuming your IDL defines a TransferEvent
use sqlx::PgPool;
use async_trait::async_trait;

/// A sample event handler for `TransferEvent`s.
pub struct MyTransferHandler;

#[async_trait]
impl EventHandler<TransferEvent> for MyTransferHandler {
    /// Handles a `TransferEvent`, prints its details, and persists it to a database.
    ///
    /// # Arguments
    ///
    /// * `event` - The deserialized `TransferEvent` object from the blockchain.
    /// * `db` - A database connection pool for performing persistence operations.
    /// * `signature` - The unique transaction signature associated with this event.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success (`Ok(())`) or a `SolanaIndexerError` if
    /// a database operation fails.
    async fn handle(&self, event: TransferEvent, db: &PgPool, signature: &str)
        -> Result<(), SolanaIndexerError> {
        println!("Processing Transfer: Sig={}, From={}, To={}, Amount={}",
                 signature, event.from, event.to, event.amount);

        // Example: Persist event data to a 'transfers' table in the database.
        // `sqlx::query!` is used for type-safe and injection-resistant queries.
        sqlx::query!(
            "INSERT INTO transfers (signature, from_wallet, to_wallet, amount)
             VALUES ($1, $2, $3, $4)",
            signature, event.from.to_string(), event.to.to_string(), event.amount as i64
        )
        .execute(db)
        .await
        .map_err(SolanaIndexerError::DatabaseError)?; // Map `sqlx::Error` to `SolanaIndexerError::DatabaseError`
        
        Ok(())
    }
}
```

### Step 5: Initialize and Run SolanaIndexer

```rust
use solana_indexer::{EventHandler, SolanaIndexer, SolanaIndexerError};
// Assuming `TransferEvent` is generated by the SolanaIndexer procedural macro
use solana_indexer::generated::TransferEvent; 
use async_trait::async_trait;
use sqlx::PgPool;
use std::env;
use std::error::Error; // Import the standard `Error` trait for main's return type

// Re-using the handler struct from Step 2 for clarity.
// In a real application, this would typically be defined in its own module.
pub struct MyTransferHandler;

#[async_trait]
impl EventHandler<TransferEvent> for MyTransferHandler {
    async fn handle(&self, event: TransferEvent, db: &PgPool, signature: &str)
        -> Result<(), SolanaIndexerError> {
        println!("Processing Transfer: Sig={}, From={}, To={}, Amount={}",
                 signature, event.from, event.to, event.amount);

        sqlx::query!(
            "INSERT INTO transfers (signature, from_wallet, to_wallet, amount)
             VALUES ($1, $2, $3, $4)",
            signature, event.from.to_string(), event.to.to_string(), event.amount as i64
        )
        .execute(db)
        .await
        .map_err(SolanaIndexerError::DatabaseError)?;
        
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Loads environment variables from a `.env` file for local development.
    // This call is optional and will not error if the file is not found,
    // but subsequent `env::var` calls will fail if variables are missing.
    dotenv::dotenv().ok(); 

    // Retrieve configuration from environment variables.
    // Errors from `env::var` are mapped to `SolanaIndexerError::ConfigError`
    // and then boxed for the consistent main function return type.
    let rpc_url = env::var("RPC_URL")
        .map_err(|e| Box::new(SolanaIndexerError::ConfigError(e)) as Box<dyn Error>)?;
    let database_url = env::var("DATABASE_URL")
        .map_err(|e| Box::new(SolanaIndexerError::ConfigError(e)) as Box<dyn Error>)?;
    let program_id_str = env::var("PROGRAM_ID")
        .map_err(|e| Box::new(SolanaIndexerError::ConfigError(e)) as Box<dyn Error>)?;

    // Initialize SolanaIndexer with the provided configuration.
    // Register both the decoder (for parsing) and handler (for processing).
    // The `SolanaIndexer::new()` builder pattern allows for flexible
    // and type-safe configuration.
    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(&rpc_url)
        .with_database(&database_url)
        .program_id(&program_id_str)
        .build()
        .map_err(|e| Box::new(e) as Box<dyn Error>)?;

    // Initialize and run the indexer.
    let mut indexer = SolanaIndexer::new(config).await
        .map_err(|e| Box::new(e) as Box<dyn Error>)?;

    indexer.register_decoder(&program_id_str, Box::new(MyDecoder));
    indexer.register_handler::<TransferEvent>(Box::new(MyTransferHandler));

    indexer.start().await
        .map_err(|e| Box::new(e) as Box<dyn Error>)?;

    println!("SolanaIndexer indexer started successfully.");

    Ok(())
}
```
**That's it! Your indexer is now configured to process `TransferEvent`s for your specified program.**

## 10. Production Roadmap & Future Enhancements

SolanaIndexer is on a continuous path of improvement, with a clear roadmap to enhance its capabilities for production environments.

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
    *   Multi-program indexing capabilities from a single SolanaIndexer instance.
    *   Custom RPC provider support for greater flexibility.
    *   Performance benchmarks and optimizations against existing indexing solutions.
    *   Support for on-chain state change tracking (e.g., account data changes).