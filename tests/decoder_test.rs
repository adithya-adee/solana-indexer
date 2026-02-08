use serde_json::json;
use solana_indexer::{Decoder, EventType};
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta, EncodedTransaction,
    EncodedTransactionWithStatusMeta, UiInstruction, UiMessage, UiParsedInstruction,
    UiParsedMessage, UiTransaction, UiTransactionStatusMeta, option_serializer::OptionSerializer,
    parse_instruction::ParsedInstruction,
};

fn create_mock_transaction(
    instructions: Vec<UiInstruction>,
    log_messages: Option<Vec<String>>,
) -> EncodedConfirmedTransactionWithStatusMeta {
    let message = UiMessage::Parsed(UiParsedMessage {
        account_keys: vec![],
        recent_blockhash: "11111111111111111111111111111111".to_string(),
        instructions,
        address_table_lookups: None,
    });

    let transaction = EncodedTransaction::Json(UiTransaction {
        signatures: vec![
            "5j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia7"
                .to_string(),
        ],
        message,
    });

    let meta = UiTransactionStatusMeta {
        err: None,
        status: Ok(()),
        fee: 5000,
        pre_balances: vec![],
        post_balances: vec![],
        inner_instructions: OptionSerializer::None,
        log_messages: log_messages
            .map(OptionSerializer::Some)
            .unwrap_or(OptionSerializer::None),
        pre_token_balances: OptionSerializer::None,
        post_token_balances: OptionSerializer::None,
        rewards: OptionSerializer::None,
        loaded_addresses: OptionSerializer::None,
        return_data: OptionSerializer::None,
        compute_units_consumed: OptionSerializer::Some(12345),
    };

    EncodedConfirmedTransactionWithStatusMeta {
        slot: 123456,
        transaction: EncodedTransactionWithStatusMeta {
            transaction,
            meta: Some(meta),
            version: None,
        },
        block_time: Some(1678888888),
    }
}

#[test]
fn test_decode_parsed_instructions() {
    let decoder = Decoder::new();
    let program_id = "11111111111111111111111111111111".to_string();
    let instructions = vec![UiInstruction::Parsed(UiParsedInstruction::Parsed(
        ParsedInstruction {
            program: "system".to_string(),
            program_id: program_id.clone(),
            parsed: json!({
                "type": "transfer",
                "info": {
                    "source": "src",
                    "destination": "dest",
                    "lamports": 1000
                }
            }),
            stack_height: None,
        },
    ))];

    let tx = create_mock_transaction(instructions, None);
    let decoded = decoder.decode_transaction(&tx).unwrap();

    assert_eq!(decoded.instructions.len(), 1);
    assert_eq!(decoded.instructions[0].program_id, program_id);
    assert_eq!(decoded.instructions[0].instruction_type, "transfer");
    assert_eq!(decoded.instructions[0].index, 0);
}

#[test]
fn test_decode_unknown_instruction_type() {
    let decoder = Decoder::new();
    let program_id = "11111111111111111111111111111111".to_string();
    let instructions = vec![UiInstruction::Parsed(UiParsedInstruction::Parsed(
        ParsedInstruction {
            program: "system".to_string(),
            program_id: program_id.clone(),
            parsed: json!({
                "info": {}
            }),
            stack_height: None,
        },
    ))];

    let tx = create_mock_transaction(instructions, None);
    let decoded = decoder.decode_transaction(&tx).unwrap();

    assert_eq!(decoded.instructions[0].instruction_type, "unknown");
}

#[test]
fn test_decode_transaction_full() {
    let decoder = Decoder::new();
    let program_id = "11111111111111111111111111111111".to_string();

    // Setup instructions
    let instructions = vec![UiInstruction::Parsed(UiParsedInstruction::Parsed(
        ParsedInstruction {
            program: "system".to_string(),
            program_id: program_id.clone(),
            parsed: json!({
                "type": "transfer",
            }),
            stack_height: None,
        },
    ))];

    // Setup logs
    let logs = vec![
        "Program log: Instruction: Transfer".to_string(),
        "Program data: SGVsbG8=".to_string(),
    ];

    let tx = create_mock_transaction(instructions, Some(logs.clone()));

    // Decode
    let decoded = decoder.decode_transaction(&tx).unwrap();

    // Verify basic fields
    assert_eq!(decoded.slot, 123456);
    assert_eq!(decoded.block_time, Some(1678888888));
    assert_eq!(decoded.compute_units_consumed, Some(12345));

    // Verify instructions
    assert_eq!(decoded.instructions.len(), 1);
    assert_eq!(decoded.instructions[0].instruction_type, "transfer");

    // Verify logs and events
    assert_eq!(decoded.logs.len(), 2);
    assert_eq!(decoded.events.len(), 2);

    let log_event = &decoded.events[0];
    assert_eq!(log_event.event_type, EventType::ProgramLog);
    assert_eq!(log_event.data, Some("Instruction: Transfer".to_string()));

    let data_event = &decoded.events[1];
    assert_eq!(data_event.event_type, EventType::ProgramData);
    assert_eq!(data_event.data, Some("SGVsbG8=".to_string()));
}

#[test]
fn test_decode_transaction_no_meta() {
    let decoder = Decoder::new();
    let instructions = vec![];

    // Create transaction without meta (simulating partial data)
    let mut tx = create_mock_transaction(instructions, None);
    tx.transaction.meta = None;

    let decoded = decoder.decode_transaction(&tx).unwrap();

    // Should default to empty logs and None compute units
    assert!(decoded.logs.is_empty());
    assert!(decoded.events.is_empty());
    assert_eq!(decoded.compute_units_consumed, None);
}
