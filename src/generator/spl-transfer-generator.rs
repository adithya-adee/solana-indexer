use std::time::Duration;

use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::Pubkey, signature::Keypair, signer::Signer, system_instruction,
    transaction::Transaction,
};
use tokio::time::interval;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables
    dotenvy::dotenv().ok();

    let rpc_url = std::env::var("RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:8899".to_string());
    let connection = RpcClient::new(rpc_url);

    let payer = Keypair::new();
    println!("Payer Pubkey: {}", payer.pubkey());

    println!("Requesting airdrop...");
    let airdrop_amount = 2_000_000_000; // 2 SOL in lamports
    let airdrop_sig = connection
        .request_airdrop(&payer.pubkey(), airdrop_amount)
        .await?;

    // Wait for airdrop confirmation
    loop {
        let confirmed = connection.confirm_transaction(&airdrop_sig).await?;
        if confirmed {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let mut ticker = interval(Duration::from_secs(5));
    println!("Airdrop confirmed.");
    println!("Starting transaction generation loop (every 5s)...");

    loop {
        ticker.tick().await;

        // Generate a random recipient
        let recipient = Pubkey::new_unique();
        let lamports = 10_000_000; // 0.01 SOL to ensure rent exemption

        // Create the transfer instruction
        let transfer_instruction =
            system_instruction::transfer(&payer.pubkey(), &recipient, lamports);

        // Fetch recent blockhash
        let recent_blockhash = connection.get_latest_blockhash().await?;

        // Build and sign the transaction
        let transaction = Transaction::new_signed_with_payer(
            &[transfer_instruction],
            Some(&payer.pubkey()),
            &[&payer],
            recent_blockhash,
        );

        // Send and confirm
        match connection.send_and_confirm_transaction(&transaction).await {
            Ok(sig) => println!("New signature for indexer: {}", sig),
            Err(e) => eprintln!("Failed to send transaction: {}", e),
        }
    }
}
