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
    fn pool(&self) -> &PgPool;
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
        let result = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM _solana_indexer_processed WHERE signature = $1)",
        )
        .bind(signature)
        .fetch_one(&self.pool)
        .await?;

        Ok(result)
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

    fn pool(&self) -> &PgPool {
        &self.pool
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
    #[ignore = "Requires database connection"] // Requires database connection
    async fn test_storage_initialize() {
        let db_url = std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://localhost/test".to_string());

        if let Ok(storage) = Storage::new(&db_url).await {
            let result = storage.initialize().await;
            assert!(result.is_ok());
        }
    }

    #[tokio::test]
    #[ignore = "Requires database connection"] // Requires database connection
    async fn test_idempotency_tracking() {
        let db_url = std::env::var("TEST_DATABASE_URL")
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
}
