//! Decoder registry for managing instruction decoders.

use crate::types::traits::DynamicInstructionDecoder;
use solana_transaction_status::UiInstruction;
use std::collections::HashMap;

/// Registry for managing instruction decoders by program ID.
pub struct DecoderRegistry {
    decoders: HashMap<String, Vec<Box<dyn DynamicInstructionDecoder>>>,
}

impl DecoderRegistry {
    /// Creates a new empty decoder registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            decoders: HashMap::new(),
        }
    }

    /// Registers an instruction decoder for a specific program ID.
    pub fn register(&mut self, program_id: String, decoder: Box<dyn DynamicInstructionDecoder>) {
        self.decoders.entry(program_id).or_default().push(decoder);
    }

    /// Decodes all instructions in a transaction.
    #[must_use]
    pub fn decode_transaction(&self, instructions: &[UiInstruction]) -> Vec<([u8; 8], Vec<u8>)> {
        let mut events = Vec::new();

        for instruction in instructions {
            if let Some(program_id) = Self::extract_program_id(instruction)
                && let Some(decoders) = self.decoders.get(&program_id)
            {
                for decoder in decoders {
                    if let Some(event) = decoder.decode_dynamic(instruction) {
                        events.push(event);
                        break;
                    }
                }
            }
        }

        events
    }

    fn extract_program_id(instruction: &UiInstruction) -> Option<String> {
        match instruction {
            UiInstruction::Parsed(parsed) => match parsed {
                solana_transaction_status::UiParsedInstruction::Parsed(p) => {
                    Some(p.program.clone())
                }
                solana_transaction_status::UiParsedInstruction::PartiallyDecoded(p) => {
                    Some(p.program_id.clone())
                }
            },
            UiInstruction::Compiled(_) => None,
        }
    }
}
impl Default for DecoderRegistry {
    fn default() -> Self {
        Self::new()
    }
}
