//! `SolanaIndexer` - A developer-centric SDK for Solana data indexing.
//!
//! `SolanaIndexer` provides a flexible, type-safe, and extensible platform for indexing
//! Solana blockchain data. It features IDL-driven type generation, multiple input
//! sources (polling and WebSocket), and a clean builder pattern for configuration.
//!
//! # Quick Start
//!
//! ```no_run
//! use solana_indexer::{SolanaIndexerConfigBuilder, Poller};
//! use std::env;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Load environment variables
//!     dotenvy::dotenv().ok();
//!
//!     // Configure SolanaIndexer
//!     let config = SolanaIndexerConfigBuilder::new()
//!         .with_rpc(env::var("RPC_URL")?)
//!         .with_database(env::var("DATABASE_URL")?)
//!         .program_id(env::var("PROGRAM_ID")?)
//!         .build()?;
//!
//!     // Create and start poller
//!     let mut poller = Poller::new(config);
//!     poller.start().await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! # Architecture
//!
//! `SolanaIndexer` operates on an event-driven pipeline:
//!
//! 1. **Input Source** - Acquires transaction signatures (Poller or WebSocket)
//! 2. **Fetcher** - Retrieves full transaction details
//! 3. **Decoder** - Parses transactions using IDL-generated types
//! 4. **Idempotency Tracker** - Prevents duplicate processing
//! 5. **Handler** - Executes custom business logic
//! 6. **Persistence** - Marks transactions as processed
//!
//! # Features
//!
//! - **Type-Safe Configuration**: Builder pattern with compile-time validation
//! - **IDL-Driven Development**: Automatic Rust type generation from Solana IDLs
//! - **Flexible Input Sources**: Polling for localnet, WebSocket for production
//! - **Comprehensive Error Handling**: Rich error types with context
//! - **Environment Variable Integration**: Seamless `.env` file support

#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

// Public API exports
pub use config::{SolanaIndexerConfig, SolanaIndexerConfigBuilder};
pub use decoder::{Decoder, DecodedTransaction, EventType, InstructionInfo, ParsedEvent};
pub use error::{Result, SolanaIndexerError};
pub use fetcher::Fetcher;
pub use indexer::SolanaIndexer;
pub use macros::{
    generate_event_struct, idl_type_to_rust, Idl, IdlAccount, IdlAccountItem, IdlEvent,
    IdlField, IdlInstruction, IdlType, IdlTypeDefinition,
};
pub use sources::poller::Poller;
pub use storage::Storage;
pub use traits::{DynamicEventHandler, EventHandler, HandlerRegistry};
pub use types::{
    calculate_discriminator, DepositEvent, EventDiscriminator, TransferEvent, WithdrawEvent,
};

// Module declarations
pub mod config;
pub mod decoder;
pub mod error;
pub mod fetcher;
pub mod indexer;
pub mod macros;
pub mod sources;
pub mod storage;
pub mod traits;
pub mod types;
