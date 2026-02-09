use criterion::{Criterion, black_box, criterion_group, criterion_main};
use serde_json::json;
use solana_indexer::Decoder;
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta, EncodedTransaction,
    EncodedTransactionWithStatusMeta, UiInstruction, UiMessage, UiParsedInstruction,
    UiParsedMessage, UiTransaction, UiTransactionStatusMeta, option_serializer::OptionSerializer,
    parse_instruction::ParsedInstruction,
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

fn bench_decode_empty_transaction(c: &mut Criterion) {
    let decoder = Decoder::new();
    let tx = create_mock_transaction(vec![]);

    c.bench_function("decode_empty_transaction", |b| {
        b.iter(|| decoder.decode_transaction(black_box(&tx)))
    });
}

fn bench_decode_single_instruction(c: &mut Criterion) {
    let decoder = Decoder::new();
    let instruction = UiInstruction::Parsed(UiParsedInstruction::Parsed(ParsedInstruction {
        program: "system".to_string(),
        program_id: "11111111111111111111111111111111".to_string(),
        parsed: json!({"type": "transfer"}),
        stack_height: None,
    }));
    let tx = create_mock_transaction(vec![instruction]);

    c.bench_function("decode_single_instruction", |b| {
        b.iter(|| decoder.decode_transaction(black_box(&tx)))
    });
}

fn bench_decode_multiple_instructions(c: &mut Criterion) {
    let decoder = Decoder::new();
    let instructions: Vec<_> = (0..10)
        .map(|i| {
            UiInstruction::Parsed(UiParsedInstruction::Parsed(ParsedInstruction {
                program: "system".to_string(),
                program_id: "11111111111111111111111111111111".to_string(),
                parsed: json!({"type": "transfer", "index": i}),
                stack_height: None,
            }))
        })
        .collect();
    let tx = create_mock_transaction(instructions);

    c.bench_function("decode_10_instructions", |b| {
        b.iter(|| decoder.decode_transaction(black_box(&tx)))
    });
}

criterion_group!(
    benches,
    bench_decode_empty_transaction,
    bench_decode_single_instruction,
    bench_decode_multiple_instructions
);
criterion_main!(benches);
