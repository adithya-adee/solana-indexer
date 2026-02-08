//! Generated event types and type generation utilities.
//!
//! This module contains example generated event types and utilities for
//! IDL-based type generation. In a full implementation, these types would
//! be automatically generated from Solana program IDLs during compilation.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Calculates the 8-byte discriminator for an event type.
///
/// The discriminator is the first 8 bytes of the SHA256 hash of the
/// event name prefixed with "event:". This matches Anchor's event
/// discriminator calculation.
///
/// # Arguments
///
/// * `event_name` - The name of the event type
///
/// # Returns
///
/// An 8-byte array representing the event discriminator.
///
/// # Example
///
/// ```
/// use solana_indexer::calculate_discriminator;
///
/// let discriminator = calculate_discriminator("TransferEvent");
/// assert_eq!(discriminator.len(), 8);
/// ```
#[must_use]
pub fn calculate_discriminator(event_name: &str) -> [u8; 8] {
    let preimage = format!("event:{event_name}");
    let hash = Sha256::digest(preimage.as_bytes());
    let mut discriminator = [0u8; 8];
    discriminator.copy_from_slice(&hash[..8]);
    discriminator
}

/// Example generated event type: `TransferEvent`
///
/// This represents a token transfer event that might be emitted by a
/// Solana program. In a real implementation, this would be automatically
/// generated from the program's IDL.
///
/// # Example
///
/// ```
/// use solana_indexer::TransferEvent;
///
/// let event = TransferEvent {
///     from: "11111111111111111111111111111111".to_string(),
///     to: "22222222222222222222222222222222".to_string(),
///     amount: 1000,
/// };
///
/// assert_eq!(event.amount, 1000);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct TransferEvent {
    /// Source wallet address
    pub from: String,
    /// Destination wallet address
    pub to: String,
    /// Transfer amount
    pub amount: u64,
}

impl TransferEvent {
    /// Returns the event discriminator for `TransferEvent`.
    ///
    /// # Example
    ///
    /// ```
    /// use solana_indexer::TransferEvent;
    ///
    /// let discriminator = TransferEvent::discriminator();
    /// assert_eq!(discriminator.len(), 8);
    /// ```
    #[must_use]
    pub fn discriminator() -> [u8; 8] {
        calculate_discriminator("TransferEvent")
    }
}

/// Example generated event type: `DepositEvent`
///
/// This represents a deposit event that might be emitted by a `DeFi` program.
///
/// # Example
///
/// ```
/// use solana_indexer::DepositEvent;
///
/// let event = DepositEvent {
///     user: "11111111111111111111111111111111".to_string(),
///     amount: 5000,
///     timestamp: 1234567890,
/// };
///
/// assert_eq!(event.amount, 5000);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct DepositEvent {
    /// User wallet address
    pub user: String,
    /// Deposit amount
    pub amount: u64,
    /// Unix timestamp
    pub timestamp: i64,
}

impl DepositEvent {
    /// Returns the event discriminator for `DepositEvent`.
    ///
    /// # Example
    ///
    /// ```
    /// use solana_indexer::DepositEvent;
    ///
    /// let discriminator = DepositEvent::discriminator();
    /// assert_eq!(discriminator.len(), 8);
    /// ```
    #[must_use]
    pub fn discriminator() -> [u8; 8] {
        calculate_discriminator("DepositEvent")
    }
}

/// Example generated event type: `WithdrawEvent`
///
/// This represents a withdrawal event that might be emitted by a `DeFi` program.
///
/// # Example
///
/// ```
/// use solana_indexer::WithdrawEvent;
///
/// let event = WithdrawEvent {
///     user: "11111111111111111111111111111111".to_string(),
///     amount: 2500,
///     timestamp: 1234567890,
/// };
///
/// assert_eq!(event.amount, 2500);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct WithdrawEvent {
    /// User wallet address
    pub user: String,
    /// Withdrawal amount
    pub amount: u64,
    /// Unix timestamp
    pub timestamp: i64,
}

impl WithdrawEvent {
    /// Returns the event discriminator for `WithdrawEvent`.
    ///
    /// # Example
    ///
    /// ```
    /// use solana_indexer::WithdrawEvent;
    ///
    /// let discriminator = WithdrawEvent::discriminator();
    /// assert_eq!(discriminator.len(), 8);
    /// ```
    #[must_use]
    pub fn discriminator() -> [u8; 8] {
        calculate_discriminator("WithdrawEvent")
    }
}

/// Trait for types that have an event discriminator.
///
/// This trait is automatically implemented for all generated event types,
/// providing a uniform way to access their discriminators.
pub trait EventDiscriminator {
    /// Returns the 8-byte discriminator for this event type.
    fn discriminator() -> [u8; 8];
}

impl EventDiscriminator for TransferEvent {
    fn discriminator() -> [u8; 8] {
        Self::discriminator()
    }
}

impl EventDiscriminator for DepositEvent {
    fn discriminator() -> [u8; 8] {
        Self::discriminator()
    }
}

impl EventDiscriminator for WithdrawEvent {
    fn discriminator() -> [u8; 8] {
        Self::discriminator()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_discriminator() {
        let discriminator = calculate_discriminator("TestEvent");
        assert_eq!(discriminator.len(), 8);

        // Discriminator should be deterministic
        let discriminator2 = calculate_discriminator("TestEvent");
        assert_eq!(discriminator, discriminator2);
    }

    #[test]
    fn test_different_events_different_discriminators() {
        let disc1 = calculate_discriminator("Event1");
        let disc2 = calculate_discriminator("Event2");
        assert_ne!(disc1, disc2);
    }

    #[test]
    fn test_transfer_event_creation() {
        let event = TransferEvent {
            from: "sender".to_string(),
            to: "receiver".to_string(),
            amount: 1000,
        };

        assert_eq!(event.from, "sender");
        assert_eq!(event.to, "receiver");
        assert_eq!(event.amount, 1000);
    }

    #[test]
    fn test_transfer_event_discriminator() {
        let discriminator = TransferEvent::discriminator();
        assert_eq!(discriminator.len(), 8);

        // Should match manual calculation
        let expected = calculate_discriminator("TransferEvent");
        assert_eq!(discriminator, expected);
    }

    #[test]
    fn test_deposit_event_creation() {
        let event = DepositEvent {
            user: "user123".to_string(),
            amount: 5000,
            timestamp: 1234567890,
        };

        assert_eq!(event.user, "user123");
        assert_eq!(event.amount, 5000);
        assert_eq!(event.timestamp, 1234567890);
    }

    #[test]
    fn test_deposit_event_discriminator() {
        let discriminator = DepositEvent::discriminator();
        assert_eq!(discriminator.len(), 8);

        let expected = calculate_discriminator("DepositEvent");
        assert_eq!(discriminator, expected);
    }

    #[test]
    fn test_withdraw_event_creation() {
        let event = WithdrawEvent {
            user: "user456".to_string(),
            amount: 2500,
            timestamp: 9876543210,
        };

        assert_eq!(event.user, "user456");
        assert_eq!(event.amount, 2500);
        assert_eq!(event.timestamp, 9876543210);
    }

    #[test]
    fn test_withdraw_event_discriminator() {
        let discriminator = WithdrawEvent::discriminator();
        assert_eq!(discriminator.len(), 8);

        let expected = calculate_discriminator("WithdrawEvent");
        assert_eq!(discriminator, expected);
    }

    #[test]
    fn test_event_discriminators_unique() {
        let transfer_disc = TransferEvent::discriminator();
        let deposit_disc = DepositEvent::discriminator();
        let withdraw_disc = WithdrawEvent::discriminator();

        assert_ne!(transfer_disc, deposit_disc);
        assert_ne!(transfer_disc, withdraw_disc);
        assert_ne!(deposit_disc, withdraw_disc);
    }

    #[test]
    fn test_borsh_serialization() {
        let event = TransferEvent {
            from: "sender".to_string(),
            to: "receiver".to_string(),
            amount: 1000,
        };

        // Serialize
        let serialized = borsh::to_vec(&event).unwrap();
        assert!(!serialized.is_empty());

        // Deserialize
        let deserialized: TransferEvent = borsh::from_slice(&serialized).unwrap();
        assert_eq!(event, deserialized);
    }

    #[test]
    fn test_serde_serialization() {
        let event = DepositEvent {
            user: "user123".to_string(),
            amount: 5000,
            timestamp: 1234567890,
        };

        // Serialize to JSON
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("user123"));

        // Deserialize from JSON
        let deserialized: DepositEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event, deserialized);
    }

    #[test]
    fn test_event_discriminator_trait() {
        let transfer_disc = <TransferEvent as EventDiscriminator>::discriminator();
        let deposit_disc = <DepositEvent as EventDiscriminator>::discriminator();
        let withdraw_disc = <WithdrawEvent as EventDiscriminator>::discriminator();

        assert_eq!(transfer_disc.len(), 8);
        assert_eq!(deposit_disc.len(), 8);
        assert_eq!(withdraw_disc.len(), 8);
    }
}
