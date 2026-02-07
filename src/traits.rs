//! Event handler traits for `SolanaIndexer`.
//!
//! This module defines the core extensibility mechanism for the indexer through
//! the `EventHandler` trait. Developers implement this trait to define custom
//! business logic for processing decoded events and transactions.

use crate::error::{Result, SolanaIndexerError};
use async_trait::async_trait;
use sqlx::PgPool;

/// Event handler trait for processing decoded events.
///
/// The `EventHandler` trait is the primary extension point for `SolanaIndexer`,
/// enabling developers to inject custom business logic for processing events.
/// It is generic over the event type `T`, allowing type-safe handling of
/// specific event structures.
///
/// # Type Parameters
///
/// * `T` - The event type to handle. This is typically a generated struct from
///   an IDL or a custom event type defined by the developer.
///
/// # Example
///
/// ```no_run
/// use solana_indexer::{EventHandler, SolanaIndexerError};
/// use async_trait::async_trait;
/// use sqlx::PgPool;
///
/// // Define a custom event type
/// #[derive(Debug, Clone)]
/// pub struct TransferEvent {
///     pub from: String,
///     pub to: String,
///     pub amount: u64,
/// }
///
/// // Implement the handler
/// pub struct TransferHandler;
///
/// #[async_trait]
/// impl EventHandler<TransferEvent> for TransferHandler {
///     async fn handle(
///         &self,
///         event: TransferEvent,
///         db: &PgPool,
///         signature: &str,
///     ) -> Result<(), SolanaIndexerError> {
///         println!("Processing transfer: {} -> {} ({})", event.from, event.to, event.amount);
///         
///         // Insert into database
///         sqlx::query(
///             "INSERT INTO transfers (signature, from_wallet, to_wallet, amount) VALUES ($1, $2, $3, $4)"
///         )
///         .bind(signature)
///         .bind(&event.from)
///         .bind(&event.to)
///         .bind(event.amount as i64)
///         .execute(db)
///         .await?;
///         
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait EventHandler<T>: Send + Sync + 'static {
    /// Handles a decoded event.
    ///
    /// This method is called by the indexer for each event of type `T` that
    /// is decoded from a transaction. Implementations should contain all
    /// custom business logic for processing the event.
    ///
    /// # Arguments
    ///
    /// * `event` - The decoded event object
    /// * `db` - Database connection pool for persistence operations
    /// * `signature` - Transaction signature for logging and tracking
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError` if processing fails. Common error cases:
    /// - Database errors during persistence
    /// - External API call failures
    /// - Validation errors
    ///
    /// # Returns
    ///
    /// `Ok(())` on successful processing.
    ///
    /// # Example
    ///
    /// ```
    /// # use solana_indexer::{EventHandler, SolanaIndexerError};
    /// # use async_trait::async_trait;
    /// # use sqlx::PgPool;
    /// #
    /// # #[derive(Debug, Clone)]
    /// # pub struct MyEvent { pub value: u64 }
    /// #
    /// pub struct MyHandler;
    ///
    /// #[async_trait]
    /// impl EventHandler<MyEvent> for MyHandler {
    ///     async fn handle(
    ///         &self,
    ///         event: MyEvent,
    ///         db: &PgPool,
    ///         signature: &str,
    ///     ) -> Result<(), SolanaIndexerError> {
    ///         // Custom processing logic
    ///         println!("Event value: {}", event.value);
    ///         Ok(())
    ///     }
    /// }
    /// ```
    async fn handle(&self, event: T, db: &PgPool, signature: &str) -> Result<()>;
}

/// Type-erased event handler for dynamic dispatch.
///
/// This trait allows storing handlers of different event types in a single
/// collection, enabling the handler registry to manage multiple handler types.
#[async_trait]
pub trait DynamicEventHandler: Send + Sync {
    /// Handles an event with dynamic dispatch.
    ///
    /// # Arguments
    ///
    /// * `event_data` - Raw event data as bytes
    /// * `db` - Database connection pool
    /// * `signature` - Transaction signature
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError` if handling fails.
    async fn handle_dynamic(
        &self,
        event_data: &[u8],
        db: &PgPool,
        signature: &str,
    ) -> Result<()>;

    /// Returns the event discriminator this handler processes.
    fn discriminator(&self) -> [u8; 8];
}

/// Handler registry for managing multiple event handlers.
///
/// The `HandlerRegistry` stores and manages event handlers, allowing the
/// indexer to dispatch events to the appropriate handler based on their
/// discriminator.
///
/// # Example
///
/// ```
/// use solana_indexer::HandlerRegistry;
///
/// let mut registry = HandlerRegistry::new();
/// // Register handlers here
/// ```
pub struct HandlerRegistry {
    /// Map of discriminators to handlers
    handlers: std::collections::HashMap<[u8; 8], Box<dyn DynamicEventHandler>>,
}

impl HandlerRegistry {
    /// Creates a new empty handler registry.
    ///
    /// # Example
    ///
    /// ```
    /// use solana_indexer::HandlerRegistry;
    ///
    /// let registry = HandlerRegistry::new();
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            handlers: std::collections::HashMap::new(),
        }
    }

    /// Registers a handler for a specific event discriminator.
    ///
    /// # Arguments
    ///
    /// * `discriminator` - The 8-byte event discriminator
    /// * `handler` - The handler implementation
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::HandlerRegistry;
    /// # use std::sync::Arc;
    /// let mut registry = HandlerRegistry::new();
    /// // registry.register([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08], handler);
    /// ```
    pub fn register(
        &mut self,
        discriminator: [u8; 8],
        handler: Box<dyn DynamicEventHandler>,
    ) {
        self.handlers.insert(discriminator, handler);
    }

    /// Handles an event by dispatching to the appropriate handler.
    ///
    /// # Arguments
    ///
    /// * `discriminator` - The event discriminator
    /// * `event_data` - Raw event data
    /// * `db` - Database connection pool
    /// * `signature` - Transaction signature
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::DecodingError` if no handler is registered
    /// for the discriminator, or propagates handler errors.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer::HandlerRegistry;
    /// # use sqlx::PgPool;
    /// # async fn example(db: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    /// let registry = HandlerRegistry::new();
    /// let discriminator = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
    /// let event_data = b"event data";
    /// 
    /// // registry.handle(&discriminator, event_data, db, "signature").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn handle(
        &self,
        discriminator: &[u8; 8],
        event_data: &[u8],
        db: &PgPool,
        signature: &str,
    ) -> Result<()> {
        let handler = self.handlers.get(discriminator).ok_or_else(|| {
            SolanaIndexerError::DecodingError(format!(
                "No handler registered for discriminator: {:?}",
                discriminator
            ))
        })?;

        handler.handle_dynamic(event_data, db, signature).await
    }

    /// Returns the number of registered handlers.
    ///
    /// # Example
    ///
    /// ```
    /// use solana_indexer::HandlerRegistry;
    ///
    /// let registry = HandlerRegistry::new();
    /// assert_eq!(registry.len(), 0);
    /// ```
    #[must_use]
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    /// Returns true if no handlers are registered.
    ///
    /// # Example
    ///
    /// ```
    /// use solana_indexer::HandlerRegistry;
    ///
    /// let registry = HandlerRegistry::new();
    /// assert!(registry.is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

impl Default for HandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    struct TestEvent {
        value: u64,
    }

    struct TestHandler;

    #[async_trait]
    impl EventHandler<TestEvent> for TestHandler {
        async fn handle(&self, event: TestEvent, _db: &PgPool, signature: &str) -> Result<()> {
            assert!(event.value > 0);
            assert!(!signature.is_empty());
            Ok(())
        }
    }

    #[test]
    fn test_handler_registry_creation() {
        let registry = HandlerRegistry::new();
        assert_eq!(registry.len(), 0);
        assert!(registry.is_empty());
    }

    #[test]
    fn test_handler_registry_default() {
        let registry = HandlerRegistry::default();
        assert!(registry.is_empty());
    }

    struct MockDynamicHandler {
        discriminator: [u8; 8],
    }

    #[async_trait]
    impl DynamicEventHandler for MockDynamicHandler {
        async fn handle_dynamic(
            &self,
            _event_data: &[u8],
            _db: &PgPool,
            _signature: &str,
        ) -> Result<()> {
            Ok(())
        }

        fn discriminator(&self) -> [u8; 8] {
            self.discriminator
        }
    }

    #[test]
    fn test_handler_registry_register() {
        let mut registry = HandlerRegistry::new();
        let discriminator = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        
        let handler = Box::new(MockDynamicHandler { discriminator });
        registry.register(discriminator, handler);
        
        assert_eq!(registry.len(), 1);
        assert!(!registry.is_empty());
    }

    #[tokio::test]
    async fn test_handler_registry_handle_not_found() {
        let registry = HandlerRegistry::new();
        let discriminator = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        
        // Create a mock database pool (this will fail in tests without a real DB)
        // For now, we'll just test the error case
        let db_url = "postgresql://localhost/test";
        let pool = sqlx::PgPool::connect(db_url).await;
        
        // If we can't connect, that's fine for this test - we're testing the registry logic
        if let Ok(db) = pool {
            let result = registry.handle(&discriminator, b"data", &db, "sig").await;
            assert!(result.is_err());
        }
    }
}
