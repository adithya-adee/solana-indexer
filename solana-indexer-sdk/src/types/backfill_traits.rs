use crate::config::RegistryConfig;
use crate::core::fetcher::Fetcher;
use crate::core::registry_metrics::RegistryMetrics;
use crate::storage::StorageBackend;
use crate::types::events::EventDiscriminator;
use crate::types::metadata::TxMetadata;
use crate::utils::error::{Result, SolanaIndexerError};
use async_trait::async_trait;
use borsh::BorshDeserialize;
use sqlx::PgPool;
use std::collections::HashMap;

/// Trait for implementing custom backfill strategies.
///
/// This trait allows developers to define how the backfill process determines
/// the range of slots to process, the batch size for fetching signatures, and
/// the concurrency level for processing transactions.
#[async_trait]
pub trait BackfillStrategy: Send + Sync {
    /// Determine the slot range to backfill.
    ///
    /// Returns a tuple of (start_slot, end_slot).
    /// If start_slot is None, it defaults to the earliest available slot (or genesis).
    /// If end_slot is None, it defaults to the latest available slot.
    async fn get_slot_range(
        &self,
        storage: &dyn StorageBackend,
    ) -> Result<(Option<u64>, Option<u64>)>;

    /// Determine batch size for fetching signatures.
    fn batch_size(&self) -> usize;

    /// Determine concurrency level for processing transactions.
    fn concurrency(&self) -> usize;
}

/// Event representing a detected reorganization.
#[derive(Debug, Clone)]
pub struct ReorgEvent {
    pub slot: u64,
    pub previous_hash: String,
    pub new_hash: String,
}

/// Trait for handling blockchain reorganizations.
///
/// This trait defines how the indexer detects and responds to block reorganizations.
#[async_trait]
pub trait ReorgHandler: Send + Sync {
    /// Detect if a reorg has occurred at the given slot.
    ///
    /// Returns `Some(ReorgEvent)` if a reorg is detected, `None` otherwise.
    async fn detect_reorg(
        &self,
        slot: u64,
        storage: &dyn StorageBackend,
        fetcher: &Fetcher,
    ) -> Result<Option<ReorgEvent>>;

    /// Handle a detected reorg by rolling back data and preparing for re-processing.
    async fn handle_reorg(&self, event: ReorgEvent, storage: &dyn StorageBackend) -> Result<()>;
}

/// Trait for tracking finalized blocks.
///
/// This trait is responsible for distinguishing between tentative (confirmed) and
/// finalized blocks, ensuring that only finalized data is permanently stored
/// as immutable.
#[async_trait]
pub trait FinalizedBlockTracker: Send + Sync {
    /// Check if a slot is finalized.
    async fn is_finalized(&self, slot: u64, fetcher: &Fetcher) -> Result<bool>;

    /// Get the latest finalized slot from the chain.
    async fn get_latest_finalized_slot(&self, fetcher: &Fetcher) -> Result<u64>;

    /// Mark a slot as finalized in storage.
    async fn mark_finalized(
        &self,
        slot: u64,
        block_hash: &str,
        storage: &dyn StorageBackend,
    ) -> Result<()>;
}

/// Trait for tracking backfill progress.
///
/// This trait manages the persistence of backfill state, allowing the process
/// to resume from where it left off in case of interruption.
#[async_trait]
pub trait BackfillProgress: Send + Sync {
    /// Save current backfill progress (the last successfully processed slot).
    async fn save_progress(&self, slot: u64, storage: &dyn StorageBackend) -> Result<()>;

    /// Load last backfill progress.
    ///
    /// Returns the last successfully processed slot, or `None` if no progress is saved.
    async fn load_progress(&self, storage: &dyn StorageBackend) -> Result<Option<u64>>;

    /// Mark backfill as complete.
    async fn mark_complete(&self, storage: &dyn StorageBackend) -> Result<()>;
}

/// Represents a range of slots to backfill.
#[derive(Debug, Clone, Copy)]
pub struct BackfillRange {
    pub start_slot: u64,
    pub end_slot: u64,
}

impl BackfillRange {
    /// Creates a new backfill range.
    #[must_use]
    pub fn new(start_slot: u64, end_slot: u64) -> Self {
        Self {
            start_slot,
            end_slot,
        }
    }

    /// Returns the number of slots in this range (inclusive).
    #[must_use]
    pub fn len(&self) -> u64 {
        if self.end_slot >= self.start_slot {
            self.end_slot - self.start_slot + 1
        } else {
            0
        }
    }

    /// Returns true if the range is empty (end < start).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.end_slot < self.start_slot
    }
}

/// Context information for backfill trigger decisions.
#[derive(Debug, Clone)]
pub struct BackfillContext {
    /// Latest finalized slot from the chain.
    pub latest_finalized_slot: u64,
    /// Last slot that was successfully backfilled (if any).
    pub last_backfilled_slot: Option<u64>,
    /// Maximum depth to backfill (slots behind latest finalized).
    pub max_depth: Option<u64>,
    /// Desired lag threshold - only backfill if lag exceeds this.
    pub desired_lag_slots: Option<u64>,
}

impl BackfillContext {
    /// Calculates the current lag (slots behind latest finalized).
    #[must_use]
    pub fn current_lag(&self) -> u64 {
        if let Some(last) = self.last_backfilled_slot {
            self.latest_finalized_slot.saturating_sub(last)
        } else {
            // No backfill progress, assume we need to catch up from genesis
            self.latest_finalized_slot
        }
    }

    /// Returns true if backfill should run based on lag threshold.
    #[must_use]
    pub fn should_backfill_by_lag(&self) -> bool {
        if let Some(threshold) = self.desired_lag_slots {
            self.current_lag() > threshold
        } else {
            true // No threshold set, always backfill if there's lag
        }
    }
}

/// Trait for handling backfilled events (historical data).
///
/// This trait mirrors `EventHandler<T>` but is specifically for events
/// processed during backfill operations. Developers can implement different
/// logic for historical vs live events.
///
/// # Type Parameters
/// * `T` - The event type to handle
///
/// # Example
/// ```no_run
/// use solana_indexer_sdk::{
///     calculate_discriminator,
///     types::backfill_traits::BackfillHandler,
///     EventDiscriminator, Result, TxMetadata,
/// };
/// use borsh::{BorshDeserialize, BorshSerialize};
/// use async_trait::async_trait;
/// use sqlx::PgPool;
///
/// #[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
/// pub struct MyEvent { pub amount: u64 }
///
/// impl EventDiscriminator for MyEvent {
///     fn discriminator() -> [u8; 8] {
///         calculate_discriminator("MyEvent")
///     }
/// }
///
/// pub struct MyBackfillHandler;
///
/// #[async_trait]
/// impl BackfillHandler<MyEvent> for MyBackfillHandler {
///     async fn handle_backfill(
///         &self,
///         event: MyEvent,
///         context: &TxMetadata,
///         db: &PgPool,
///     ) -> Result<()> {
///         // Custom backfill-specific logic
///         println!("Backfilling event: {} at slot {}", event.amount, context.slot);
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait BackfillHandler<T>: Send + Sync + 'static
where
    T: EventDiscriminator + BorshDeserialize + Send + Sync + 'static,
{
    /// Called for each decoded backfill event.
    ///
    /// # Arguments
    /// * `event` - The decoded event object
    /// * `context` - The transaction context (slot, block time, fee, etc.)
    /// * `db` - Database connection pool for persistence operations
    async fn handle_backfill(&self, event: T, context: &TxMetadata, db: &PgPool) -> Result<()>;

    /// Optional hook when a backfill range completes.
    ///
    /// Called after all slots in a `BackfillRange` have been processed.
    /// Useful for batch operations, compaction, or summary statistics.
    ///
    /// # Arguments
    /// * `range` - The completed backfill range
    /// * `db` - Database connection pool
    async fn on_backfill_range_complete(&self, _range: &BackfillRange, _db: &PgPool) -> Result<()> {
        Ok(())
    }

    /// Initializes custom database schema for this backfill handler.
    ///
    /// Called once during indexer startup if backfill is enabled.
    ///
    /// # Arguments
    /// * `db` - Database connection pool
    async fn initialize_backfill_schema(&self, _db: &PgPool) -> Result<()> {
        Ok(())
    }
}

/// Trait for determining when and what to backfill.
///
/// This trait allows developers to define custom logic for deciding:
/// - When backfill should run (e.g., based on lag, time, or custom conditions)
/// - What slot range should be backfilled
///
/// The SDK will call `next_range()` periodically (based on `BackfillConfig.poll_interval_secs`)
/// to determine if backfill work should be scheduled.
///
/// # Example
/// ```no_run
/// use solana_indexer_sdk::types::backfill_traits::{BackfillTrigger, BackfillContext, BackfillRange};
/// use solana_indexer_sdk::{Result, StorageBackend};
/// use async_trait::async_trait;
/// use std::sync::Arc;
///
/// pub struct MyCustomTrigger;
///
/// #[async_trait]
/// impl BackfillTrigger for MyCustomTrigger {
///     async fn next_range(
///         &self,
///         ctx: &BackfillContext,
///         storage: &dyn StorageBackend,
///     ) -> Result<Option<BackfillRange>> {
///         // Only backfill if lag > 1000 slots
///         if ctx.current_lag() > 1000 {
///             let start = ctx.last_backfilled_slot.unwrap_or(0);
///             let end = ctx.latest_finalized_slot.saturating_sub(100); // Keep 100 slot buffer
///             Ok(Some(BackfillRange::new(start, end)))
///         } else {
///             Ok(None)
///         }
///     }
/// }
/// ```
#[async_trait]
pub trait BackfillTrigger: Send + Sync {
    /// Determines the next slot range to backfill, if any.
    ///
    /// Returns `Some(BackfillRange)` if backfill should run, `None` otherwise.
    ///
    /// # Arguments
    /// * `ctx` - Context about current chain state and backfill progress
    /// * `storage` - Storage backend for querying additional state if needed
    ///
    /// # Returns
    /// * `Ok(Some(range))` - Backfill should run for this range
    /// * `Ok(None)` - No backfill needed at this time
    /// * `Err(...)` - Error determining range
    async fn next_range(
        &self,
        ctx: &BackfillContext,
        storage: &dyn StorageBackend,
    ) -> Result<Option<BackfillRange>>;
}

/// Type-erased backfill handler for dynamic dispatch.
#[doc(hidden)]
#[async_trait]
pub trait DynamicBackfillHandler: Send + Sync {
    /// Handles a dynamic backfill event (discriminator + raw bytes).
    async fn handle_backfill_dynamic(
        &self,
        discriminator: &[u8; 8],
        data: &[u8],
        context: &TxMetadata,
        db: &PgPool,
    ) -> Result<()>;

    /// Called when a backfill range completes.
    async fn on_backfill_range_complete_dynamic(
        &self,
        range: &BackfillRange,
        db: &PgPool,
    ) -> Result<()>;

    /// Initializes schema for the dynamic backfill handler.
    async fn initialize_backfill_schema(&self, pool: &PgPool) -> Result<()>;
}

/// Automatic conversion from typed backfill handler to dynamic handler.
#[async_trait]
impl<T> DynamicBackfillHandler for Box<dyn BackfillHandler<T>>
where
    T: EventDiscriminator + BorshDeserialize + Send + Sync + 'static,
{
    async fn handle_backfill_dynamic(
        &self,
        discriminator: &[u8; 8],
        data: &[u8],
        context: &TxMetadata,
        db: &PgPool,
    ) -> Result<()> {
        // Verify discriminator matches
        if *discriminator != T::discriminator() {
            return Err(SolanaIndexerError::DecodingError(
                "Discriminator mismatch".to_string(),
            ));
        }

        // Deserialize event
        let event = T::try_from_slice(data).map_err(|e| {
            SolanaIndexerError::DecodingError(format!(
                "Failed to deserialize backfill event: {}",
                e
            ))
        })?;

        // Delegate to typed handler
        self.handle_backfill(event, context, db).await
    }

    async fn on_backfill_range_complete_dynamic(
        &self,
        range: &BackfillRange,
        db: &PgPool,
    ) -> Result<()> {
        (**self).on_backfill_range_complete(range, db).await
    }

    async fn initialize_backfill_schema(&self, pool: &PgPool) -> Result<()> {
        (**self).initialize_backfill_schema(pool).await
    }
}

/// Registry for managing backfill handlers.
///
/// Similar to `HandlerRegistry` but specifically for backfill operations.
/// Stores handlers keyed by event discriminator and dispatches backfill events.
pub struct BackfillHandlerRegistry {
    /// Map of discriminators to handlers
    handlers: HashMap<[u8; 8], Box<dyn DynamicBackfillHandler>>,
    metrics: RegistryMetrics,
}

impl BackfillHandlerRegistry {
    /// Creates a new empty backfill handler registry with unlimited capacity.
    #[must_use]
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            metrics: RegistryMetrics::new("BackfillHandler", 0),
        }
    }

    /// Creates a new backfill handler registry with a specific capacity limit.
    pub fn new_bounded(config: &RegistryConfig) -> Self {
        Self {
            handlers: HashMap::new(),
            metrics: RegistryMetrics::new("BackfillHandler", config.max_handlers),
        }
    }

    /// Registers a backfill handler for a specific event discriminator.
    ///
    /// # Arguments
    /// * `discriminator` - The 8-byte event discriminator
    /// * `handler` - The handler implementation
    ///
    /// # Errors
    /// Returns `SolanaIndexerError::RegistryCapacityExceeded` if the registry is full.
    pub fn register(
        &mut self,
        discriminator: [u8; 8],
        handler: Box<dyn DynamicBackfillHandler>,
    ) -> Result<()> {
        if !self.handlers.contains_key(&discriminator) && self.metrics.is_full() {
            return Err(SolanaIndexerError::RegistryCapacityExceeded(format!(
                "BackfillHandler registry full (limit: {})",
                self.metrics.capacity_limit
            )));
        }

        self.handlers.insert(discriminator, handler);
        self.metrics.inc_registered();
        Ok(())
    }

    /// Handles a backfill event by dispatching to the appropriate handler.
    ///
    /// # Arguments
    /// * `discriminator` - The event discriminator
    /// * `event_data` - Raw event data
    /// * `context` - The transaction context
    /// * `db` - Database connection pool
    ///
    /// # Errors
    /// Returns `SolanaIndexerError::DecodingError` if no handler is registered
    /// for the discriminator, or propagates handler errors.
    pub async fn handle_backfill(
        &self,
        discriminator: &[u8; 8],
        event_data: &[u8],
        context: &TxMetadata,
        db: &PgPool,
    ) -> Result<()> {
        self.metrics.inc_calls();
        let handler = self.handlers.get(discriminator).ok_or_else(|| {
            SolanaIndexerError::DecodingError(format!(
                "No backfill handler registered for discriminator: {discriminator:?}"
            ))
        })?;

        let result = handler
            .handle_backfill_dynamic(discriminator, event_data, context, db)
            .await;
        if result.is_ok() {
            self.metrics.inc_hits();
        }
        result
    }

    /// Notifies all handlers that a backfill range has completed.
    pub async fn notify_range_complete(&self, range: &BackfillRange, db: &PgPool) -> Result<()> {
        for handler in self.handlers.values() {
            handler
                .on_backfill_range_complete_dynamic(range, db)
                .await?;
        }
        Ok(())
    }

    /// Initializes schemas for all registered backfill handlers.
    pub async fn initialize_schemas(&self, pool: &PgPool) -> Result<()> {
        for handler in self.handlers.values() {
            handler.initialize_backfill_schema(pool).await?;
        }
        Ok(())
    }

    /// Returns the number of registered handlers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    /// Returns true if no handlers are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }

    /// Returns the metrics for this registry.
    pub fn metrics(&self) -> &RegistryMetrics {
        &self.metrics
    }
}

impl Default for BackfillHandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}
