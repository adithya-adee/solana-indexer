//! Configuration management for `SolanaIndexer`.
//!
//! This module provides a flexible configuration system using the builder pattern,
//! allowing developers to configure `SolanaIndexer` with type safety and discoverability.

use crate::utils::error::{Result, SolanaIndexerError};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use std::str::FromStr;

const HELIUS_MAINNET_RPC_URL: &str = "https://mainnet.helius-rpc.com/";
const HELIUS_MAINNET_WS_URL: &str = "wss://mainnet.helius-rpc.com/";
const HELIUS_DEVNET_RPC_URL: &str = "https://devnet.helius-rpc.com/";
const HELIUS_DEVNET_WS_URL: &str = "wss://devnet.helius-rpc.com/";

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

    /// Indexing mode (Inputs, Logs, or All)
    pub indexing_mode: IndexingMode,

    /// Strategy for determining where to start indexing from
    pub start_strategy: StartStrategy,
}

impl SolanaIndexerConfig {
    /// Helper to get the RPC URL regardless of the source type
    #[must_use]
    pub fn rpc_url(&self) -> &str {
        match &self.source {
            SourceConfig::Rpc { rpc_url, .. } | SourceConfig::WebSocket { rpc_url, .. } => rpc_url,
            SourceConfig::Helius {
                api_key, network, ..
            } => {
                // Dynamically construct Helius RPC URL
                let base_url = match network {
                    HeliusNetwork::Mainnet => HELIUS_MAINNET_RPC_URL,
                    HeliusNetwork::Devnet => HELIUS_DEVNET_RPC_URL,
                };
                Box::leak(format!("{base_url}?api-key={api_key}").into_boxed_str())
            }
        }
    }

    /// Helper to get the Helius WebSocket URL, if Helius source is configured.
    #[must_use]
    pub fn helius_ws_url(&self) -> Option<&str> {
        match &self.source {
            SourceConfig::Helius {
                api_key, network, ..
            } => {
                // Dynamically construct Helius WebSocket URL
                let base_url = match network {
                    HeliusNetwork::Mainnet => HELIUS_MAINNET_WS_URL,
                    HeliusNetwork::Devnet => HELIUS_DEVNET_WS_URL,
                };
                Some(Box::leak(
                    format!("{base_url}?api-key={api_key}").into_boxed_str(),
                ))
            }
            _ => None,
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
    /// Helius-specific source
    Helius {
        api_key: String,
        network: HeliusNetwork,
        use_websocket: bool,
        reconnect_delay_secs: u64,
    },
}

/// Network selection for Helius.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeliusNetwork {
    /// Solana Mainnet Beta
    #[default]
    Mainnet,
    /// Solana Devnet
    Devnet,
}

/// Mode of indexing operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IndexingMode {
    /// Index based on instruction data (default).
    #[default]
    Inputs,
    /// Index based on event logs.
    Logs,
    /// Index both inputs and logs.
    All,
}

/// Strategy for determining where to start indexing from.
#[derive(Debug, Clone, Default)]
pub enum StartStrategy {
    /// Fetch the most recent signature from RPC at startup (start from "now").
    #[default]
    Latest,
    /// Start from a specific transaction signature (manual backfill).
    Signature(Signature),
    /// Resume from the last processed signature in the database (prevents gaps).
    Resume,
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
    indexing_mode: Option<IndexingMode>,
    start_strategy: Option<StartStrategy>,
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

    /// Sets the Helius source.
    ///
    /// # Arguments
    ///
    /// * `api_key` - The Helius API key
    /// * `use_websocket` - Whether to use WebSocket (true) or RPC polling only (false)
    #[must_use]
    pub fn with_helius(mut self, api_key: impl Into<String>, use_websocket: bool) -> Self {
        self.source = Some(SourceConfig::Helius {
            api_key: api_key.into(),
            network: HeliusNetwork::Mainnet,
            use_websocket,
            reconnect_delay_secs: 5,
        });
        self
    }

    /// Sets the Helius source with a specific network.
    ///
    /// # Arguments
    ///
    /// * `api_key` - The Helius API key
    /// * `network` - The Helius network (Mainnet or Devnet)
    /// * `use_websocket` - Whether to use WebSocket (true) or RPC polling only (false)
    #[must_use]
    pub fn with_helius_network(
        mut self,
        api_key: impl Into<String>,
        network: HeliusNetwork,
        use_websocket: bool,
    ) -> Self {
        self.source = Some(SourceConfig::Helius {
            api_key: api_key.into(),
            network,
            use_websocket,
            reconnect_delay_secs: 5,
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

    /// Sets the starting signature for indexing (manual backfill).
    ///
    /// This sets the strategy to `StartStrategy::Signature`.
    ///
    /// # Arguments
    ///
    /// * `signature` - The transaction signature to start from
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::SolanaIndexerConfigBuilder;
    /// let builder = SolanaIndexerConfigBuilder::new()
    ///     .with_start_signature("5j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia7");
    /// ```
    #[must_use]
    pub fn with_start_signature(mut self, signature: impl Into<String>) -> Self {
        let sig_str = signature.into();
        if let Ok(sig) = Signature::from_str(&sig_str) {
            self.start_strategy = Some(StartStrategy::Signature(sig));
        }
        self
    }

    /// Sets the start strategy for the indexer.
    ///
    /// # Arguments
    ///
    /// * `strategy` - The resumption strategy to use
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::SolanaIndexerConfigBuilder;
    /// # use solana_indexer::config::StartStrategy;
    /// let builder = SolanaIndexerConfigBuilder::new()
    ///     .with_start_strategy(StartStrategy::Resume);
    /// ```
    #[must_use]
    pub fn with_start_strategy(mut self, strategy: StartStrategy) -> Self {
        self.start_strategy = Some(strategy);
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

        // If source is not set, error out
        let source = self.source.ok_or_else(|| {
             SolanaIndexerError::ConfigError("Source configuration (RPC or WebSocket) is required. Use .with_rpc() or .with_ws()".to_string())
        })?;

        Ok(SolanaIndexerConfig {
            database_url,
            program_id,
            poll_interval_secs,
            batch_size,
            source,
            indexing_mode: self.indexing_mode.unwrap_or_default(),
            start_strategy: self.start_strategy.unwrap_or_default(),
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

    #[test]
    fn test_builder_helius_config() {
        let config = SolanaIndexerConfigBuilder::new()
            .with_helius("test-api-key", true)
            .with_database("postgresql://localhost/db")
            .program_id("11111111111111111111111111111111")
            .build()
            .unwrap();

        assert_eq!(
            config.rpc_url(),
            "https://mainnet.helius-rpc.com/?api-key=test-api-key"
        );
        assert_eq!(
            config.helius_ws_url(),
            Some("wss://mainnet.helius-rpc.com/?api-key=test-api-key")
        );

        match config.source {
            SourceConfig::Helius {
                api_key,
                network,
                use_websocket,
                reconnect_delay_secs,
            } => {
                assert_eq!(api_key, "test-api-key");
                assert_eq!(network, HeliusNetwork::Mainnet);
                assert!(use_websocket);
                assert_eq!(reconnect_delay_secs, 5);
            }
            _ => panic!("Expected Helius source"),
        }
    }

    #[test]
    fn test_builder_helius_devnet_config() {
        let config = SolanaIndexerConfigBuilder::new()
            .with_helius_network("test-api-key", HeliusNetwork::Devnet, true)
            .with_database("postgresql://localhost/db")
            .program_id("11111111111111111111111111111111")
            .build()
            .unwrap();

        assert_eq!(
            config.rpc_url(),
            "https://devnet.helius-rpc.com/?api-key=test-api-key"
        );
        assert_eq!(
            config.helius_ws_url(),
            Some("wss://devnet.helius-rpc.com/?api-key=test-api-key")
        );

        match config.source {
            SourceConfig::Helius {
                api_key,
                network,
                use_websocket,
                ..
            } => {
                assert_eq!(api_key, "test-api-key");
                assert_eq!(network, HeliusNetwork::Devnet);
                assert!(use_websocket);
            }
            _ => panic!("Expected Helius source"),
        }
    }
}
