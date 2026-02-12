use serde_json::json;
use solana_indexer::{DecoderRegistry, DynamicInstructionDecoder};
use solana_transaction_status::{
    UiInstruction, UiParsedInstruction, parse_instruction::ParsedInstruction,
};

struct MockDecoder {
    should_succeed: bool,
}

impl DynamicInstructionDecoder for MockDecoder {
    fn decode_dynamic(&self, _instruction: &UiInstruction) -> Option<([u8; 8], Vec<u8>)> {
        if self.should_succeed {
            Some(([1, 2, 3, 4, 5, 6, 7, 8], vec![10, 20, 30]))
        } else {
            None
        }
    }
}

#[test]
fn test_register_and_decode() {
    let mut registry = DecoderRegistry::new();
    let registry_key = "spl-token".to_string();

    registry
        .register(
            registry_key.clone(),
            Box::new(MockDecoder {
                should_succeed: true,
            }),
        )
        .unwrap();

    // Mock an instruction that matches the registered program
    let instruction = UiInstruction::Parsed(UiParsedInstruction::Parsed(ParsedInstruction {
        program: registry_key,
        program_id: "Program1111...".to_string(),
        parsed: json!({}),
        stack_height: None,
    }));

    let events = registry.decode_transaction(&[instruction]);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(events[0].1, vec![10, 20, 30]);
}

#[test]
fn test_decode_no_matching_decoder() {
    let mut registry = DecoderRegistry::new();
    registry
        .register(
            "other-program".to_string(),
            Box::new(MockDecoder {
                should_succeed: true,
            }),
        )
        .unwrap();

    let instruction = UiInstruction::Parsed(UiParsedInstruction::Parsed(ParsedInstruction {
        program: "spl-token".to_string(),
        program_id: "Program1111...".to_string(),
        parsed: json!({}),
        stack_height: None,
    }));

    let events = registry.decode_transaction(&[instruction]);
    assert!(events.is_empty());
}

#[test]
fn test_decode_decoder_returns_none() {
    let mut registry = DecoderRegistry::new();
    registry
        .register(
            "spl-token".to_string(),
            Box::new(MockDecoder {
                should_succeed: false,
            }),
        )
        .unwrap();

    let instruction = UiInstruction::Parsed(UiParsedInstruction::Parsed(ParsedInstruction {
        program: "spl-token".to_string(),
        program_id: "Program1111...".to_string(),
        parsed: json!({}),
        stack_height: None,
    }));

    let events = registry.decode_transaction(&[instruction]);
    assert!(events.is_empty());
}
