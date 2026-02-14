//! Event handler traits for `SolanaIndexer`.
//!
//! This module defines the core extensibility mechanism for the indexer through
//! the `EventHandler` trait. Developers implement this trait to define custom
//! business logic for processing decoded events and transactions.

use crate::types::events::{EventDiscriminator, ParsedEvent};
use crate::types::metadata::TxMetadata;
use crate::utils::error::{Result, SolanaIndexerError};
use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_transaction_status::UiInstruction;
use sqlx::PgPool;

/// Generic instruction decoder trait for custom parsing logic.
///
/// Developers implement this trait to define how raw Solana instructions
/// are parsed into typed event structures.
///
/// # Type Parameters
/// * `T` - The event type this decoder produces
///
/// # Example
/// ```no_run
/// use solana_indexer_sdk::InstructionDecoder;
/// use solana_transaction_status::UiInstruction;
///
/// pub struct MyDecoder;
///
/// pub struct MyEvent;
///
/// impl InstructionDecoder<MyEvent> for MyDecoder {
///     fn decode(&self, instruction: &UiInstruction) -> Option<MyEvent> {
///         // Parse instruction and return event if successful
///         None
///     }
/// }
/// ```
pub trait InstructionDecoder<T>: Send + Sync {
    /// Decodes a Solana instruction into a typed event.
    ///
    /// # Arguments
    /// * `instruction` - The UI instruction from the transaction
    ///
    /// # Returns
    /// * `Some(T)` - Successfully decoded event
    /// * `None` - Instruction doesn't match or failed to decode
    fn decode(&self, instruction: &UiInstruction) -> Option<T>;
}

/// Type-erased instruction decoder for internal SDK use.
///
/// This trait enables the SDK to store and invoke decoders of different
/// types uniformly. Developers don't implement this directly.
pub trait DynamicInstructionDecoder: Send + Sync {
    /// Decodes an instruction into discriminator + raw event data.
    fn decode_dynamic(&self, instruction: &UiInstruction) -> Option<([u8; 8], Vec<u8>)>;
}

/// Automatic conversion from typed decoder to dynamic decoder.
impl<T> DynamicInstructionDecoder for Box<dyn InstructionDecoder<T>>
where
    T: EventDiscriminator + BorshSerialize + Send + Sync + 'static,
{
    fn decode_dynamic(&self, instruction: &UiInstruction) -> Option<([u8; 8], Vec<u8>)> {
        self.decode(instruction).map(|event| {
            let discriminator = T::discriminator();
            let data = borsh::to_vec(&event).expect("Failed to serialize event");
            (discriminator, data)
        })
    }
}

/// Generic log decoder trait for custom parsing logic.
///
/// Developers implement this trait to define how transaction logs
/// are parsed into typed event structures.
pub trait LogDecoder<T>: Send + Sync {
    /// Decodes a parsed log event into a typed event.
    ///
    /// # Arguments
    /// * `event` - The parsed event from transaction logs
    ///
    /// # Returns
    /// * `Some(T)` - Successfully decoded event
    /// * `None` - Event doesn't match or failed to decode
    fn decode(&self, event: &ParsedEvent) -> Option<T>;
}

/// Type-erased log decoder for internal SDK use.
pub trait DynamicLogDecoder: Send + Sync {
    /// Decodes a log event into discriminator + raw event data.
    fn decode_log_dynamic(&self, event: &ParsedEvent) -> Option<([u8; 8], Vec<u8>)>;
}

impl<T> DynamicLogDecoder for Box<dyn LogDecoder<T>>
where
    T: EventDiscriminator + BorshSerialize + Send + Sync + 'static,
{
    fn decode_log_dynamic(&self, event: &ParsedEvent) -> Option<([u8; 8], Vec<u8>)> {
        self.decode(event).map(|event| {
            let discriminator = T::discriminator();
            let data = borsh::to_vec(&event).expect("Failed to serialize event");
            (discriminator, data)
        })
    }
}

/// Generic account decoder trait for custom parsing logic.
///
/// Developers implement this trait to define how Solana accounts
/// are parsed into typed structures.
pub trait AccountDecoder<T>: Send + Sync {
    /// Decodes a Solana account into a typed structure.
    ///
    /// # Arguments
    /// * `account` - The raw account data
    ///
    /// # Returns
    /// * `Some(T)` - Successfully decoded type
    /// * `None` - Data doesn't match or failed to decode
    fn decode(&self, account: &solana_sdk::account::Account) -> Option<T>;
}

/// Type-erased account decoder for internal SDK use.
pub trait DynamicAccountDecoder: Send + Sync {
    /// Decodes an account into discriminator + raw data.
    fn decode_account_dynamic(
        &self,
        account: &solana_sdk::account::Account,
    ) -> Option<([u8; 8], Vec<u8>)>;
}

impl<T> DynamicAccountDecoder for Box<dyn AccountDecoder<T>>
where
    T: EventDiscriminator + BorshSerialize + Send + Sync + 'static,
{
    fn decode_account_dynamic(
        &self,
        account: &solana_sdk::account::Account,
    ) -> Option<([u8; 8], Vec<u8>)> {
        self.decode(account).map(|event| {
            let discriminator = T::discriminator();
            let data = borsh::to_vec(&event).expect("Failed to serialize event");
            (discriminator, data)
        })
    }
}

/// Trait for initializing custom database schemas.
///
/// Implement this trait to define custom table creation logic that runs
/// when the indexer starts.
#[async_trait]
pub trait SchemaInitializer: Send + Sync {
    /// Initializes the database schema.
    ///
    /// # Arguments
    ///
    /// * `db` - Database connection pool
    async fn initialize(&self, db: &PgPool) -> Result<()>;
}

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
/// use solana_indexer_sdk::{EventHandler, SolanaIndexerError};
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
///         context: &solana_indexer_sdk::types::metadata::TxMetadata,
///         db: &PgPool,
///     ) -> Result<(), SolanaIndexerError> {
///         let signature = &context.signature;
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
    /// * `context` - The transaction context (slot, block time, fee, etc.)
    /// * `db` - Database connection pool for persistence operations
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
    /// # use solana_indexer_sdk::{EventHandler, SolanaIndexerError};
    /// # use solana_indexer_sdk::types::metadata::TxMetadata;
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
    ///         context: &TxMetadata,
    ///         db: &PgPool,
    ///     ) -> Result<(), SolanaIndexerError> {
    ///         // Custom processing logic
    ///         println!("Event value: {}, Slot: {}", event.value, context.slot);
    ///         Ok(())
    ///     }
    /// }
    /// ```
    async fn handle(&self, event: T, context: &TxMetadata, db: &PgPool) -> Result<()>;

    /// Called when a previously-confirmed transaction is rolled back (reorg).
    ///
    /// This is an optional hook. Default implementation is a no-op.
    ///
    /// # Arguments
    /// * `context` - Metadata about the rolled-back transaction (signature, slot will be invalid now)
    /// * `db` - Database connection pool
    async fn on_rollback(&self, _context: &TxMetadata, _db: &PgPool) -> Result<()> {
        Ok(())
    }

    /// Initializes custom database schema for this handler.
    ///
    /// Override this method to create custom tables. Called once
    /// during indexer startup.
    ///
    /// # Default Implementation
    /// Does nothing (no-op).
    /// # Arguments
    /// * `pool` - Database connection pool
    ///
    /// # Returns
    /// * `Result<()>` - Success or error
    async fn initialize_schema(&self, pool: &PgPool) -> Result<()> {
        let _ = pool; // Default implementation does nothing
        Ok(())
    }
}

/// Type-erased event handler for dynamic dispatch.
///
/// This trait allows storing handlers of different event types in a single
/// collection, enabling the handler registry to manage multiple handler types.
#[async_trait]
pub trait DynamicEventHandler: Send + Sync {
    /// Handles a dynamic event (discriminator + raw bytes).
    async fn handle_dynamic(
        &self,
        discriminator: &[u8; 8],
        data: &[u8],
        context: &TxMetadata,
        db: &PgPool,
    ) -> Result<()>;

    /// Handles a rollback for a dynamic event.
    async fn handle_rollback_dynamic(&self, context: &TxMetadata, db: &PgPool) -> Result<()>;

    /// Initializes schema for the dynamic handler.
    async fn initialize_schema(&self, pool: &PgPool) -> Result<()>;
}

/// Automatic conversion from typed handler to dynamic handler.
#[async_trait]
impl<T> DynamicEventHandler for Box<dyn EventHandler<T>>
where
    T: EventDiscriminator + BorshDeserialize + Send + Sync + 'static,
{
    async fn handle_dynamic(
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
            SolanaIndexerError::DecodingError(format!("Failed to deserialize event: {}", e))
        })?;

        // Delegate to typed handler
        self.handle(event, context, db).await
    }

    async fn handle_rollback_dynamic(&self, context: &TxMetadata, db: &PgPool) -> Result<()> {
        (**self).on_rollback(context, db).await
    }

    async fn initialize_schema(&self, pool: &PgPool) -> Result<()> {
        (**self).initialize_schema(pool).await
    }
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
/// use solana_indexer_sdk::HandlerRegistry;
///
/// let mut registry = HandlerRegistry::new();
/// // Register handlers here
/// ```
pub struct HandlerRegistry {
    /// Map of discriminators to handlers
    handlers: std::collections::HashMap<[u8; 8], Box<dyn DynamicEventHandler>>,
    metrics: crate::core::registry_metrics::RegistryMetrics,
}

impl HandlerRegistry {
    /// Creates a new empty handler registry with unlimited capacity.
    ///
    /// # Example
    ///
    /// ```
    /// use solana_indexer_sdk::HandlerRegistry;
    ///
    /// let registry = HandlerRegistry::new();
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            handlers: std::collections::HashMap::new(),
            metrics: crate::core::registry_metrics::RegistryMetrics::new("EventHandler", 0),
        }
    }

    /// Creates a new handler registry with a specific capacity limit.
    pub fn new_bounded(config: &crate::config::RegistryConfig) -> Self {
        Self {
            handlers: std::collections::HashMap::new(),
            metrics: crate::core::registry_metrics::RegistryMetrics::new(
                "EventHandler",
                config.max_handlers,
            ),
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
    /// # use solana_indexer_sdk::HandlerRegistry;
    /// # use std::sync::Arc;
    /// let mut registry = HandlerRegistry::new();
    /// // registry.register([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08], handler);
    /// ```
    pub fn register(
        &mut self,
        discriminator: [u8; 8],
        handler: Box<dyn DynamicEventHandler>,
    ) -> Result<()> {
        if !self.handlers.contains_key(&discriminator) && self.metrics.is_full() {
            return Err(SolanaIndexerError::RegistryCapacityExceeded(format!(
                "EventHandler registry full (limit: {})",
                self.metrics.capacity_limit
            )));
        }

        self.handlers.insert(discriminator, handler);
        self.metrics.inc_registered();
        Ok(())
    }

    /// Triggers rollback on all registered handlers.
    pub async fn handle_rollback(&self, context: &TxMetadata, db: &PgPool) -> Result<()> {
        for handler in self.handlers.values() {
            handler.handle_rollback_dynamic(context, db).await?;
        }
        Ok(())
    }

    /// Handles an event by dispatching to the appropriate handler.
    ///
    /// # Arguments
    ///
    /// * `discriminator` - The event discriminator
    /// * `event_data` - Raw event data
    /// * `context` - The transaction context
    /// * `db` - Database connection pool
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::DecodingError` if no handler is registered
    /// for the discriminator, or propagates handler errors.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer_sdk::HandlerRegistry;
    /// # use solana_indexer_sdk::types::metadata::TxMetadata;
    /// # use sqlx::PgPool;
    /// # async fn example(db: &PgPool, context: &TxMetadata) -> Result<(), Box<dyn std::error::Error>> {
    /// let registry = HandlerRegistry::new();
    /// let discriminator = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
    /// let event_data = b"event data";
    ///
    /// // registry.handle(&discriminator, event_data, context, db).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn handle(
        &self,
        discriminator: &[u8; 8],
        event_data: &[u8],
        context: &TxMetadata,
        db: &PgPool,
    ) -> Result<()> {
        self.metrics.inc_calls();
        let handler = self.handlers.get(discriminator).ok_or_else(|| {
            SolanaIndexerError::DecodingError(format!(
                "No handler registered for discriminator: {discriminator:?}"
            ))
        })?;

        let result = handler
            .handle_dynamic(discriminator, event_data, context, db)
            .await;
        if result.is_ok() {
            self.metrics.inc_hits();
        }
        result
    }

    /// Returns the number of registered handlers.
    ///
    /// # Example
    ///
    /// ```
    /// use solana_indexer_sdk::HandlerRegistry;
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
    /// use solana_indexer_sdk::HandlerRegistry;
    ///
    /// let registry = HandlerRegistry::new();
    /// assert!(registry.is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }

    /// Returns the metrics for this registry.
    pub fn metrics(&self) -> &crate::core::registry_metrics::RegistryMetrics {
        &self.metrics
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
    #[allow(dead_code)]
    struct TestEvent {
        value: u64,
    }

    #[allow(dead_code)]
    struct TestHandler;

    #[async_trait]
    impl EventHandler<TestEvent> for TestHandler {
        async fn handle(
            &self,
            event: TestEvent,
            context: &crate::types::metadata::TxMetadata,
            _db: &PgPool,
        ) -> Result<()> {
            assert!(event.value > 0);
            assert!(!context.signature.is_empty());
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
            discriminator: &[u8; 8],
            _data: &[u8],
            _context: &crate::types::metadata::TxMetadata,
            _db: &PgPool,
        ) -> Result<()> {
            if *discriminator != self.discriminator {
                return Err(SolanaIndexerError::DecodingError(
                    "Discriminator mismatch".to_string(),
                ));
            }
            Ok(())
        }

        async fn handle_rollback_dynamic(
            &self,
            _context: &crate::types::metadata::TxMetadata,
            _db: &PgPool,
        ) -> Result<()> {
            Ok(())
        }

        async fn initialize_schema(&self, _pool: &PgPool) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_handler_registry_register() {
        let mut registry = HandlerRegistry::new();
        let discriminator = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];

        let handler = Box::new(MockDynamicHandler { discriminator });
        registry.register(discriminator, handler).unwrap();

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
            let context = crate::types::metadata::TxMetadata {
                slot: 0,
                block_time: None,
                fee: 0,
                pre_balances: vec![],
                post_balances: vec![],
                pre_token_balances: vec![],
                post_token_balances: vec![],
                signature: "sig".to_string(),
            };
            let result = registry
                .handle(&discriminator, b"data", &context, &db)
                .await;
            assert!(result.is_err());
        }
    }
}
