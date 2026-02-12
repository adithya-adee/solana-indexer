use borsh::{BorshDeserialize, BorshSerialize};
use solana_indexer::{
    core::account_registry::AccountDecoderRegistry,
    types::{events::EventDiscriminator, traits::AccountDecoder},
};
use solana_sdk::account::Account;
use solana_sdk::pubkey::Pubkey;

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
struct MockUserAccount {
    discriminator: [u8; 8],
    user_id: u64,
}

impl EventDiscriminator for MockUserAccount {
    fn discriminator() -> [u8; 8] {
        [10, 20, 30, 40, 50, 60, 70, 80]
    }
}

struct MockUserAccountDecoder;

impl AccountDecoder<MockUserAccount> for MockUserAccountDecoder {
    fn decode(&self, account: &Account) -> Option<MockUserAccount> {
        if account.data.len() < 8 {
            return None;
        }

        let mut data = account.data.as_slice();
        MockUserAccount::deserialize(&mut data)
            .ok()
            .filter(|acc| acc.discriminator == MockUserAccount::discriminator())
    }
}

#[test]
fn test_account_registry_workflow() {
    let mut registry = AccountDecoderRegistry::new();

    // Register the decoder
    registry
        .register(Box::new(
            Box::new(MockUserAccountDecoder) as Box<dyn AccountDecoder<MockUserAccount>>
        ))
        .unwrap();

    // Create a mock account with correct data
    let account_data = MockUserAccount {
        discriminator: MockUserAccount::discriminator(),
        user_id: 12345,
    };
    let serialized_data = borsh::to_vec(&account_data).unwrap();

    let account = Account {
        lamports: 1000,
        data: serialized_data,
        owner: Pubkey::new_unique(),
        executable: false,
        rent_epoch: 0,
    };

    // Decode
    let results = registry.decode_account(&account);
    assert_eq!(results.len(), 1);

    let (disc, data) = &results[0];
    assert_eq!(*disc, MockUserAccount::discriminator());

    let decoded_event = MockUserAccount::try_from_slice(data).unwrap();
    assert_eq!(decoded_event.user_id, 12345);
}

#[test]
fn test_account_registry_invalid_data() {
    let mut registry = AccountDecoderRegistry::new();
    registry
        .register(Box::new(
            Box::new(MockUserAccountDecoder) as Box<dyn AccountDecoder<MockUserAccount>>
        ))
        .unwrap();

    // Account with wrong discriminator
    let account_data = MockUserAccount {
        discriminator: [0; 8],
        user_id: 12345,
    };
    let serialized_data = borsh::to_vec(&account_data).unwrap();

    let account = Account {
        lamports: 1000,
        data: serialized_data,
        owner: Pubkey::new_unique(),
        executable: false,
        rent_epoch: 0,
    };

    let results = registry.decode_account(&account);
    assert!(results.is_empty());
}
