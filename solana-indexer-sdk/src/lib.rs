#![doc = include_str!("../README.md")]
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

// Module declarations
pub mod config;
pub mod core;
pub mod storage;
pub mod streams;
pub mod types;
pub mod utils;
