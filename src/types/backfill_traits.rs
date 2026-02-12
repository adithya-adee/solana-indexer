use crate::core::fetcher::Fetcher;
use crate::storage::StorageBackend;
use crate::utils::error::Result;
use async_trait::async_trait;

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
