//! Example of an indexer tracking Solana accounts via transaction processing.
//!
//! This example shows how to configure the indexer to process accounts involved in transactions.

use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_indexer::{
    AccountDecoder, EventDiscriminator, Result, SchemaInitializer, SolanaIndexer,
    SolanaIndexerConfigBuilder, config::IndexingMode, types::traits::EventHandler,
};
use solana_sdk::account::Account;
use sqlx::PgPool;

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
        // Example discriminator (e.g., sha256("account:UserProfile")[..8])
        [101, 202, 103, 204, 105, 206, 107, 208]
    }
}

// 3. Implement AccountDecoder
pub struct UserProfileDecoder;

impl AccountDecoder<UserProfile> for UserProfileDecoder {
    fn decode(&self, account: &Account) -> Option<UserProfile> {
        // Typically check owner matches program_id first
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

// 4. Implement EventHandler
pub struct UserProfileHandler;

#[async_trait]
impl EventHandler<UserProfile> for UserProfileHandler {
    async fn handle(&self, event: UserProfile, db: &PgPool, signature: &str) -> Result<()> {
        println!(
            "✅ Found UserProfile in tx {}: {} (Rep: {})",
            signature, event.username, event.reputation
        );

        // Store in Database
        sqlx::query("INSERT INTO users (username, reputation, last_signature) VALUES ($1, $2, $3) ON CONFLICT (username) DO UPDATE SET reputation = $2, last_signature = $3")
            .bind(&event.username)
            .bind(event.reputation as i64)
            .bind(signature)
            .execute(db)
            .await
            .map_err(solana_indexer::utils::error::SolanaIndexerError::DatabaseError)?;

        Ok(())
    }
}

// 5. Implement SchemaInitializer
pub struct UserSchemaInitializer;

#[async_trait]
impl SchemaInitializer for UserSchemaInitializer {
    async fn initialize(&self, db: &PgPool) -> Result<()> {
        println!("🛠️  Initializing User Schema...");
        sqlx::query("CREATE TABLE IF NOT EXISTS users (username TEXT PRIMARY KEY, reputation BIGINT, last_signature TEXT)")
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
    // Use System Program or your specific program ID
    let program_id_str = std::env::var("PROGRAM_ID")
        .unwrap_or_else(|_| "11111111111111111111111111111111".to_string());

    println!("🚀 Starting Account Indexer (Live Mode)");

    // Configure Indexer
    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(rpc_url)
        .with_database(db_url)
        .program_id(program_id_str)
        .with_indexing_mode(IndexingMode::accounts()) // Enable Account Indexing
        .build()?;

    let mut indexer = SolanaIndexer::new(config).await?;

    // Register Components
    indexer.register_schema_initializer(Box::new(UserSchemaInitializer));

    indexer.account_decoder_registry_mut().register(Box::new(
        Box::new(UserProfileDecoder) as Box<dyn AccountDecoder<UserProfile>>
    ));

    indexer.handler_registry_mut().register(
        UserProfile::discriminator(),
        Box::new(Box::new(UserProfileHandler) as Box<dyn EventHandler<UserProfile>>),
    );

    println!("✅ Registered Decoder, Handler, and Schema");
    println!("🔄 Starting Indexer Loop...");

    // Run the standard indexer loop
    indexer.start().await?;

    Ok(())
}
