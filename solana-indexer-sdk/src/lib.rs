//! # Solana Indexer SDK
//!
//! A batteries-included, high-performance framework for building custom Solana indexers in Rust.
//!
//! This SDK provides a complete toolkit to subscribe to on-chain data, decode it into
//! structured events, and process it with custom handlers, all with built-in concurrency,
//! error handling, and backfill support.
//!
//! ## Features
//!
//! - **Multiple Data Sources**: Ingest data via RPC polling, WebSocket subscriptions, Helius Enhanced RPC, or Laserstream (Yellowstone gRPC).
//! - **Automatic Indexing Modes**: The indexer automatically detects which on-chain data to process (instructions, logs, or account states) based on the decoders you register.
//! - **Dynamic Backfill**: Automatically detects when the indexer is behind the chain tip and backfills historical data concurrently with live indexing.
//! - **Simple Handler Pattern**: Implement the `EventHandler` trait to add your custom business logic (e.g., writing to a database).
//! - **Concurrent & Performant**: Built on `tokio` to process transactions in parallel, with configurable worker limits.
//!
//! ## Getting Started
//!
//! Here is a complete example of a simple indexer that tracks native SOL transfers.
//!
//! ```no_run
//! use solana_indexer_sdk::{
//!     SolanaIndexer, SolanaIndexerConfigBuilder, InstructionDecoder, EventHandler,
//!     EventDiscriminator, TxMetadata, SolanaIndexerError,
//!     calculate_discriminator,
//! };
//! use solana_sdk::pubkey::Pubkey;
//! use solana_transaction_status::{UiInstruction, UiParsedInstruction};
//! use async_trait::async_trait;
//! use sqlx::PgPool;
//! use borsh::{BorshSerialize, BorshDeserialize};
//!
//! // 1. Define the event you want to track.
//! #[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
//! pub struct SystemTransferEvent {
//!     pub from: Pubkey,
//!     pub to: Pubkey,
//!     pub amount: u64,
//! }
//!
//! // The discriminator helps route the event to the correct handler.
//! impl EventDiscriminator for SystemTransferEvent {
//!     fn discriminator() -> [u8; 8] {
//!         calculate_discriminator("SystemTransferEvent")
//!     }
//! }
//!
//! // 2. Create a decoder to extract your event from a transaction.
//! pub struct SystemTransferDecoder;
//! impl InstructionDecoder<SystemTransferEvent> for SystemTransferDecoder {
//!     fn decode(&self, instruction: &UiInstruction) -> Option<SystemTransferEvent> {
//!         if let UiInstruction::Parsed(UiParsedInstruction::Parsed(parsed)) = instruction {
//!             if parsed.program == "system" && parsed.parsed.get("type")?.as_str()? == "transfer" {
//!                 let info = parsed.parsed.get("info")?.as_object()?;
//!                 return Some(SystemTransferEvent {
//!                     from: info.get("source")?.as_str()?.parse().ok()?,
//!                     to: info.get("destination")?.as_str()?.parse().ok()?,
//!                     amount: info.get("lamports")?.as_u64()?,
//!                 });
//!             }
//!         }
//!         None
//!     }
//! }
//!
//! // 3. Create a handler to process your event.
//! pub struct TransferHandler;
//! #[async_trait]
//! impl EventHandler<SystemTransferEvent> for TransferHandler {
//!     async fn handle(
//!         &self,
//!         event: SystemTransferEvent,
//!         context: &TxMetadata,
//!         db: &PgPool, // This example uses a mock DB; in reality, this is your database pool.
//!     ) -> Result<(), SolanaIndexerError> {
//!         println!(
//!             "✅ Found SOL transfer in tx {}: {} -> {} ({} lamports)",
//!             context.signature, event.from, event.to, event.amount
//!         );
//!         // In a real application, you would insert this into your database.
//!         // Example:
//!         // sqlx::query("INSERT INTO transfers (signature, from, to, amount) VALUES (...)")
//!         //     .execute(db).await?;
//!         Ok(())
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let rpc_url = "https://api.devnet.solana.com";
//!     let database_url = "postgresql://user:pass@localhost:5432/mydb"; // Your database URL
//!
//!     // 4. Configure the indexer.
//!     let config = SolanaIndexerConfigBuilder::new()
//!         .with_rpc(rpc_url)
//!         .with_database(database_url)
//!         .program_id("11111111111111111111111111111111") // System Program
//!         .build()?;
//!
//!     // 5. Create the indexer and register your components.
//!     let mut indexer = SolanaIndexer::new(config).await?;
//!
//!     // Registering the decoder automatically enables instruction (`inputs`) indexing mode.
//!     indexer.register_decoder("system", SystemTransferDecoder)?;
//!
//!     // Register the handler for the event type.
//!     indexer.register_handler(TransferHandler)?;
//!
//!     println!("▶️ Starting indexer... Press Ctrl+C to stop.");
//!
//!     // 6. Start the indexer.
//!     // This will run indefinitely, processing live data and backfilling if needed.
//!     indexer.start().await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Core Concepts
//!
//! - **[`SolanaIndexer`]**: The main orchestrator struct that runs the indexing pipeline.
//!
//! - **[`SolanaIndexerConfigBuilder`]**: A fluent builder for creating the indexer's configuration. This is where you set the RPC URL, database connection, program IDs to watch, and other parameters.
//!
//! - **Decoders**: These are traits you implement to parse on-chain data. The SDK supports three types, and automatically enables the required mode when you register a decoder:
//!   - **[`InstructionDecoder`]**: Parses data from a transaction's instructions.
//!   - **[`LogDecoder`]**: Parses data from program log messages (`sol_log_data`).
//!   - **[`AccountDecoder`]**: Parses the state of an account's data buffer.
//!
//! - **[`EventHandler`]**: The heart of your indexer's business logic. You implement this trait for each of your event types. The `handle` method receives the decoded, typed event and the transaction metadata, allowing you to save the data to a database, send a notification, or perform any other action.
//!
//! - **Dynamic Backfill**: If the indexer detects it is far behind the current state of the blockchain (based on the `desired_lag_slots` config), it will automatically start a background task to fetch and process historical transactions, all while continuing to process live data.

#![warn(clippy::all)]
#![allow(clippy::module_name_repetitions)]

// Public API exports
pub use config::{SolanaIndexerConfig, SolanaIndexerConfigBuilder};
pub use core::account_registry::AccountDecoderRegistry;
pub use core::decoder::{DecodedTransaction, Decoder, InstructionInfo};
pub use core::fetcher::Fetcher;
pub use core::indexer::SolanaIndexer;
pub use core::log_registry::LogDecoderRegistry;
pub use core::registry::DecoderRegistry;
pub use storage::{Storage, StorageBackend};
pub use streams::poller::Poller;
pub use types::backfill_traits::{
    BackfillContext, BackfillHandler, BackfillHandlerRegistry, BackfillProgress, BackfillRange,
    BackfillStrategy, BackfillTrigger, FinalizedBlockTracker, ReorgEvent, ReorgHandler,
};
pub use types::events::{
    calculate_discriminator, DepositEvent, EventDiscriminator, EventType, ParsedEvent,
    TransferEvent, WithdrawEvent,
};
pub use types::metadata::{TokenBalanceInfo, TxMetadata};
pub use types::traits::{
    AccountDecoder, DynamicAccountDecoder, DynamicEventHandler, DynamicInstructionDecoder,
    EventHandler, HandlerRegistry, InstructionDecoder, LogDecoder, SchemaInitializer,
};
pub use utils::error::{Result, SolanaIndexerError};
pub use utils::macros::{
    generate_event_struct, idl_type_to_rust, Idl, IdlAccount, IdlAccountItem, IdlEvent, IdlField,
    IdlInstruction, IdlType, IdlTypeDefinition,
};

// IDL module is available for documentation purposes
// Use solana_idl_parser::generate_sdk_types in build.rs scripts

// Module declarations
pub mod config;
pub mod core;
pub mod idl;
pub mod storage;
pub mod streams;
pub mod types;
pub mod utils;

// Note: Generated types from IDL files should be included in your application code:
// include!(concat!(env!("OUT_DIR"), "/generated_types.rs"));
// This is not included here automatically to avoid compilation errors when IDL_PATH is not set.
