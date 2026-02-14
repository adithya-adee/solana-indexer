//! `SolanaIndexer` - A developer-centric SDK for Solana data indexing.
//!
//! `SolanaIndexer` provides a flexible, type-safe, and extensible platform for indexing
//! Solana blockchain data. It features IDL-driven type generation, multiple input
//! sources (polling and WebSocket), and a clean builder pattern for configuration.
//!
//! # Quick Start
//!
//! ## Basic System Transfer Indexer
//!
//! ```no_run
//! use async_trait::async_trait;
//! use borsh::{BorshDeserialize, BorshSerialize};
//! use solana_indexer_sdk::{
//!     SolanaIndexer, SolanaIndexerConfigBuilder,
//!     InstructionDecoder, EventHandler, EventDiscriminator,
//!     calculate_discriminator, SolanaIndexerError,
//! };
//! use solana_sdk::pubkey::Pubkey;
//! use solana_transaction_status::{UiInstruction, UiParsedInstruction};
//! use sqlx::PgPool;
//!
//! // Define your event structure
//! #[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
//! pub struct TransferEvent {
//!     pub from: Pubkey,
//!     pub to: Pubkey,
//!     pub amount: u64,
//! }
//!
//! impl EventDiscriminator for TransferEvent {
//!     fn discriminator() -> [u8; 8] {
//!         calculate_discriminator("TransferEvent")
//!     }
//! }
//!
//! // Implement decoder
//! pub struct TransferDecoder;
//!
//! impl InstructionDecoder<TransferEvent> for TransferDecoder {
//!     fn decode(&self, instruction: &UiInstruction) -> Option<TransferEvent> {
//!         // Parse instruction and return event
//!         // See examples/system_transfer_indexer.rs for full implementation
//!         None
//!     }
//! }
//!
//! // Implement handler
//! pub struct TransferHandler;
//!
//! #[async_trait]
//! impl EventHandler<TransferEvent> for TransferHandler {
//!     async fn handle(
//!         &self,
//!         event: TransferEvent,
//!         context: &solana_indexer_sdk::TxMetadata,
//!         db: &PgPool,
//!     ) -> Result<(), SolanaIndexerError> {
//!         let _signature = &context.signature;
//!         println!("Transfer: {} -> {} ({} lamports)", event.from, event.to, event.amount);
//!         Ok(())
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     dotenvy::dotenv().ok();
//!
//!     // Build configuration
//!     let config = SolanaIndexerConfigBuilder::new()
//!         .with_rpc(std::env::var("RPC_URL")?)
//!         .with_database(std::env::var("DATABASE_URL")?)
//!         .program_id("11111111111111111111111111111111") // System Program
//!         .with_poll_interval(2)
//!         .with_batch_size(10)
//!         .build()?;
//!
//!     // Create indexer
//!     let mut indexer = SolanaIndexer::new(config).await?;
//!
//!     // Register decoder
//!     indexer.decoder_registry_mut().register(
//!         "system".to_string(),
//!         Box::new(Box::new(TransferDecoder) as Box<dyn InstructionDecoder<TransferEvent>>),
//!     );
//!
//!     // Register handler
//!     let handler: Box<dyn EventHandler<TransferEvent>> = Box::new(TransferHandler);
//!     indexer.handler_registry_mut().register(
//!         TransferEvent::discriminator(),
//!         Box::new(handler),
//!     );
//!
//!     // Start indexing
//!     indexer.start().await?;
//!     Ok(())
//! }
//! ```
//!
//! ## See Also
//!
//! - `examples/system_transfer_indexer.rs` - Complete working example
//! - `examples/spl_token_indexer.rs` - SPL token transfer indexing
//!
//! # Architecture
//!
//! `SolanaIndexer` operates on an event-driven pipeline:
//!
//! 1. **Poller** - Fetches new transaction signatures from RPC
//! 2. **Fetcher** - Retrieves full transaction details with parsed instructions
//! 3. **`DecoderRegistry`** - Routes instructions to appropriate decoders
//! 4. **`InstructionDecoder`** - Parses instructions into typed events
//! 5. **`HandlerRegistry`** - Routes events to appropriate handlers
//! 6. **`EventHandler`** - Processes events (database writes, webhooks, etc.)
//! 7. **Storage** - Tracks processed transactions for idempotency
//!
//! # Features
//!
//! - **Type-Safe Event Handling**: Compile-time guarantees for event structures
//! - **Multi-Program Support**: Index multiple programs simultaneously
//! - **Idempotency**: Automatic deduplication via transaction signatures
//! - **Flexible Decoders**: Custom instruction parsing logic
//! - **Database Integration**: Built-in `PostgreSQL` support with `SQLx`
//! - **Production Ready**: Colorful logging, error recovery, metrics
//!

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
pub use types::events::{
    DepositEvent, EventDiscriminator, EventType, ParsedEvent, TransferEvent, WithdrawEvent,
    calculate_discriminator,
};
pub use types::metadata::{TokenBalanceInfo, TxMetadata};
pub use types::traits::{
    AccountDecoder, DynamicAccountDecoder, DynamicEventHandler, DynamicInstructionDecoder,
    EventHandler, HandlerRegistry, InstructionDecoder, LogDecoder, SchemaInitializer,
};
pub use utils::error::{Result, SolanaIndexerError};
pub use utils::macros::{
    Idl, IdlAccount, IdlAccountItem, IdlEvent, IdlField, IdlInstruction, IdlType,
    IdlTypeDefinition, generate_event_struct, idl_type_to_rust,
};

// Module declarations
pub mod config;
pub mod core;
pub mod storage;
pub mod streams;
pub mod types;
pub mod utils;
