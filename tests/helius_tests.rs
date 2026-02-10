use solana_indexer::SolanaIndexerConfigBuilder;
use solana_indexer::config::{HeliusNetwork, SourceConfig};

#[test]
fn test_helius_config_mainnet() {
    let api_key = "test-key";
    let config = SolanaIndexerConfigBuilder::new()
        .with_helius(api_key, true)
        .with_database("postgres://localhost")
        .program_id("11111111111111111111111111111111")
        .build()
        .unwrap();

    assert_eq!(
        config.rpc_url(),
        format!("https://mainnet.helius-rpc.com/?api-key={}", api_key)
    );
    assert_eq!(
        config.helius_ws_url(),
        Some(format!("wss://mainnet.helius-rpc.com/?api-key={}", api_key).as_str())
    );

    if let SourceConfig::Helius { network, .. } = config.source {
        assert_eq!(network, HeliusNetwork::Mainnet);
    } else {
        panic!("Expected Helius source");
    }
}

#[test]
fn test_helius_config_devnet() {
    let api_key = "test-key";
    let config = SolanaIndexerConfigBuilder::new()
        .with_helius_network(api_key, HeliusNetwork::Devnet, true)
        .with_database("postgres://localhost")
        .program_id("11111111111111111111111111111111")
        .build()
        .unwrap();

    assert_eq!(
        config.rpc_url(),
        format!("https://devnet.helius-rpc.com/?api-key={}", api_key)
    );
    assert_eq!(
        config.helius_ws_url(),
        Some(format!("wss://devnet.helius-rpc.com/?api-key={}", api_key).as_str())
    );

    if let SourceConfig::Helius { network, .. } = config.source {
        assert_eq!(network, HeliusNetwork::Devnet);
    } else {
        panic!("Expected Helius source");
    }
}
