//! Error types for `SolanaIndexer` operations.
//!
//! This module defines a comprehensive error enumeration using `thiserror`
//! to provide clear, actionable error reporting throughout the SDK.

use thiserror::Error;

/// Custom error type for `SolanaIndexer` operations.
///
/// This error type covers all potential failure modes in the `SolanaIndexer` SDK,
/// from configuration issues to runtime failures in RPC communication,
/// database operations, and data decoding.
#[derive(Debug, Error)]
pub enum SolanaIndexerError {
    /// Errors encountered during database operations.
    ///
    /// This variant automatically wraps `sqlx::Error` using the `#[from]` attribute,
    /// allowing seamless error propagation with the `?` operator.
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    /// Errors during transaction data or event decoding.
    ///
    /// This includes failures in parsing IDL-based structures, deserializing
    /// instruction data, or interpreting event logs.
    #[error("Decoding error: {0}")]
    DecodingError(String),

    /// Errors interacting with the Solana RPC.
    ///
    /// This covers network failures, timeout errors, or unexpected responses
    /// from the Solana RPC endpoint.
    #[error("RPC error: {0}")]
    RpcError(String),

    /// Errors related to configuration.
    ///
    /// This includes missing environment variables, invalid configuration values,
    /// or failures in parsing configuration data.
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// Errors from environment variable operations.
    ///
    /// Automatically wraps `std::env::VarError` for convenient error propagation
    /// when reading environment variables.
    #[error("Environment variable error: {0}")]
    EnvVarError(#[from] std::env::VarError),

    /// Errors during Solana public key parsing.
    ///
    /// This occurs when attempting to parse an invalid public key string.
    #[error("Invalid public key: {0}")]
    InvalidPublicKey(#[from] solana_sdk::pubkey::ParsePubkeyError),

    /// Generic errors for operations that don't fit other categories.
    ///
    /// This provides a catch-all for unexpected errors while still maintaining
    /// error context.
    #[error("Internal error: {0}")]
    InternalError(String),

    /// Error when a registry exceeds its configured capacity.
    #[error("Registry capacity exceeded: {0}")]
    RegistryCapacityExceeded(String),
}

/// Type alias for Results using `SolanaIndexerError`.
///
/// This provides a convenient shorthand for functions that return
/// `Result<T, SolanaIndexerError>`.
pub type Result<T> = std::result::Result<T, SolanaIndexerError>;
