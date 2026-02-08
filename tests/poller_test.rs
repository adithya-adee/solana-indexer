use serde_json::json;
use solana_indexer::{Poller, SolanaIndexerConfigBuilder};
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn setup_rpc_mocks(mock_server: &MockServer) {
    // Mock getVersion
    Mock::given(method("POST"))
        .and(body_string_contains("getVersion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": { "solana-core": "1.16.7", "feature-set": 0 },
            "id": 1
        })))
        .mount(mock_server)
        .await;
}

#[tokio::test]
async fn test_poller_fetch_new_signatures() {
    let mock_server = MockServer::start().await;
    setup_rpc_mocks(&mock_server).await;

    let program_id_str = "11111111111111111111111111111111";
    let sig_str1 =
        "5j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia7";
    let sig_str2 =
        "2j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia8";

    // Mock getSignaturesForAddress
    Mock::given(method("POST"))
        .and(body_string_contains("getSignaturesForAddress"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": [
                {
                    "signature": sig_str1,
                    "slot": 123456,
                    "err": null,
                    "memo": null,
                    "blockTime": 1678888888
                },
                {
                    "signature": sig_str2,
                    "slot": 123455,
                    "err": null,
                    "memo": null,
                    "blockTime": 1678888880
                }
            ],
            "id": 1
        })))
        .mount(&mock_server)
        .await;

    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(mock_server.uri())
        .with_database("postgresql://mock/db")
        .program_id(program_id_str)
        .with_batch_size(10)
        .build()
        .unwrap();

    let mut poller = Poller::new(config);

    // Fetch signatures
    let signatures = poller.fetch_new_signatures().await.unwrap();

    assert_eq!(signatures.len(), 2);
    assert_eq!(signatures[0].to_string(), sig_str1);
    assert_eq!(signatures[1].to_string(), sig_str2);
}

#[tokio::test]
async fn test_poller_fetch_empty() {
    let mock_server = MockServer::start().await;
    setup_rpc_mocks(&mock_server).await;

    Mock::given(method("POST"))
        .and(body_string_contains("getSignaturesForAddress"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": [],
            "id": 1
        })))
        .mount(&mock_server)
        .await;

    let config = SolanaIndexerConfigBuilder::new()
        .with_rpc(mock_server.uri())
        .with_database("postgresql://mock/db")
        .program_id("11111111111111111111111111111111")
        .build()
        .unwrap();

    let mut poller = Poller::new(config);
    let signatures = poller.fetch_new_signatures().await.unwrap();
    assert!(signatures.is_empty());
}
