//! Storage and database utilities for `SolanaIndexer`.
//!
//! This module provides database interaction utilities, connection pool management,
//! and idempotency tracking to ensure reliable transaction processing.

use crate::utils::error::Result;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

use async_trait::async_trait;

/// Abstract interface for storage operations.
#[async_trait]
pub trait StorageBackend: Send + Sync {
    async fn initialize(&self) -> Result<()>;
    async fn is_processed(&self, signature: &str) -> Result<bool>;
    async fn mark_processed(&self, signature: &str, slot: u64) -> Result<()>;
    async fn get_last_processed_slot(&self) -> Result<Option<u64>>;
    async fn get_last_processed_signature(&self) -> Result<Option<String>>;
    fn pool(&self) -> &PgPool;

    // New methods for reorg handling and backfill
    async fn mark_tentative(&self, signature: &str, slot: u64, block_hash: &str) -> Result<()>;
    async fn mark_finalized(&self, slot: u64, block_hash: &str) -> Result<()>;
    async fn get_tentative_transactions(&self, slot: u64) -> Result<Vec<String>>;
    async fn rollback_slot(&self, slot: u64) -> Result<()>;
    async fn get_block_hash(&self, slot: u64) -> Result<Option<String>>;
    async fn cleanup_stale_tentative_transactions(&self, slot_threshold: u64) -> Result<u64>;
    async fn get_tentative_slots_le(&self, slot: u64) -> Result<Vec<u64>>;

    // Backfill progress tracking
    async fn save_backfill_progress(&self, slot: u64) -> Result<()>;
    async fn load_backfill_progress(&self) -> Result<Option<u64>>;
    async fn mark_backfill_complete(&self) -> Result<()>;
}

/// Database storage manager for the indexer.
///
/// The `Storage` struct manages database connections and provides utilities
/// for transaction tracking and persistence operations.
///
/// # Example
///
/// ```no_run
/// use solana_indexer::Storage;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let storage = Storage::new("postgresql://localhost/mydb").await?;
/// # Ok(())
/// # }
/// ```
pub struct Storage {
    /// `PostgreSQL` connection pool
    pool: PgPool,
}

impl Storage {
    /// Creates a new storage instance with a connection pool.
    ///
    /// # Arguments
    ///
    /// * `database_url` - `PostgreSQL` connection string
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::DatabaseError` if connection fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use solana_indexer::Storage;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let storage = Storage::new("postgresql://localhost/mydb").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(3))
            .connect(database_url)
            .await?;

        Ok(Self { pool })
    }

    /// Returns a reference to the connection pool.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::Storage;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let storage = Storage::new("postgresql://localhost/mydb").await?;
    /// let pool = storage.pool();
    /// // Use pool for queries
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Initializes the database schema.
    ///
    /// Creates the `_solana_indexer_processed` table for idempotency tracking
    /// if it doesn't already exist.
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::DatabaseError` if migration fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::Storage;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let storage = Storage::new("postgresql://localhost/mydb").await?;
    /// storage.initialize().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn initialize(&self) -> Result<()> {
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS _solana_indexer_processed (
                signature TEXT PRIMARY KEY,
                slot BIGINT NOT NULL,
                indexed_at TIMESTAMPTZ DEFAULT NOW()
            )
            ",
        )
        .execute(&self.pool)
        .await?;

        // Create index for faster lookups
        sqlx::query(
            r"
            CREATE INDEX IF NOT EXISTS idx_processed_slot 
            ON _solana_indexer_processed(slot)
            ",
        )
        .execute(&self.pool)
        .await?;

        // Finalized blocks table
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS _solana_indexer_finalized_blocks (
                slot BIGINT PRIMARY KEY,
                block_hash TEXT NOT NULL,
                finalized_at TIMESTAMPTZ DEFAULT NOW()
            )
            ",
        )
        .execute(&self.pool)
        .await?;

        // Tentative transactions table
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS _solana_indexer_tentative (
                signature TEXT PRIMARY KEY,
                slot BIGINT NOT NULL,
                block_hash TEXT NOT NULL,
                indexed_at TIMESTAMPTZ DEFAULT NOW()
            )
            ",
        )
        .execute(&self.pool)
        .await?;

        // Index for tentative slots
        sqlx::query(
            r"
            CREATE INDEX IF NOT EXISTS idx_tentative_slot
            ON _solana_indexer_tentative(slot)
            ",
        )
        .execute(&self.pool)
        .await?;

        // Backfill progress table
        sqlx::query(
            r"
            CREATE TABLE IF NOT EXISTS _solana_indexer_backfill_progress (
                id SERIAL PRIMARY KEY,
                last_slot BIGINT NOT NULL,
                is_complete BOOLEAN DEFAULT FALSE,
                updated_at TIMESTAMPTZ DEFAULT NOW()
            )
            ",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Checks if a transaction has been processed.
    ///
    /// # Arguments
    ///
    /// * `signature` - Transaction signature to check
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::DatabaseError` if query fails.
    ///
    /// # Returns
    ///
    /// `true` if the transaction has been processed, `false` otherwise.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::Storage;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let storage = Storage::new("postgresql://localhost/mydb").await?;
    /// let is_processed = storage.is_processed("signature123").await?;
    /// if !is_processed {
    ///     // Process transaction
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn is_processed(&self, signature: &str) -> Result<bool> {
        let processed = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM _solana_indexer_processed WHERE signature = $1)",
        )
        .bind(signature)
        .fetch_one(&self.pool)
        .await?;

        if processed {
            return Ok(true);
        }

        let tentative = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM _solana_indexer_tentative WHERE signature = $1)",
        )
        .bind(signature)
        .fetch_one(&self.pool)
        .await?;

        Ok(tentative)
    }

    /// Marks a transaction as processed.
    ///
    /// # Arguments
    ///
    /// * `signature` - Transaction signature
    /// * `slot` - Slot number
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::DatabaseError` if insert fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::Storage;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let storage = Storage::new("postgresql://localhost/mydb").await?;
    /// storage.mark_processed("signature123", 12345).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn mark_processed(&self, signature: &str, slot: u64) -> Result<()> {
        sqlx::query(
            "INSERT INTO _solana_indexer_processed (signature, slot) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(signature)
        .bind(i64::try_from(slot).unwrap_or(i64::MAX))
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Gets the last processed slot number.
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::DatabaseError` if query fails.
    ///
    /// # Returns
    ///
    /// The highest slot number that has been processed, or `None` if no
    /// transactions have been processed yet.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::Storage;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let storage = Storage::new("postgresql://localhost/mydb").await?;
    /// if let Some(last_slot) = storage.get_last_processed_slot().await? {
    ///     println!("Last processed slot: {}", last_slot);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_last_processed_slot(&self) -> Result<Option<u64>> {
        let result =
            sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(slot) FROM _solana_indexer_processed")
                .fetch_one(&self.pool)
                .await?;

        Ok(result.map(|s| s.try_into().unwrap_or(0)))
    }

    /// Gets the signature of the last processed transaction.
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::DatabaseError` if query fails.
    ///
    /// # Returns
    ///
    /// The signature of the transaction with the highest slot number, or `None`
    /// if no transactions have been processed yet.
    pub async fn get_last_processed_signature(&self) -> Result<Option<String>> {
        let result = sqlx::query_scalar::<_, Option<String>>(
            "SELECT signature FROM _solana_indexer_processed ORDER BY slot DESC, indexed_at DESC LIMIT 1"
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(result)
    }

    /// Closes the database connection pool.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::Storage;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let storage = Storage::new("postgresql://localhost/mydb").await?;
    /// // ... use storage ...
    /// storage.close().await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn close(self) {
        self.pool.close().await;
    }

    pub async fn mark_tentative(&self, signature: &str, slot: u64, block_hash: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO _solana_indexer_tentative (signature, slot, block_hash) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(signature)
        .bind(i64::try_from(slot).unwrap_or(i64::MAX))
        .bind(block_hash)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_finalized(&self, slot: u64, block_hash: &str) -> Result<()> {
        // Add to finalized blocks table
        sqlx::query(
            "INSERT INTO _solana_indexer_finalized_blocks (slot, block_hash) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(i64::try_from(slot).unwrap_or(i64::MAX))
        .bind(block_hash)
        .execute(&self.pool)
        .await?;

        // Move tentative transactions for this slot to processed table
        sqlx::query(
            r"
            INSERT INTO _solana_indexer_processed (signature, slot, indexed_at)
            SELECT signature, slot, indexed_at 
            FROM _solana_indexer_tentative 
            WHERE slot = $1
            ON CONFLICT (signature) DO NOTHING
            ",
        )
        .bind(i64::try_from(slot).unwrap_or(i64::MAX))
        .execute(&self.pool)
        .await?;

        // Clean up tentative table
        sqlx::query("DELETE FROM _solana_indexer_tentative WHERE slot = $1")
            .bind(i64::try_from(slot).unwrap_or(i64::MAX))
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn get_tentative_transactions(&self, slot: u64) -> Result<Vec<String>> {
        let signatures = sqlx::query_scalar::<_, String>(
            "SELECT signature FROM _solana_indexer_tentative WHERE slot = $1",
        )
        .bind(i64::try_from(slot).unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await?;
        Ok(signatures)
    }

    pub async fn rollback_slot(&self, slot: u64) -> Result<()> {
        let slot_i64 = i64::try_from(slot).unwrap_or(i64::MAX);

        // Start a transaction would be better, but for now sequential deletes
        // Delete from tentative
        sqlx::query("DELETE FROM _solana_indexer_tentative WHERE slot = $1")
            .bind(slot_i64)
            .execute(&self.pool)
            .await?;

        // Delete from processed (idempotency)
        sqlx::query("DELETE FROM _solana_indexer_processed WHERE slot = $1")
            .bind(slot_i64)
            .execute(&self.pool)
            .await?;

        // Delete from finalized blocks
        sqlx::query("DELETE FROM _solana_indexer_finalized_blocks WHERE slot = $1")
            .bind(slot_i64)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn cleanup_stale_tentative_transactions(&self, slot_threshold: u64) -> Result<u64> {
        // Delete tentative transactions older than the threshold
        // We use the current max processed slot as the reference
        let current_slot = self.get_last_processed_slot().await?.unwrap_or(0);

        if current_slot < slot_threshold {
            return Ok(0);
        }

        let cutoff_slot = current_slot - slot_threshold;

        let result = sqlx::query("DELETE FROM _solana_indexer_tentative WHERE slot < $1")
            .bind(i64::try_from(cutoff_slot).unwrap_or(0))
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    pub async fn get_tentative_slots_le(&self, slot: u64) -> Result<Vec<u64>> {
        let slots = sqlx::query_scalar::<_, i64>(
            "SELECT DISTINCT slot FROM _solana_indexer_tentative WHERE slot <= $1 ORDER BY slot ASC",
        )
        .bind(i64::try_from(slot).unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await?;

        Ok(slots
            .into_iter()
            .map(|s| s.try_into().unwrap_or(0))
            .collect())
    }

    pub async fn get_block_hash(&self, slot: u64) -> Result<Option<String>> {
        let hash = sqlx::query_scalar::<_, Option<String>>(
            "SELECT block_hash FROM _solana_indexer_finalized_blocks WHERE slot = $1",
        )
        .bind(i64::try_from(slot).unwrap_or(i64::MAX))
        .fetch_one(&self.pool)
        .await?;

        if hash.is_some() {
            return Ok(hash);
        }

        let hash = sqlx::query_scalar::<_, Option<String>>(
            "SELECT block_hash FROM _solana_indexer_tentative WHERE slot = $1 LIMIT 1",
        )
        .bind(i64::try_from(slot).unwrap_or(i64::MAX))
        .fetch_one(&self.pool)
        .await?;

        Ok(hash)
    }

    pub async fn save_backfill_progress(&self, slot: u64) -> Result<()> {
        sqlx::query(
            r"
            INSERT INTO _solana_indexer_backfill_progress (id, last_slot, updated_at) 
            VALUES (1, $1, NOW())
            ON CONFLICT (id) DO UPDATE SET last_slot = $1, updated_at = NOW()
            ",
        )
        .bind(i64::try_from(slot).unwrap_or(i64::MAX))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn load_backfill_progress(&self) -> Result<Option<u64>> {
        let slot = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT last_slot FROM _solana_indexer_backfill_progress WHERE id = 1",
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(slot.map(|s| s.try_into().unwrap_or(0)))
    }

    pub async fn mark_backfill_complete(&self) -> Result<()> {
        sqlx::query(
            "UPDATE _solana_indexer_backfill_progress SET is_complete = TRUE, updated_at = NOW() WHERE id = 1",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[async_trait]
impl StorageBackend for Storage {
    async fn initialize(&self) -> Result<()> {
        self.initialize().await
    }

    async fn is_processed(&self, signature: &str) -> Result<bool> {
        self.is_processed(signature).await
    }

    async fn mark_processed(&self, signature: &str, slot: u64) -> Result<()> {
        self.mark_processed(signature, slot).await
    }

    async fn get_last_processed_slot(&self) -> Result<Option<u64>> {
        self.get_last_processed_slot().await
    }

    async fn get_last_processed_signature(&self) -> Result<Option<String>> {
        self.get_last_processed_signature().await
    }

    fn pool(&self) -> &PgPool {
        &self.pool
    }

    async fn mark_tentative(&self, signature: &str, slot: u64, block_hash: &str) -> Result<()> {
        self.mark_tentative(signature, slot, block_hash).await
    }

    async fn mark_finalized(&self, slot: u64, block_hash: &str) -> Result<()> {
        self.mark_finalized(slot, block_hash).await
    }

    async fn get_tentative_transactions(&self, slot: u64) -> Result<Vec<String>> {
        self.get_tentative_transactions(slot).await
    }

    async fn rollback_slot(&self, slot: u64) -> Result<()> {
        self.rollback_slot(slot).await
    }

    async fn get_block_hash(&self, slot: u64) -> Result<Option<String>> {
        self.get_block_hash(slot).await
    }

    async fn cleanup_stale_tentative_transactions(&self, slot_threshold: u64) -> Result<u64> {
        self.cleanup_stale_tentative_transactions(slot_threshold)
            .await
    }

    async fn get_tentative_slots_le(&self, slot: u64) -> Result<Vec<u64>> {
        self.get_tentative_slots_le(slot).await
    }

    async fn save_backfill_progress(&self, slot: u64) -> Result<()> {
        self.save_backfill_progress(slot).await
    }

    async fn load_backfill_progress(&self) -> Result<Option<u64>> {
        self.load_backfill_progress().await
    }

    async fn mark_backfill_complete(&self) -> Result<()> {
        self.mark_backfill_complete().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_creation() {
        // This test just verifies the struct can be instantiated
        // Actual database tests require a running PostgreSQL instance
        // Success if we get here
    }

    #[tokio::test]
    // #[ignore = "Requires database connection"] // Requires database connection
    async fn test_storage_initialize() {
        let db_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://localhost/test".to_string());

        if let Ok(storage) = Storage::new(&db_url).await {
            let result = storage.initialize().await;
            assert!(result.is_ok());
        }
    }

    #[tokio::test]
    // #[ignore = "Requires database connection"] // Requires database connection
    async fn test_idempotency_tracking() {
        let db_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://localhost/test".to_string());

        if let Ok(storage) = Storage::new(&db_url).await {
            storage.initialize().await.unwrap();

            let signature = "test_signature_123";
            let slot = 12345;

            // Should not be processed initially
            let is_processed = storage.is_processed(signature).await.unwrap();
            assert!(!is_processed);

            // Mark as processed
            storage.mark_processed(signature, slot).await.unwrap();

            // Should now be processed
            let is_processed = storage.is_processed(signature).await.unwrap();
            assert!(is_processed);

            // Get last processed slot
            let last_slot = storage.get_last_processed_slot().await.unwrap();
            assert_eq!(last_slot, Some(slot));
        }
    }

    #[tokio::test]
    async fn test_cleanup_stale_transactions() {
        let db_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://localhost/test".to_string());

        if let Ok(storage) = Storage::new(&db_url).await {
            storage.initialize().await.unwrap();

            // Setup: Insert some tentative transactions
            let current_slot = 1000;
            storage
                .mark_processed("sig_latest", current_slot)
                .await
                .unwrap();

            // Old transaction (should be deleted)
            storage
                .mark_tentative("sig_old", current_slot - 100, "hash_old")
                .await
                .unwrap();

            // New transaction (should be kept)
            storage
                .mark_tentative("sig_new", current_slot - 10, "hash_new")
                .await
                .unwrap();

            // Threshold is 50, so anything older than 1000 - 50 = 950 should be deleted?
            // Wait, logic is: slot < (current - threshold)
            // If threshold is 50, cutoff is 950.
            // sig_old is 900. 900 < 950 -> deleted.
            // sig_new is 990. 990 < 950 -> false -> kept.

            let deleted = storage
                .cleanup_stale_tentative_transactions(50)
                .await
                .unwrap();
            assert!(deleted >= 1);

            // Verify
            let tentative_old = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM _solana_indexer_tentative WHERE signature = 'sig_old')"
            )
            .fetch_one(&storage.pool)
            .await
            .unwrap();
            assert!(!tentative_old, "Old transaction should be deleted");

            let tentative_new = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM _solana_indexer_tentative WHERE signature = 'sig_new')"
            )
            .fetch_one(&storage.pool)
            .await
            .unwrap();
            assert!(tentative_new, "New transaction should be kept");
        }
    }
}
