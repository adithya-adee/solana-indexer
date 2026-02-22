//! Configuration management for `SolanaIndexer`.
//!
//! This module provides a flexible configuration system using the builder pattern,
//! allowing developers to configure `SolanaIndexer` with type safety and discoverability.

use crate::utils::error::{Result, SolanaIndexerError};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use std::str::FromStr;

const HELIUS_MAINNET_RPC_URL: &str = "https://mainnet.helius-rpc.com/";
const HELIUS_MAINNET_WS_URL: &str = "wss://mainnet.helius-rpc.com/";
const HELIUS_DEVNET_RPC_URL: &str = "https://devnet.helius-rpc.com/";
const HELIUS_DEVNET_WS_URL: &str = "wss://devnet.helius-rpc.com/";

/// Transaction commitment level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CommitmentLevel {
    Processed,
    #[default]
    Confirmed,
    Finalized,
}

impl From<CommitmentLevel> for solana_sdk::commitment_config::CommitmentConfig {
    fn from(level: CommitmentLevel) -> Self {
        match level {
            CommitmentLevel::Processed => {
                solana_sdk::commitment_config::CommitmentConfig::processed()
            }
            CommitmentLevel::Confirmed => {
                solana_sdk::commitment_config::CommitmentConfig::confirmed()
            }
            CommitmentLevel::Finalized => {
                solana_sdk::commitment_config::CommitmentConfig::finalized()
            }
        }
    }
}

/// Configuration for registry memory limits and monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RegistryConfig {
    /// Max number of program IDs in the instruction decoder registry (0 = unlimited).
    pub max_decoder_programs: usize,
    /// Max number of program IDs in the log decoder registry (0 = unlimited).
    pub max_log_decoder_programs: usize,
    /// Max number of account decoders (0 = unlimited).
    pub max_account_decoders: usize,
    /// Max number of event handlers (0 = unlimited).
    pub max_handlers: usize,
    /// Enable runtime metrics logging.
    pub enable_metrics: bool,
}

/// Configuration for `SolanaIndexer` indexer.
///
/// This struct holds all necessary configuration parameters for running
/// a `SolanaIndexer` indexer instance. Use `SolanaIndexerConfigBuilder` to construct
/// instances of this struct.
#[derive(Debug, Clone)]
pub struct SolanaIndexerConfig {
    /// Database connection URL (e.g., <postgresql://user:pass@localhost:5432/db>)
    pub database_url: String,

    /// Program IDs to index transactions for
    pub program_ids: Vec<Pubkey>,

    /// Accounts to decode
    pub accounts_to_decode: Vec<Pubkey>,

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

    /// Desired commitment level for indexing (default: Confirmed)
    pub commitment_level: CommitmentLevel,

    /// Backfill configuration
    pub backfill: BackfillConfig,

    /// Registry configuration (limits and metrics)
    pub registry: RegistryConfig,

    /// Threshold in slots for cleaning up stale tentative transactions (default: 1000)
    pub stale_tentative_threshold: u64,

    /// Number of worker threads for parallel transaction processing (default: 10)
    pub worker_threads: usize,
}

impl SolanaIndexerConfig {
    /// Helper to get the RPC URL regardless of the source type
    #[must_use]
    pub fn rpc_url(&self) -> &str {
        match &self.source {
            SourceConfig::Rpc { rpc_url, .. } => rpc_url,
            #[cfg(feature = "websockets")]
            SourceConfig::WebSocket { rpc_url, .. } | SourceConfig::Hybrid { rpc_url, .. } => {
                rpc_url
            }
            #[cfg(feature = "helius")]
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
            #[cfg(feature = "laserstream")]
            SourceConfig::Laserstream { grpc_url, .. } => grpc_url,
        }
    }

    /// Helper to get the Helius WebSocket URL, if Helius source is configured.
    #[must_use]
    #[cfg(feature = "helius")]
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
            // _ is unreachable when only Rpc and Helius exist, but we still need a fallback for other optional configs
            #[allow(unreachable_patterns)]
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
    #[cfg(feature = "websockets")]
    WebSocket {
        ws_url: String,
        rpc_url: String,
        reconnect_delay_secs: u64,
    },
    /// Helius-specific source
    #[cfg(feature = "helius")]
    Helius {
        api_key: String,
        network: HeliusNetwork,
        use_websocket: bool,
        reconnect_delay_secs: u64,
    },
    /// Hybrid source with WebSocket for real-time and RPC for gap filling
    #[cfg(feature = "websockets")]
    Hybrid {
        ws_url: String,
        rpc_url: String,
        poll_interval_secs: u64,
        reconnect_delay_secs: u64,
        gap_threshold_slots: u64,
    },
    /// Laserstream (Yellowstone gRPC) source
    #[cfg(feature = "laserstream")]
    Laserstream {
        grpc_url: String,
        x_token: Option<String>,
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
///
/// Configures which types of data the indexer should process.
/// Supports multi-selection of options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IndexingMode {
    /// Index based on instruction data (Inputs).
    pub inputs: bool,
    /// Index based on event logs.
    pub logs: bool,
    /// Index accounts.
    pub accounts: bool,
}

impl IndexingMode {
    /// Create a mode computing Inputs only.
    pub fn inputs() -> Self {
        Self {
            inputs: true,
            logs: false,
            accounts: false,
        }
    }

    /// Create a mode computing Logs only.
    pub fn logs() -> Self {
        Self {
            inputs: false,
            logs: true,
            accounts: false,
        }
    }

    /// Create a mode computing Accounts only.
    pub fn accounts() -> Self {
        Self {
            inputs: false,
            logs: false,
            accounts: true,
        }
    }

    /// Create a mode computing All (Inputs + Logs + Accounts).
    pub fn all() -> Self {
        Self {
            inputs: true,
            logs: true,
            accounts: true,
        }
    }
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

/// Configuration for backfill operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackfillConfig {
    /// Enable backfill mode (runs alongside live indexer)
    pub enabled: bool,

    /// Start slot for backfill (None = from genesis/earliest)
    pub start_slot: Option<u64>,

    /// End slot for backfill (None = to latest finalized)
    pub end_slot: Option<u64>,

    /// Batch size for signature fetching
    pub batch_size: usize,

    /// Concurrency level for processing
    pub concurrency: usize,

    /// Enable reorg detection and handling
    pub enable_reorg_handling: bool,

    /// Interval for checking finalized blocks (in slots)
    pub finalization_check_interval: u64,

    /// How often BackfillManager checks for new ranges to backfill (in seconds)
    pub poll_interval_secs: u64,

    /// Maximum depth to backfill (slots behind latest finalized)
    /// If None, backfills all available history
    pub max_depth: Option<u64>,

    /// Desired lag threshold - only backfill if lag exceeds this many slots
    /// If None, backfills whenever there's any lag
    pub desired_lag_slots: Option<u64>,
}

impl Default for BackfillConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            start_slot: None,
            end_slot: None,
            batch_size: 100,
            concurrency: 50,
            enable_reorg_handling: true,
            finalization_check_interval: 32,
            poll_interval_secs: 5,
            max_depth: None,
            desired_lag_slots: Some(1000), // Default: backfill if lag > 1000 slots
        }
    }
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
/// use solana_indexer_sdk::SolanaIndexerConfigBuilder;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let config = SolanaIndexerConfigBuilder::new()
///         .with_rpc("https://api.mainnet-beta.solana.com")
///         .with_database("postgresql://postgres:password@localhost/indexer")
///         .program_id("675k1q2wE7s6L3R29fs6tcMbtFD4vT759Wcx3CY6CSLg")
///         .with_poll_interval(2)
///         .with_batch_size(50)
///         .build()?;
///     Ok(())
/// }
/// ```
#[derive(Debug, Default)]
pub struct SolanaIndexerConfigBuilder {
    database_url: Option<String>,
    program_ids: Option<Vec<String>>,
    accounts_to_decode: Option<Vec<String>>,
    poll_interval_secs: Option<u64>,
    batch_size: Option<usize>,
    source: Option<SourceConfig>,
    start_strategy: Option<StartStrategy>,
    backfill: Option<BackfillConfig>,
    registry: Option<RegistryConfig>,
    stale_tentative_threshold: Option<u64>,
    worker_threads: Option<usize>,
    commitment_level: Option<CommitmentLevel>,
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
    /// # use solana_indexer_sdk::SolanaIndexerConfigBuilder;
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
    /// # use solana_indexer_sdk::SolanaIndexerConfigBuilder;
    /// let builder = SolanaIndexerConfigBuilder::new()
    ///     .with_ws("ws://127.0.0.1:8900", "http://127.0.0.1:8899");
    /// ```
    #[must_use]
    #[cfg(feature = "websockets")]
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
    #[cfg(feature = "helius")]
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
    #[cfg(feature = "helius")]
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

    /// Sets the Laserstream (Yellowstone gRPC) source.
    ///
    /// # Arguments
    ///
    /// * `grpc_url` - The gRPC endpoint URL
    /// * `x_token` - Optional authentication token (e.g., Helius API key)
    #[must_use]
    #[cfg(feature = "laserstream")]
    pub fn with_laserstream(
        mut self,
        grpc_url: impl Into<String>,
        x_token: Option<String>,
    ) -> Self {
        self.source = Some(SourceConfig::Laserstream {
            grpc_url: grpc_url.into(),
            x_token,
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
    /// # use solana_indexer_sdk::SolanaIndexerConfigBuilder;
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
    /// # use solana_indexer_sdk::SolanaIndexerConfigBuilder;
    /// let builder = SolanaIndexerConfigBuilder::new()
    ///     .program_id("YourProgramPublicKey111111111111111111111");
    /// ```
    #[must_use]
    pub fn program_id(mut self, id: impl Into<String>) -> Self {
        let mut ids = self.program_ids.take().unwrap_or_default();
        ids.push(id.into());
        self.program_ids = Some(ids);
        self
    }

    /// Sets the program IDs to index.
    ///
    /// # Arguments
    ///
    /// * `ids` - A vector of program IDs as strings
    #[must_use]
    pub fn program_ids(mut self, ids: Vec<impl Into<String>>) -> Self {
        self.program_ids = Some(ids.into_iter().map(Into::into).collect());
        self
    }

    /// Sets the accounts to decode.
    ///
    /// # Arguments
    ///
    /// * `accounts` - A vector of account public keys as strings
    #[must_use]
    pub fn accounts_to_decode(mut self, accounts: Vec<impl Into<String>>) -> Self {
        self.accounts_to_decode = Some(accounts.into_iter().map(Into::into).collect());
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
    /// # use solana_indexer_sdk::SolanaIndexerConfigBuilder;
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
    /// # use solana_indexer_sdk::SolanaIndexerConfigBuilder;
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
    /// # use solana_indexer_sdk::SolanaIndexerConfigBuilder;
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
    /// # use solana_indexer_sdk::SolanaIndexerConfigBuilder;
    /// # use solana_indexer_sdk::config::StartStrategy;
    /// let builder = SolanaIndexerConfigBuilder::new()
    ///     .with_start_strategy(StartStrategy::Resume);
    /// ```
    #[must_use]
    pub fn with_start_strategy(mut self, strategy: StartStrategy) -> Self {
        self.start_strategy = Some(strategy);
        self
    }

    /// Sets the backfill configuration.
    #[must_use]
    pub fn with_backfill(mut self, config: BackfillConfig) -> Self {
        self.backfill = Some(config);
        self
    }

    /// Sets the registry configuration.
    #[must_use]
    pub fn with_registry_config(mut self, config: RegistryConfig) -> Self {
        self.registry = Some(config);
        self
    }

    /// Sets the stale tentative transaction threshold in slots.
    ///
    /// # Arguments
    ///
    /// * `threshold` - Number of slots to wait before considering a tentative transaction stale
    #[must_use]
    pub fn with_stale_tentative_threshold(mut self, threshold: u64) -> Self {
        self.stale_tentative_threshold = Some(threshold);
        self
    }

    /// Sets the number of worker threads for parallel transaction processing.
    ///
    /// # Arguments
    ///
    /// Sets the number of worker threads for parallel fetching.
    pub fn with_worker_threads(mut self, threads: usize) -> Self {
        self.worker_threads = Some(threads); // Assuming builder still uses Option<usize>
        self
    }

    /// Sets the commitment level for indexing.
    #[must_use]
    pub fn with_commitment(mut self, level: CommitmentLevel) -> Self {
        self.commitment_level = Some(level);
        self
    }

    /// Builds and validates the configuration.
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::ConfigError` if:
    /// Set the source to a Hybrid configuration (WebSocket + RPC polling).
    #[cfg(feature = "websockets")]
    pub fn with_hybrid(
        mut self,
        ws_url: impl Into<String>,
        rpc_url: impl Into<String>,
        poll_interval_secs: u64,
        reconnect_delay_secs: u64,
        gap_threshold_slots: u64,
    ) -> Self {
        self.source = Some(SourceConfig::Hybrid {
            ws_url: ws_url.into(),
            rpc_url: rpc_url.into(),
            poll_interval_secs,
            reconnect_delay_secs,
            gap_threshold_slots,
        });
        self
    }

    /// Build the configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Any required field (RPC URL, database URL, or program ID) is missing
    /// - The program ID cannot be parsed into a valid `Pubkey`
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer_sdk::SolanaIndexerConfigBuilder;
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

        let program_id_strs = self.program_ids.ok_or_else(|| {
            SolanaIndexerError::ConfigError("Program IDs are required".to_string())
        })?;

        let program_ids = program_id_strs
            .into_iter()
            .map(|s| {
                Pubkey::from_str(&s).map_err(|e| {
                    SolanaIndexerError::ConfigError(format!("Invalid program ID '{s}': {e}"))
                })
            })
            .collect::<Result<Vec<Pubkey>>>()?;

        let accounts_to_decode_strs = self.accounts_to_decode.unwrap_or_default();
        let accounts_to_decode = accounts_to_decode_strs
            .into_iter()
            .map(|s| {
                Pubkey::from_str(&s).map_err(|e| {
                    SolanaIndexerError::ConfigError(format!(
                        "Invalid account public key '{s}': {e}"
                    ))
                })
            })
            .collect::<Result<Vec<Pubkey>>>()?;

        let poll_interval_secs = self.poll_interval_secs.unwrap_or(5);
        let batch_size = self.batch_size.unwrap_or(100);

        // If source is not set, error out
        let source = self.source.ok_or_else(|| {
             SolanaIndexerError::ConfigError("Source configuration (RPC or WebSocket) is required. Use .with_rpc() or .with_ws()".to_string())
        })?;

        Ok(SolanaIndexerConfig {
            database_url,
            program_ids,
            accounts_to_decode,
            poll_interval_secs,
            batch_size,
            source,
            indexing_mode: IndexingMode::default(),
            start_strategy: self.start_strategy.unwrap_or_default(),
            backfill: self.backfill.unwrap_or_default(),
            registry: self.registry.unwrap_or_default(),
            stale_tentative_threshold: self.stale_tentative_threshold.unwrap_or(1000),
            worker_threads: self.worker_threads.unwrap_or(10),
            commitment_level: self.commitment_level.unwrap_or_default(),
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
    fn test_builder_defaults() -> Result<()> {
        let config = SolanaIndexerConfigBuilder::new()
            .with_rpc("http://127.0.0.1:8899")
            .with_database("postgresql://localhost/db")
            .program_id("11111111111111111111111111111111")
            .build()?;

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
            #[allow(unreachable_patterns)]
            _ => panic!("Expected RPC source"),
        }
        Ok(())
    }

    #[test]
    #[cfg(feature = "websockets")]
    fn test_builder_websocket_config() -> Result<()> {
        let config = SolanaIndexerConfigBuilder::new()
            .with_ws("ws://127.0.0.1:8900", "http://127.0.0.1:8899")
            .with_database("postgresql://localhost/db")
            .program_id("11111111111111111111111111111111")
            .build()?;

        match config.source {
            SourceConfig::WebSocket {
                ws_url, rpc_url, ..
            } => {
                assert_eq!(ws_url, "ws://127.0.0.1:8900");
                assert_eq!(rpc_url, "http://127.0.0.1:8899");
            }
            _ => panic!("Expected WebSocket source"),
        }
        Ok(())
    }

    #[test]
    #[cfg(feature = "helius")]
    fn test_builder_helius_config() -> Result<()> {
        let config = SolanaIndexerConfigBuilder::new()
            .with_helius("test-api-key", true)
            .with_database("postgresql://localhost/db")
            .program_id("11111111111111111111111111111111")
            .build()?;

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
        Ok(())
    }

    #[test]
    #[cfg(feature = "helius")]
    fn test_builder_helius_network_config() -> Result<()> {
        let config = SolanaIndexerConfigBuilder::new()
            .with_helius_network("test-api-key", HeliusNetwork::Devnet, true)
            .with_database("postgresql://localhost/db")
            .program_id("11111111111111111111111111111111")
            .build()?;

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
        Ok(())
    }
}
