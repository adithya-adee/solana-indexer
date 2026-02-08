//! Configuration management for `SolanaIndexer`.
//!
//! This module provides a flexible configuration system using the builder pattern,
//! allowing developers to configure `SolanaIndexer` with type safety and discoverability.

use crate::utils::error::{Result, SolanaIndexerError};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

/// Configuration for `SolanaIndexer` indexer.
///
/// This struct holds all necessary configuration parameters for running
/// a `SolanaIndexer` indexer instance. Use `SolanaIndexerConfigBuilder` to construct
/// instances of this struct.
#[derive(Debug, Clone)]
pub struct SolanaIndexerConfig {
    /// Database connection URL (e.g., <postgresql://user:pass@localhost:5432/db>)
    pub database_url: String,

    /// Program ID to index transactions for
    pub program_id: Pubkey,

    /// Polling interval in seconds (default: 5)
    pub poll_interval_secs: u64,

    /// Batch size for fetching transactions (default: 100)
    pub batch_size: usize,

    /// Source configuration
    pub source: SourceConfig,
}

impl SolanaIndexerConfig {
    /// Helper to get the RPC URL regardless of the source type
    pub fn rpc_url(&self) -> &str {
        match &self.source {
            SourceConfig::Rpc { rpc_url, .. } => rpc_url,
            SourceConfig::WebSocket { rpc_url, .. } => rpc_url,
            SourceConfig::Helius { rpc_url, .. } => rpc_url,
        }
    }
}

/// Configuration for the data source
#[derive(Debug, Clone)]
pub enum SourceConfig {
    /// Standard RPC polling source
    Rpc {
        rpc_url: String,
        poll_interval_secs: u64,
        batch_size: usize,
    },
    /// WebSocket source with RPC fallback
    WebSocket {
        ws_url: String,
        rpc_url: String,
        reconnect_delay_secs: u64,
    },
    /// Helius-specific source (future proofing)
    Helius { api_key: String, rpc_url: String },
}

/// Builder for `SolanaIndexerConfig`.
///
/// This builder provides a fluent API for constructing `SolanaIndexerConfig` instances
/// with type safety and validation. All required fields must be set before calling
/// `build()`.
///
/// # Example
///
/// ```no_run
/// use solana_indexer::SolanaIndexerConfigBuilder;
/// use std::env;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = SolanaIndexerConfigBuilder::new()
///     .with_rpc(env::var("RPC_URL")?)
///     .with_database(env::var("DATABASE_URL")?)
///     .program_id(env::var("PROGRAM_ID")?)
///     .with_poll_interval(10)
///     .build()?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Default)]
pub struct SolanaIndexerConfigBuilder {
    database_url: Option<String>,
    program_id: Option<String>,
    poll_interval_secs: Option<u64>,
    batch_size: Option<usize>,
    source: Option<SourceConfig>,
}

impl SolanaIndexerConfigBuilder {
    /// Creates a new configuration builder with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the Solana RPC endpoint URL.
    ///
    /// # Arguments
    ///
    /// * `url` - The RPC endpoint URL (e.g., <http://127.0.0.1:8899>)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::SolanaIndexerConfigBuilder;
    /// let builder = SolanaIndexerConfigBuilder::new()
    ///     .with_rpc("http://127.0.0.1:8899");
    /// ```
    #[must_use]
    pub fn with_rpc(mut self, url: impl Into<String>) -> Self {
        let url = url.into();
        self.source = Some(SourceConfig::Rpc {
            rpc_url: url,
            poll_interval_secs: self.poll_interval_secs.unwrap_or(5),
            batch_size: self.batch_size.unwrap_or(100),
        });
        self
    }

    /// Sets the WebSocket source.
    ///
    /// # Arguments
    ///
    /// * `ws_url` - The WebSocket URL
    /// * `rpc_url` - The RPC URL for fetching full transactions
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::SolanaIndexerConfigBuilder;
    /// let builder = SolanaIndexerConfigBuilder::new()
    ///     .with_ws("ws://127.0.0.1:8900", "http://127.0.0.1:8899");
    /// ```
    #[must_use]
    pub fn with_ws(mut self, ws_url: impl Into<String>, rpc_url: impl Into<String>) -> Self {
        self.source = Some(SourceConfig::WebSocket {
            ws_url: ws_url.into(),
            rpc_url: rpc_url.into(),
            reconnect_delay_secs: 5, // Default
        });
        self
    }

    /// Sets the database connection URL.
    ///
    /// # Arguments
    ///
    /// * `url` - The database connection URL (e.g., <postgresql://user:pass@localhost:5432/db>)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::SolanaIndexerConfigBuilder;
    /// let builder = SolanaIndexerConfigBuilder::new()
    ///     .with_database("postgresql://user:pass@localhost:5432/mydb");
    /// ```
    #[must_use]
    pub fn with_database(mut self, url: impl Into<String>) -> Self {
        self.database_url = Some(url.into());
        self
    }

    /// Sets the program ID to index.
    ///
    /// # Arguments
    ///
    /// * `id` - The program ID as a string (will be parsed into a `Pubkey`)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::SolanaIndexerConfigBuilder;
    /// let builder = SolanaIndexerConfigBuilder::new()
    ///     .program_id("YourProgramPublicKey111111111111111111111");
    /// ```
    #[must_use]
    pub fn program_id(mut self, id: impl Into<String>) -> Self {
        self.program_id = Some(id.into());
        self
    }

    /// Sets the polling interval in seconds.
    ///
    /// # Arguments
    ///
    /// * `secs` - Polling interval in seconds (default: 5)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::SolanaIndexerConfigBuilder;
    /// let builder = SolanaIndexerConfigBuilder::new()
    ///     .with_poll_interval(10);
    /// ```
    #[must_use]
    pub fn with_poll_interval(mut self, secs: u64) -> Self {
        self.poll_interval_secs = Some(secs);
        // Update source if it's already set to Rpc
        if let Some(SourceConfig::Rpc {
            rpc_url,
            batch_size,
            ..
        }) = &self.source
        {
            self.source = Some(SourceConfig::Rpc {
                rpc_url: rpc_url.clone(),
                poll_interval_secs: secs,
                batch_size: *batch_size,
            });
        }
        self
    }

    /// Sets the batch size for fetching transactions.
    ///
    /// # Arguments
    ///
    /// * `size` - Number of transactions to fetch per batch (default: 100)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::SolanaIndexerConfigBuilder;
    /// let builder = SolanaIndexerConfigBuilder::new()
    ///     .with_batch_size(50);
    /// ```
    #[must_use]
    pub fn with_batch_size(mut self, size: usize) -> Self {
        self.batch_size = Some(size);
        // Update source if it's already set to Rpc
        if let Some(SourceConfig::Rpc {
            rpc_url,
            poll_interval_secs,
            ..
        }) = &self.source
        {
            self.source = Some(SourceConfig::Rpc {
                rpc_url: rpc_url.clone(),
                poll_interval_secs: *poll_interval_secs,
                batch_size: size,
            });
        }
        self
    }

    /// Builds and validates the configuration.
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::ConfigError` if:
    /// - Any required field (RPC URL, database URL, or program ID) is missing
    /// - The program ID cannot be parsed into a valid `Pubkey`
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::SolanaIndexerConfigBuilder;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = SolanaIndexerConfigBuilder::new()
    ///     .with_rpc("http://127.0.0.1:8899")
    ///     .with_database("postgresql://user:pass@localhost:5432/db")
    ///     .program_id("YourProgramPublicKey111111111111111111111")
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn build(self) -> Result<SolanaIndexerConfig> {
        let database_url = self.database_url.ok_or_else(|| {
            SolanaIndexerError::ConfigError("Database URL is required".to_string())
        })?;

        let program_id_str = self
            .program_id
            .ok_or_else(|| SolanaIndexerError::ConfigError("Program ID is required".to_string()))?;

        let program_id = Pubkey::from_str(&program_id_str).map_err(|e| {
            SolanaIndexerError::ConfigError(format!("Invalid program ID '{program_id_str}': {e}"))
        })?;

        let poll_interval_secs = self.poll_interval_secs.unwrap_or(5);
        let batch_size = self.batch_size.unwrap_or(100);

        // If source is not set, error out (or default? user said remove rpc_url, so we force setting it via with_rpc or with_ws)
        let source = self.source.ok_or_else(|| {
             SolanaIndexerError::ConfigError("Source configuration (RPC or WebSocket) is required. Use .with_rpc() or .with_ws()".to_string())
        })?;

        Ok(SolanaIndexerConfig {
            database_url,
            program_id,
            poll_interval_secs,
            batch_size,
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_missing_required_fields() {
        let result = SolanaIndexerConfigBuilder::new().build();
        assert!(result.is_err());
    }

    #[test]
    fn test_builder_invalid_program_id() {
        let result = SolanaIndexerConfigBuilder::new()
            .with_rpc("http://127.0.0.1:8899")
            .with_database("postgresql://localhost/db")
            .program_id("invalid_pubkey")
            .build();

        assert!(result.is_err());
        if let Err(SolanaIndexerError::ConfigError(msg)) = result {
            assert!(msg.contains("Invalid program ID"));
        }
    }

    #[test]
    fn test_builder_defaults() {
        let config = SolanaIndexerConfigBuilder::new()
            .with_rpc("http://127.0.0.1:8899")
            .with_database("postgresql://localhost/db")
            .program_id("11111111111111111111111111111111")
            .build()
            .unwrap();

        assert_eq!(config.poll_interval_secs, 5);
        assert_eq!(config.batch_size, 100);

        match config.source {
            SourceConfig::Rpc {
                rpc_url,
                poll_interval_secs,
                batch_size,
            } => {
                assert_eq!(rpc_url, "http://127.0.0.1:8899");
                assert_eq!(poll_interval_secs, 5);
                assert_eq!(batch_size, 100);
            }
            _ => panic!("Expected RPC source"),
        }
    }

    #[test]
    fn test_builder_websocket_config() {
        let config = SolanaIndexerConfigBuilder::new()
            .with_ws("ws://127.0.0.1:8900", "http://127.0.0.1:8899")
            .with_database("postgresql://localhost/db")
            .program_id("11111111111111111111111111111111")
            .build()
            .unwrap();

        match config.source {
            SourceConfig::WebSocket {
                ws_url, rpc_url, ..
            } => {
                assert_eq!(ws_url, "ws://127.0.0.1:8900");
                assert_eq!(rpc_url, "http://127.0.0.1:8899");
            }
            _ => panic!("Expected WebSocket source"),
        }
    }
}
