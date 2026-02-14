use solana_indexer_sdk::streams::hybrid::HybridSource;
use solana_indexer_sdk::streams::TransactionSource;
use solana_sdk::pubkey::Pubkey;

#[tokio::test]
async fn test_hybrid_source_creation() {
    let ws_url = "ws://127.0.0.1:8900";
    let rpc_url = "http://127.0.0.1:8899";
    let program_id = Pubkey::new_unique();

    let source = HybridSource::new(ws_url, rpc_url, vec![program_id], 5, 5, 100);

    assert_eq!(source.source_name(), "Hybrid (WS + RPC)");
}
