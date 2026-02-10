//! Example of an indexer tracking Solana accounts.
//!
//! This example demonstrates how to configure and run the Solana Indexer to
//! fetch and decode specific account types.

use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_indexer::{
    AccountDecoder, EventDiscriminator, Result, SchemaInitializer, SolanaIndexer,
    SolanaIndexerConfigBuilder,
};
use solana_sdk::{account::Account, pubkey::Pubkey};
use sqlx::PgPool;
use std::str::FromStr;

// 1. Define your Account structure
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct UserProfile {
    pub discriminator: [u8; 8],
    pub username: String,
    pub reputation: u64,
}

// 2. Implement EventDiscriminator
impl EventDiscriminator for UserProfile {
    fn discriminator() -> [u8; 8] {
        // In Anchor, this is usually sha256("account:UserProfile")[..8]
        // For this example, we use a dummy value
        [101, 202, 103, 204, 105, 206, 107, 208]
    }
}

// 3. Implement AccountDecoder
pub struct UserProfileDecoder;

impl AccountDecoder<UserProfile> for UserProfileDecoder {
    fn decode(&self, account: &Account) -> Option<UserProfile> {
        // Typically check owner matches program_id first
        // Check data length
        if account.data.len() < 8 {
            return None;
        }

        // Check discriminator
        if account.data[0..8] != UserProfile::discriminator() {
            return None;
        }

        // Deserialize
        UserProfile::try_from_slice(&account.data).ok()
    }
}

// 4. Implement SchemaInitializer
pub struct UserSchemaInitializer;

#[async_trait]
impl SchemaInitializer for UserSchemaInitializer {
    async fn initialize(&self, db: &PgPool) -> Result<()> {
        println!("🛠️  Initializing User Schema (Creating 'users' table if not exists)...");
        // Example SQL:
        sqlx::query("CREATE TABLE IF NOT EXISTS users (pubkey TEXT PRIMARY KEY, username TEXT, reputation BIGINT)")
           .execute(db).await?;
        println!("✅ User Schema Initialized");
        Ok(())
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    // Check if env vars are set, otherwise skip or use defaults
    let rpc_url = std::env::var("RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:8899".to_string());
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://user:pass@localhost/db".to_string());

    println!("🚀 Starting Account Indexer Example");

    // Configure Indexer
    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(rpc_url)
        .with_database(db_url)
        .program_id(
            std::env::var("PROGRAM_ID")
                .unwrap_or_else(|_| "11111111111111111111111111111111".to_string()),
        ) // Dummy Program ID
        .build()?;

    let mut indexer = SolanaIndexer::new(config).await?;

    // Register our Schema Initializer
    let initializer = UserSchemaInitializer;
    indexer.register_schema_initializer(Box::new(UserSchemaInitializer));

    // Manually run initialization since we are not calling start()
    initializer.initialize(indexer.storage().pool()).await?;

    // Register our Account Decoder
    indexer.account_decoder_registry_mut().register(Box::new(
        Box::new(UserProfileDecoder) as Box<dyn AccountDecoder<UserProfile>>
    ));

    println!("✅ Registered UserProfile Decoder");

    // Run Account Indexing
    run_account_indexing(&indexer).await?;

    Ok(())
}

async fn run_account_indexing(
    indexer: &SolanaIndexer,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    println!("🔄 Starting Account Indexing Loop...");

    // In a real app, you would use the actual program ID you are indexing
    let program_id = Pubkey::from_str("11111111111111111111111111111111")?;

    // 1. Fetch all accounts for the program
    // We treat fetch failure as Ok(()) for the example to not crash if RPC is down/mocked
    let mut accounts = match indexer.fetcher().get_program_accounts(&program_id).await {
        Ok(accs) => accs,
        Err(e) => {
            eprintln!(
                "⚠️  Failed to fetch program accounts (RPC might be unreachable): {}",
                e
            );
            vec![]
        }
    };

    // 2. Inject Mock Data for Demonstration
    // Since we don't have a real program deployed on mainnet for this example custom struct,
    // we manually inject an account that has the correct data structure to prove the decoder works.
    println!("ℹ️  Injecting mock account for demonstration...");

    let mock_user = UserProfile {
        discriminator: UserProfile::discriminator(),
        username: "solana_dev".to_string(),
        reputation: 9001,
    };
    let mock_data = borsh::to_vec(&mock_user)?;

    let mock_account = Account {
        lamports: 1_000_000,
        data: mock_data,
        owner: program_id,
        executable: false,
        rent_epoch: 0,
    };
    let mock_pubkey = Pubkey::new_unique();

    accounts.push((mock_pubkey, mock_account));

    println!("📊 Processing {} accounts...", accounts.len());

    let pool = indexer.storage().pool();

    // 3. Iterate and Decode
    for (pubkey, account) in accounts {
        let decoded_results = indexer.account_decoder_registry().decode_account(&account);

        for (_discriminator, data) in decoded_results {
            // 4. Deserialize into typed struct
            if let Ok(user_profile) = UserProfile::try_from_slice(&data) {
                println!(
                    "✅ Found UserProfile: {} (Rep: {})",
                    user_profile.username, user_profile.reputation
                );

                // 5. Store in Database
                // This assumes a 'users' table exists.

                // Example SQL (commented out as table doesn't exist in generic setup)
                sqlx::query("INSERT INTO users (pubkey, username, reputation) VALUES ($1, $2, $3) ON CONFLICT (pubkey) DO UPDATE SET reputation = $3")
                    .bind(pubkey.to_string())
                    .bind(&user_profile.username)
                    .bind(user_profile.reputation as i64)
                    .execute(pool)
                    .await?;
                println!("   -> Stored account {}", pubkey);
            }
        }
    }

    println!("✅ Account indexing complete.");
    Ok(())
}
