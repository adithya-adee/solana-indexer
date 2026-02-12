//! Account decoder registry for managing account decoders.
//!
//! The `AccountDecoderRegistry` stores account decoders and allows decoding
//! raw `Account` data into typed structures, similar to `DecoderRegistry` for instructions.

use crate::config::RegistryConfig;
use crate::core::registry_metrics::RegistryMetrics;
use crate::types::traits::DynamicAccountDecoder;
use crate::utils::error::{Result, SolanaIndexerError};

/// Registry for managing account decoders.
pub struct AccountDecoderRegistry {
    decoders: Vec<Box<dyn DynamicAccountDecoder>>,
    metrics: RegistryMetrics,
}

impl AccountDecoderRegistry {
    /// Creates a new `AccountDecoderRegistry` with unlimited capacity.
    pub fn new() -> Self {
        Self {
            decoders: Vec::new(),
            metrics: RegistryMetrics::new("AccountDecoder", 0),
        }
    }

    /// Creates a new account decoder registry with a specific capacity limit.
    pub fn new_bounded(config: &RegistryConfig) -> Self {
        Self {
            decoders: Vec::new(),
            metrics: RegistryMetrics::new("AccountDecoder", config.max_account_decoders),
        }
    }

    /// Registers a new account decoder.
    pub fn register(&mut self, decoder: Box<dyn DynamicAccountDecoder>) -> Result<()> {
        if self.metrics.is_full() {
            return Err(SolanaIndexerError::RegistryCapacityExceeded(format!(
                "AccountDecoder registry full (limit: {})",
                self.metrics.capacity_limit
            )));
        }

        self.decoders.push(decoder);
        self.metrics.inc_registered();
        Ok(())
    }

    /// Iterates through all registered decoders and returns the first successful decode.
    ///
    /// Returns a vector of tuples `(discriminator, data)` for all matches if multiple decoders handle it,
    /// or typically just one. For accounts, usually only one decoder matches a given account structure.
    /// However, following the pattern of `DecoderRegistry`, we return a list of matches.
    pub fn decode_account(
        &self,
        account: &solana_sdk::account::Account,
    ) -> Vec<([u8; 8], Vec<u8>)> {
        self.metrics.inc_calls();
        let results: Vec<_> = self
            .decoders
            .iter()
            .filter_map(|decoder| decoder.decode_account_dynamic(account))
            .collect();

        if !results.is_empty() {
            self.metrics.inc_hits();
        }

        results
    }

    /// Returns the metrics for this registry.
    pub fn metrics(&self) -> &RegistryMetrics {
        &self.metrics
    }
}

impl Default for AccountDecoderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::events::EventDiscriminator;
    use crate::types::traits::AccountDecoder;
    use borsh::{BorshDeserialize, BorshSerialize};
    use solana_sdk::account::Account;

    #[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
    struct TestAccount {
        value: u64,
    }

    impl EventDiscriminator for TestAccount {
        fn discriminator() -> [u8; 8] {
            [1, 2, 3, 4, 5, 6, 7, 8]
        }
    }

    struct TestDecoder;

    impl AccountDecoder<TestAccount> for TestDecoder {
        fn decode(&self, account: &Account) -> Option<TestAccount> {
            if account.data.len() >= 8 {
                TestAccount::try_from_slice(&account.data).ok()
            } else {
                None
            }
        }
    }

    #[test]
    fn test_register_and_decode() {
        let mut registry = AccountDecoderRegistry::new();
        registry
            .register(Box::new(
                Box::new(TestDecoder) as Box<dyn crate::types::traits::AccountDecoder<TestAccount>>
            ))
            .unwrap();

        let account = Account {
            lamports: 100,
            data: vec![10, 0, 0, 0, 0, 0, 0, 0], // u64 value 10
            owner: solana_sdk::pubkey::Pubkey::default(),
            executable: false,
            rent_epoch: 0,
        };

        let decoded = registry.decode_account(&account);
        assert_eq!(decoded.len(), 1);
        let (discriminator, data) = &decoded[0];
        assert_eq!(*discriminator, TestAccount::discriminator());

        let event = TestAccount::try_from_slice(data).unwrap();
        assert_eq!(event.value, 10);
    }

    #[test]
    fn test_decode_empty() {
        let registry = AccountDecoderRegistry::new();
        let account = Account::default();
        let decoded = registry.decode_account(&account);
        assert!(decoded.is_empty());
    }
}
