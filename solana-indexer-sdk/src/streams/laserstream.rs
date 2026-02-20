//! Laserstream (Yellowstone gRPC) stream handler.

use crate::config::SolanaIndexerConfig;
use crate::utils::error::{Result, SolanaIndexerError};
use async_trait::async_trait;
use bincode;
use futures_util::stream::StreamExt;
use solana_account_decoder::parse_token::UiTokenAmount;
use solana_sdk::message::MessageHeader;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::TransactionError;
use solana_transaction_status::{
    option_serializer::OptionSerializer, EncodedConfirmedTransactionWithStatusMeta,
    EncodedTransaction, EncodedTransactionWithStatusMeta, UiCompiledInstruction,
    UiInnerInstructions, UiInstruction, UiMessage, UiRawMessage, UiTransaction,
    UiTransactionStatusMeta, UiTransactionTokenBalance,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;
use yellowstone_grpc_proto::geyser::geyser_client::GeyserClient;
use yellowstone_grpc_proto::geyser::{
    SubscribeRequest, SubscribeRequestFilterTransactions, SubscribeUpdate,
};
use yellowstone_grpc_proto::tonic::transport::{ClientTlsConfig, Endpoint};
use yellowstone_grpc_proto::tonic::Request;

use super::TransactionSource;

/// Laserstream source for acquiring transaction data via gRPC.
pub struct LaserstreamSource {
    receiver: mpsc::Receiver<crate::streams::TransactionEvent>,
}

impl LaserstreamSource {
    /// Creates a new `LaserstreamSource` instance.
    pub async fn new(config: SolanaIndexerConfig) -> Result<Self> {
        let (grpc_url, x_token, reconnect_delay) = match &config.source {
            crate::config::SourceConfig::Laserstream {
                grpc_url,
                x_token,
                reconnect_delay_secs,
            } => (grpc_url.clone(), x_token.clone(), *reconnect_delay_secs),
            _ => {
                return Err(SolanaIndexerError::ConfigError(
                    "Not a Laserstream config".to_string(),
                ));
            }
        };

        let program_ids: Vec<String> = config.program_ids.iter().map(|p| p.to_string()).collect();
        let (sender, receiver) = mpsc::channel(1000); // Buffer size

        // Spawn background task to handle gRPC connection
        tokio::spawn(Self::run_stream(
            grpc_url,
            x_token,
            reconnect_delay,
            program_ids,
            sender,
        ));

        Ok(Self { receiver })
    }

    async fn run_stream(
        grpc_url: String,
        x_token: Option<String>,
        reconnect_delay: u64,
        program_ids: Vec<String>,
        sender: mpsc::Sender<crate::streams::TransactionEvent>,
    ) {
        loop {
            crate::utils::logging::log(
                crate::utils::logging::LogLevel::Info,
                &format!("Connecting to Laserstream gRPC: {grpc_url}"),
            );

            match Self::connect_and_subscribe(&grpc_url, &x_token, &program_ids).await {
                Ok(mut stream) => {
                    crate::utils::logging::log(
                        crate::utils::logging::LogLevel::Success,
                        "Connected to Laserstream gRPC",
                    );

                    while let Some(message) = stream.next().await {
                        match message {
                            Ok(update) => {
                                if let Err(e) = Self::process_update(update, &sender).await {
                                    crate::utils::logging::log_error(
                                        "Laserstream update error",
                                        &e.to_string(),
                                    );
                                }
                            }
                            Err(e) => {
                                crate::utils::logging::log_error(
                                    "gRPC stream error",
                                    &e.to_string(),
                                );
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    crate::utils::logging::log_error(
                        "Failed to connect to Laserstream",
                        &e.to_string(),
                    );
                }
            }

            crate::utils::logging::log(
                crate::utils::logging::LogLevel::Warning,
                &format!("Reconnecting to Laserstream in {reconnect_delay}s..."),
            );
            sleep(Duration::from_secs(reconnect_delay)).await;
        }
    }

    async fn connect_and_subscribe(
        grpc_url: &str,
        x_token: &Option<String>,
        program_ids: &[String],
    ) -> Result<yellowstone_grpc_proto::tonic::Streaming<SubscribeUpdate>> {
        // Create endpoint with TLS config if URL uses https/tls
        let endpoint = Endpoint::from_shared(grpc_url.to_string())
            .map_err(|e| SolanaIndexerError::ConfigError(format!("Invalid URL: {e}")))?
            .tls_config(ClientTlsConfig::new())
            .map_err(|e| SolanaIndexerError::ConfigError(format!("TLS error: {e}")))?;

        let channel = endpoint.connect().await.map_err(|e| {
            SolanaIndexerError::ConnectionError(format!("gRPC connect failed: {e}"))
        })?;

        let token = x_token.clone();
        let mut client = GeyserClient::with_interceptor(channel, move |mut req: Request<()>| {
            if let Some(ref t) = token {
                req.metadata_mut().insert(
                    "x-token",
                    t.parse().unwrap_or_else(|_| "invalid".parse().unwrap()),
                );
            }
            Ok(req)
        });

        // Subscribe to transaction events for the configured program IDs
        let mut transactions = HashMap::new();
        transactions.insert(
            "tx_sub".to_string(),
            SubscribeRequestFilterTransactions {
                vote: Some(false),
                failed: Some(false),
                signature: None,
                account_include: program_ids.to_vec(),
                account_exclude: vec![],
                account_required: vec![],
            },
        );

        let request = SubscribeRequest {
            transactions,
            slots: HashMap::new(),
            accounts: HashMap::new(),
            blocks: HashMap::new(),
            blocks_meta: HashMap::new(),
            entry: HashMap::new(),
            commitment: Some(1), // Confirmed
            accounts_data_slice: vec![],
            ping: None,
            transactions_status: HashMap::new(),
        };

        let stream = client
            .subscribe(futures_util::stream::once(async move { request }))
            .await
            .map_err(|e| SolanaIndexerError::ConnectionError(format!("Subscribe failed: {e}")))?
            .into_inner();

        Ok(stream)
    }

    /// Converts a raw gRPC `SubscribeUpdate` into a `TransactionEvent::FullTransaction`.
    ///
    /// This maps the Yellowstone protobuf types to the `EncodedConfirmedTransactionWithStatusMeta`
    /// structure used throughout the indexer SDK:
    /// - `Transaction.signatures` → base58 strings
    /// - `Message.account_keys` → base58 strings
    /// - `Message.instructions` → `UiCompiledInstruction` list
    /// - `TransactionStatusMeta` → `UiTransactionStatusMeta` (balances, logs, inner ixs, tokens)
    async fn process_update(
        update: SubscribeUpdate,
        sender: &mpsc::Sender<crate::streams::TransactionEvent>,
    ) -> Result<()> {
        let Some(update_msg) = update.update_oneof else {
            return Ok(());
        };

        let yellowstone_grpc_proto::geyser::subscribe_update::UpdateOneof::Transaction(tx_update) =
            update_msg
        else {
            return Ok(()); // Ignore slots, accounts, block meta, ping, etc.
        };

        let slot = tx_update.slot;

        let Some(tx_info) = tx_update.transaction else {
            return Ok(());
        };

        // Extract top-level signature
        let signature = Signature::try_from(tx_info.signature.as_slice())
            .map_err(|e| SolanaIndexerError::DataError(format!("Invalid signature bytes: {e}")))?;

        let Some(tx) = tx_info.transaction else {
            return Ok(());
        };

        let msg = tx.message.as_ref();

        // Convert account_keys bytes → base58 strings
        let account_keys: Vec<String> = msg
            .map(|m| {
                m.account_keys
                    .iter()
                    .map(|k| bs58::encode(k).into_string())
                    .collect()
            })
            .unwrap_or_default();

        // Convert CompiledInstructions
        let instructions: Vec<UiCompiledInstruction> = msg
            .map(|m| {
                m.instructions
                    .iter()
                    .map(|ix| UiCompiledInstruction {
                        program_id_index: ix.program_id_index as u8,
                        accounts: ix.accounts.to_vec(),
                        data: bs58::encode(&ix.data).into_string(),
                        stack_height: None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Address lookup tables (v0 transactions)
        let address_table_lookups: Option<Vec<solana_transaction_status::UiAddressTableLookup>> =
            msg.and_then(|m| {
                if m.address_table_lookups.is_empty() {
                    None
                } else {
                    Some(
                        m.address_table_lookups
                            .iter()
                            .map(|atl| solana_transaction_status::UiAddressTableLookup {
                                account_key: bs58::encode(&atl.account_key).into_string(),
                                writable_indexes: atl.writable_indexes.clone(),
                                readonly_indexes: atl.readonly_indexes.clone(),
                            })
                            .collect(),
                    )
                }
            });

        // Message header
        let num_required_signatures = msg
            .and_then(|m| m.header.as_ref())
            .map(|h| h.num_required_signatures)
            .unwrap_or(0);
        let num_readonly_signed_accounts = msg
            .and_then(|m| m.header.as_ref())
            .map(|h| h.num_readonly_signed_accounts)
            .unwrap_or(0);
        let num_readonly_unsigned_accounts = msg
            .and_then(|m| m.header.as_ref())
            .map(|h| h.num_readonly_unsigned_accounts)
            .unwrap_or(0);
        let recent_blockhash = msg
            .map(|m| bs58::encode(&m.recent_blockhash).into_string())
            .unwrap_or_default();

        // All signatures (first is the transaction signature)
        let sig_strings: Vec<String> = tx
            .signatures
            .iter()
            .map(|s| bs58::encode(s).into_string())
            .collect();

        let encoded_tx = EncodedTransaction::Json(UiTransaction {
            signatures: sig_strings,
            message: UiMessage::Raw(UiRawMessage {
                header: MessageHeader {
                    num_required_signatures: num_required_signatures as u8,
                    num_readonly_signed_accounts: num_readonly_signed_accounts as u8,
                    num_readonly_unsigned_accounts: num_readonly_unsigned_accounts as u8,
                },
                account_keys,
                recent_blockhash,
                instructions,
                address_table_lookups,
            }),
        });

        // Build UiTransactionStatusMeta from the protobuf meta
        let ui_meta: Option<UiTransactionStatusMeta> = tx_info.meta.map(|meta| {
            // Inner instructions — may be absent in older validator versions
            let inner_instructions = if meta.inner_instructions_none {
                OptionSerializer::None
            } else {
                OptionSerializer::Some(
                    meta.inner_instructions
                        .iter()
                        .map(|inner| UiInnerInstructions {
                            index: inner.index as u8,
                            instructions: inner
                                .instructions
                                .iter()
                                .map(|ix| {
                                    UiInstruction::Compiled(UiCompiledInstruction {
                                        program_id_index: ix.program_id_index as u8,
                                        accounts: ix.accounts.to_vec(),
                                        data: bs58::encode(&ix.data).into_string(),
                                        stack_height: ix.stack_height,
                                    })
                                })
                                .collect(),
                        })
                        .collect(),
                )
            };

            // Log messages — may be absent
            let log_messages = if meta.log_messages_none {
                OptionSerializer::None
            } else {
                OptionSerializer::Some(meta.log_messages.clone())
            };

            // Helper to map proto TokenBalance → UiTransactionTokenBalance
            let map_token_balance =
                |tb: &yellowstone_grpc_proto::prelude::TokenBalance| UiTransactionTokenBalance {
                    account_index: tb.account_index as u8,
                    mint: tb.mint.clone(),
                    ui_token_amount: UiTokenAmount {
                        ui_amount: tb.ui_token_amount.as_ref().and_then(|a| {
                            if a.ui_amount == 0.0 {
                                None
                            } else {
                                Some(a.ui_amount)
                            }
                        }),
                        decimals: tb
                            .ui_token_amount
                            .as_ref()
                            .map(|a| a.decimals as u8)
                            .unwrap_or(0),
                        amount: tb
                            .ui_token_amount
                            .as_ref()
                            .map(|a| a.amount.clone())
                            .unwrap_or_default(),
                        ui_amount_string: tb
                            .ui_token_amount
                            .as_ref()
                            .map(|a| a.ui_amount_string.clone())
                            .unwrap_or_default(),
                    },
                    owner: OptionSerializer::Some(tb.owner.clone()),
                    program_id: OptionSerializer::Some(tb.program_id.clone()),
                };

            let pre_token_balances = OptionSerializer::Some(
                meta.pre_token_balances
                    .iter()
                    .map(map_token_balance)
                    .collect::<Vec<_>>(),
            );
            let post_token_balances = OptionSerializer::Some(
                meta.post_token_balances
                    .iter()
                    .map(map_token_balance)
                    .collect::<Vec<_>>(),
            );

            // Deserialize the transaction error (protobuf stores it as bincode bytes)
            let tx_err: Option<TransactionError> =
                meta.err.and_then(|e| bincode::deserialize(&e.err).ok());

            UiTransactionStatusMeta {
                err: tx_err.clone(),
                status: if let Some(tx_err) = tx_err {
                    Err(tx_err)
                } else {
                    Ok(())
                },
                fee: meta.fee,
                pre_balances: meta.pre_balances.clone(),
                post_balances: meta.post_balances.clone(),
                inner_instructions,
                log_messages,
                pre_token_balances,
                post_token_balances,
                rewards: OptionSerializer::Skip,
                loaded_addresses: OptionSerializer::Skip,
                compute_units_consumed: meta
                    .compute_units_consumed
                    .map(OptionSerializer::Some)
                    .unwrap_or(OptionSerializer::Skip),
                return_data: OptionSerializer::Skip,
            }
        });

        // Assemble the full `EncodedConfirmedTransactionWithStatusMeta`
        let confirmed_tx = Arc::new(EncodedConfirmedTransactionWithStatusMeta {
            slot,
            block_time: None, // Not in transaction-level updates; available in block updates
            transaction: EncodedTransactionWithStatusMeta {
                version: None,
                transaction: encoded_tx,
                meta: ui_meta,
            },
        });

        let event = crate::streams::TransactionEvent::FullTransaction {
            signature,
            slot,
            tx: confirmed_tx,
        };

        if sender.send(event).await.is_err() {
            return Err(SolanaIndexerError::InternalError(
                "Laserstream receiver dropped".to_string(),
            ));
        }

        Ok(())
    }
}

#[async_trait]
impl TransactionSource for LaserstreamSource {
    async fn next_batch(&mut self) -> Result<Vec<crate::streams::TransactionEvent>> {
        let mut events = Vec::new();

        // Block until at least one event is available
        if let Some(event) = self.receiver.recv().await {
            events.push(event);
        } else {
            // Channel closed - stream ended
            return Ok(vec![]);
        }

        // Drain any additional buffered events (up to 100 per batch)
        while let Ok(event) = self.receiver.try_recv() {
            events.push(event);
            if events.len() >= 100 {
                break;
            }
        }

        Ok(events)
    }

    fn source_name(&self) -> &'static str {
        "Laserstream (gRPC)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;
    use yellowstone_grpc_proto::geyser::{
        subscribe_update::UpdateOneof, SubscribeUpdateTransaction, SubscribeUpdateTransactionInfo,
    };
    use yellowstone_grpc_proto::prelude::{
        Message, MessageHeader, Transaction, TransactionStatusMeta,
    };

    #[tokio::test]
    async fn test_process_update_success() {
        let (sender, mut receiver) = mpsc::channel(100);

        // Mock data
        let signature = Signature::new_unique();
        let slot = 12345;
        let program_id = Pubkey::new_unique();
        let account = Pubkey::new_unique();

        // Construct a mock SubscribeUpdate
        let update = SubscribeUpdate {
            filters: vec![],
            update_oneof: Some(UpdateOneof::Transaction(SubscribeUpdateTransaction {
                transaction: Some(SubscribeUpdateTransactionInfo {
                    signature: signature.as_ref().to_vec(),
                    is_vote: false,
                    transaction: Some(Transaction {
                        signatures: vec![signature.as_ref().to_vec()],
                        message: Some(Message {
                            header: Some(MessageHeader {
                                num_required_signatures: 1,
                                num_readonly_signed_accounts: 0,
                                num_readonly_unsigned_accounts: 0,
                            }),
                            account_keys: vec![
                                program_id.as_ref().to_vec(),
                                account.as_ref().to_vec(),
                            ],
                            recent_blockhash: vec![0; 32],
                            instructions: vec![
                                yellowstone_grpc_proto::prelude::CompiledInstruction {
                                    program_id_index: 0,
                                    accounts: vec![1],
                                    data: vec![1, 2, 3],
                                },
                            ],
                            versioned: false,
                            address_table_lookups: vec![],
                        }),
                    }),
                    meta: Some(TransactionStatusMeta {
                        err: None,
                        fee: 5000,
                        pre_balances: vec![100, 200],
                        post_balances: vec![90, 200],
                        inner_instructions: vec![],
                        inner_instructions_none: true,
                        log_messages: vec!["log1".to_string()],
                        log_messages_none: false,
                        pre_token_balances: vec![],
                        post_token_balances: vec![],
                        rewards: vec![],
                        loaded_writable_addresses: vec![],
                        loaded_readonly_addresses: vec![],
                        return_data: None,
                        return_data_none: true,
                        compute_units_consumed: Some(100),
                    }),
                    index: 0,
                }),
                slot,
            })),
        };

        // Call process_update
        LaserstreamSource::process_update(update, &sender)
            .await
            .expect("process_update failed");

        // Verify result
        let event = receiver.recv().await.expect("No event received");
        match event {
            crate::streams::TransactionEvent::FullTransaction {
                signature: sig,
                slot: s,
                tx,
            } => {
                assert_eq!(sig, signature);
                assert_eq!(s, slot);
                assert_eq!(tx.slot, slot);

                // Verify transaction details
                if let EncodedTransaction::Json(ui_tx) = &tx.transaction.transaction {
                    assert_eq!(ui_tx.signatures[0], signature.to_string());
                    if let UiMessage::Raw(msg) = &ui_tx.message {
                        assert_eq!(msg.account_keys.len(), 2);
                        assert_eq!(msg.account_keys[0], program_id.to_string());
                        assert_eq!(msg.instructions.len(), 1);
                        assert_eq!(msg.instructions[0].program_id_index, 0);
                    } else {
                        panic!("Expected Raw message");
                    }
                } else {
                    panic!("Expected Json transaction");
                }

                // Verify meta
                let meta = tx.transaction.meta.as_ref().expect("Meta missing");
                assert_eq!(meta.fee, 5000);
                assert_eq!(meta.pre_balances, vec![100, 200]);
                if let OptionSerializer::Some(logs) = &meta.log_messages {
                    assert_eq!(logs.len(), 1);
                } else {
                    panic!("Expected logs");
                }
            }
            _ => panic!("Expected FullTransaction event"),
        }
    }

    #[tokio::test]
    async fn test_process_update_ignore_non_transaction() {
        let (sender, _) = mpsc::channel(100);

        // Mock a non-transaction update (e.g., Slot)
        let update = SubscribeUpdate {
            filters: vec![],
            update_oneof: Some(UpdateOneof::Slot(
                yellowstone_grpc_proto::geyser::SubscribeUpdateSlot {
                    slot: 123,
                    parent: Some(122),
                    status: 1,
                },
            )),
        };

        // Should return Ok(()) but send nothing
        LaserstreamSource::process_update(update, &sender)
            .await
            .expect("Should not error on ignored update");
    }

    #[tokio::test]
    async fn test_process_update_invalid_signature() {
        let (sender, _) = mpsc::channel(100);

        let update = SubscribeUpdate {
            filters: vec![],
            update_oneof: Some(UpdateOneof::Transaction(SubscribeUpdateTransaction {
                transaction: Some(SubscribeUpdateTransactionInfo {
                    signature: vec![0; 5], // Invalid length
                    is_vote: false,
                    transaction: None,
                    meta: None,
                    index: 0,
                }),
                slot: 1,
            })),
        };

        let result = LaserstreamSource::process_update(update, &sender).await;
        assert!(matches!(result, Err(SolanaIndexerError::DataError(_))));
    }
}
