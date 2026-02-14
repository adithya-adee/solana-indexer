use criterion::{black_box, criterion_group, criterion_main, Criterion};
use serde_json::json;
use solana_indexer_sdk::core::decoder::Decoder;
use solana_transaction_status::{
    option_serializer::OptionSerializer, EncodedConfirmedTransactionWithStatusMeta,
    EncodedTransaction, EncodedTransactionWithStatusMeta, UiInstruction, UiMessage,
    UiParsedInstruction, UiParsedMessage, UiTransaction, UiTransactionStatusMeta,
};

fn create_mock_transaction(
    instructions: Vec<UiInstruction>,
) -> EncodedConfirmedTransactionWithStatusMeta {
    EncodedConfirmedTransactionWithStatusMeta {
        slot: 123456,
        block_time: Some(1678888888),
        transaction: EncodedTransactionWithStatusMeta {
            version: None,
            transaction: EncodedTransaction::Json(UiTransaction {
                signatures: vec!["sig1".to_string()],
                message: UiMessage::Parsed(UiParsedMessage {
                    account_keys: vec![],
                    recent_blockhash: "hash".to_string(),
                    instructions,
                    address_table_lookups: None,
                }),
            }),
            meta: Some(UiTransactionStatusMeta {
                err: None,
                status: Ok(()),
                fee: 5000,
                pre_balances: vec![],
                post_balances: vec![],
                inner_instructions: OptionSerializer::None,
                log_messages: OptionSerializer::None,
                pre_token_balances: OptionSerializer::None,
                post_token_balances: OptionSerializer::None,
                rewards: OptionSerializer::None,
                loaded_addresses: OptionSerializer::None,
                return_data: OptionSerializer::None,
                compute_units_consumed: OptionSerializer::None,
            }),
        },
    }
}

fn decoder_benchmark(c: &mut Criterion) {
    let decoder = Decoder::new();

    // Setup input data
    let data = vec![0u8; 32];
    let instruction = UiInstruction::Parsed(UiParsedInstruction::Parsed(
        solana_transaction_status::parse_instruction::ParsedInstruction {
            program: "system".to_string(),
            program_id: "11111111111111111111111111111111".to_string(),
            parsed: json!({"type": "transfer", "data": data}),
            stack_height: None,
        },
    ));

    let tx = create_mock_transaction(vec![instruction.clone()]);
    let large_tx = create_mock_transaction(vec![instruction; 100]); // Stress test with 100 instructions

    let mut group = c.benchmark_group("decoder");

    group.bench_function("decode_single_instruction", |b| {
        b.iter(|| {
            decoder.decode_transaction(black_box(&tx)).unwrap();
        })
    });

    group.bench_function("decode_100_instructions", |b| {
        b.iter(|| {
            decoder.decode_transaction(black_box(&large_tx)).unwrap();
        })
    });

    group.finish();
}

criterion_group!(benches, decoder_benchmark);
criterion_main!(benches);
