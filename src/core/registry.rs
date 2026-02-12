use crate::config::RegistryConfig;
use crate::core::registry_metrics::RegistryMetrics;
use crate::types::traits::DynamicInstructionDecoder;
use crate::utils::error::{Result, SolanaIndexerError};
use solana_transaction_status::UiInstruction;
use std::collections::HashMap;

/// Registry for managing instruction decoders by program ID.
pub struct DecoderRegistry {
    decoders: HashMap<String, Vec<Box<dyn DynamicInstructionDecoder>>>,
    metrics: RegistryMetrics,
}

impl DecoderRegistry {
    /// Creates a new empty decoder registry with unlimited capacity.
    #[must_use]
    pub fn new() -> Self {
        Self {
            decoders: HashMap::new(),
            metrics: RegistryMetrics::new("InstructionDecoder", 0),
        }
    }

    /// Creates a new decoder registry with a specific capacity limit.
    ///
    /// # Arguments
    ///
    /// * `config` - Registry configuration containing limits
    pub fn new_bounded(config: &RegistryConfig) -> Self {
        Self {
            decoders: HashMap::new(),
            metrics: RegistryMetrics::new("InstructionDecoder", config.max_decoder_programs),
        }
    }

    /// Registers an instruction decoder for a specific program ID.
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::RegistryCapacityExceeded` if the registry is full
    /// and a new program ID is being added.
    pub fn register(
        &mut self,
        program_id: String,
        decoder: Box<dyn DynamicInstructionDecoder>,
    ) -> Result<()> {
        // specific check: if key doesn't exist and we are full, error
        if !self.decoders.contains_key(&program_id) && self.metrics.is_full() {
            return Err(SolanaIndexerError::RegistryCapacityExceeded(format!(
                "InstructionDecoder registry full (limit: {})",
                self.metrics.capacity_limit
            )));
        }

        self.decoders.entry(program_id).or_default().push(decoder);
        self.metrics.inc_registered();
        Ok(())
    }

    /// Decodes all instructions in a transaction.
    #[must_use]
    pub fn decode_transaction(&self, instructions: &[UiInstruction]) -> Vec<([u8; 8], Vec<u8>)> {
        let mut events = Vec::new();

        for instruction in instructions {
            // Count every instruction processed as a "call" opportunity
            self.metrics.inc_calls();

            if let Some(program_id) = Self::extract_program_id(instruction)
                && let Some(decoders) = self.decoders.get(&program_id)
            {
                for decoder in decoders {
                    if let Some(event) = decoder.decode_dynamic(instruction) {
                        events.push(event);
                        self.metrics.inc_hits();
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

    /// Returns the metrics for this registry.
    pub fn metrics(&self) -> &RegistryMetrics {
        &self.metrics
    }
}
impl Default for DecoderRegistry {
    fn default() -> Self {
        Self::new()
    }
}
