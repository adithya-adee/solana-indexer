use serde_json::json;
use solana_indexer::Fetcher;
use solana_sdk::signature::Signature;
use std::str::FromStr;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_fetch_transaction_success() {
    let mock_server = MockServer::start().await;
    let fetcher = Fetcher::new(
        mock_server.uri(),
        solana_sdk::commitment_config::CommitmentConfig::confirmed(),
    );

    // Mock version call which RpcClient makes
    Mock::given(method("POST"))
        .and(body_string_contains("getVersion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": { "solana-core": "1.16.7", "feature-set": 0 },
            "id": 1
        })))
        .mount(&mock_server)
        .await;

    let signature_str =
        "5j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia7";
    let signature = Signature::from_str(signature_str).unwrap();

    // Mock response matching what solana-client expects for getTransaction
    let response_body = json!({
        "jsonrpc": "2.0",
        "result": {
            "slot": 123456,
            "blockTime": 1678888888,
            "transaction": {
                "signatures": [signature_str],
                "message": {
                    "accountKeys": [
                        { "pubkey": "11111111111111111111111111111111", "signer": true, "writable": true, "source": "transaction" }
                    ],
                    "instructions": [],
                    "recentBlockhash": "11111111111111111111111111111111"
                }
            },
            "meta": {
                "err": null,
                "status": { "Ok": null },
                "fee": 5000,
                "preBalances": [100000],
                "postBalances": [95000],
                "innerInstructions": [],
                "logMessages": [],
                "preTokenBalances": [],
                "postTokenBalances": [],
                "rewards": []
            }
        },
        "id": 1
    });

    Mock::given(method("POST"))
        .and(body_string_contains("getTransaction"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
        .mount(&mock_server)
        .await;

    let result = fetcher.fetch_transaction(&signature).await;
    assert!(
        result.is_ok(),
        "Failed to fetch transaction: {:?}",
        result.err()
    );

    let tx = result.unwrap();
    assert_eq!(tx.slot, 123456);
    assert_eq!(tx.block_time, Some(1678888888));
}

#[tokio::test]
async fn test_fetch_transaction_not_found() {
    let mock_server = MockServer::start().await;
    let fetcher = Fetcher::new(
        mock_server.uri(),
        solana_sdk::commitment_config::CommitmentConfig::confirmed(),
    );
    let signature = Signature::default();

    let response_body = json!({
        "jsonrpc": "2.0",
        "result": null,
        "id": 1
    });

    Mock::given(method("POST"))
        .and(body_string_contains("getTransaction"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
        .mount(&mock_server)
        .await;

    let result = fetcher.fetch_transaction(&signature).await;
    // solana-client returns an error if result is null for getTransaction
    assert!(result.is_err());
}

#[tokio::test]
async fn test_fetch_transaction_rpc_error() {
    let mock_server = MockServer::start().await;
    let fetcher = Fetcher::new(
        mock_server.uri(),
        solana_sdk::commitment_config::CommitmentConfig::confirmed(),
    );
    let signature = Signature::default();

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock_server)
        .await;

    let result = fetcher.fetch_transaction(&signature).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_fetch_transactions_batch() {
    let mock_server = MockServer::start().await;
    let fetcher = Fetcher::new(
        mock_server.uri(),
        solana_sdk::commitment_config::CommitmentConfig::confirmed(),
    );

    // Mock version call
    Mock::given(method("POST"))
        .and(body_string_contains("getVersion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "result": { "solana-core": "1.16.7", "feature-set": 0 },
            "id": 1
        })))
        .mount(&mock_server)
        .await;

    let sig1_str =
        "5j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia7";
    let sig2_str =
        "2j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia8"; // modified last char

    let sig1 = Signature::from_str(sig1_str).unwrap();
    let sig2 = Signature::from_str(sig2_str).unwrap();
    let signatures = vec![sig1, sig2];

    let response_body = json!({
        "jsonrpc": "2.0",
        "result": {
            "slot": 123456,
            "blockTime": 1678888888,
            "transaction": {
                "signatures": [sig1_str],
                "message": {
                    "accountKeys": [],
                    "instructions": [],
                    "recentBlockhash": "11111111111111111111111111111111"
                }
            },
            "meta": {
                "err": null,
                "status": { "Ok": null },
                "fee": 5000,
                "preBalances": [],
                "postBalances": [],
                "innerInstructions": [],
                "logMessages": [],
                "preTokenBalances": [],
                "postTokenBalances": [],
                "rewards": []
            }
        },
        "id": 1
    });

    // Mock returns success for both
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
        .mount(&mock_server)
        .await;

    let results = fetcher.fetch_transactions(&signatures).await.unwrap();

    assert_eq!(results.len(), 2);
    assert!(results[0].is_ok());
    assert!(results[1].is_ok()); // Both succeed because mock returns valid JSON for both requests
}
