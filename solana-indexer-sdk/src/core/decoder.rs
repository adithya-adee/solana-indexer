//! Transaction decoding module for `SolanaIndexer`.
//!
//! This module is responsible for parsing raw transaction data into structured,
//! type-safe Rust objects. It supports IDL-driven decoding for program-specific
//! events and instructions, as well as common Solana instruction types.

use crate::types::events::{EventType, ParsedEvent};
use crate::utils::error::{Result, SolanaIndexerError};
use solana_sdk::pubkey::Pubkey;
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta, EncodedTransaction, UiInstruction, UiMessage,
    UiParsedInstruction,
};
use std::collections::HashMap;

// pub mod registry; // Removed, now a sibling in core
// pub use registry::DecoderRegistry; // Removed, exported from core/mod.rs

/// Transaction decoder for parsing Solana transaction data.
///
/// The `Decoder` handles the transformation of raw transaction data into
/// structured, type-safe Rust objects. It supports:
/// - IDL-based event decoding (future integration)
/// - Event log parsing with discriminator support
/// - Common Solana instruction types (SPL Token, System Program)
///
/// # Example
///
/// ```no_run
/// # use solana_indexer_sdk::Decoder;
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let decoder = Decoder::new();
///
/// // Parse event logs from a transaction
/// let logs = vec!["Program log: Instruction: Transfer".to_string()];
/// let events = decoder.parse_event_logs(&logs)?;
/// # Ok(())
/// # }
/// ```
pub struct Decoder {
    /// Event discriminators for identifying event types
    /// Maps discriminator (8-byte hash) to event type name
    event_discriminators: HashMap<[u8; 8], String>,
}

impl Decoder {
    /// Creates a new `Decoder` instance.
    ///
    /// # Example
    ///
    /// ```
    /// # use solana_indexer_sdk::Decoder;
    /// let decoder = Decoder::new();
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            event_discriminators: HashMap::new(),
        }
    }

    /// Registers an event discriminator for a specific event type.
    ///
    /// Event discriminators are 8-byte hashes used to identify event types
    /// in transaction logs. This is typically derived from the event name
    /// in the IDL.
    ///
    /// # Arguments
    ///
    /// * `discriminator` - The 8-byte discriminator hash
    /// * `event_name` - The name of the event type
    ///
    /// # Example
    ///
    /// ```
    /// # use solana_indexer_sdk::Decoder;
    /// let mut decoder = Decoder::new();
    /// decoder.register_event_discriminator([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08], "TransferEvent");
    /// ```
    pub fn register_event_discriminator(&mut self, discriminator: [u8; 8], event_name: &str) {
        self.event_discriminators
            .insert(discriminator, event_name.to_string());
    }

    /// Parses event logs from a transaction.
    ///
    /// This method extracts and categorizes event logs from transaction log messages.
    /// It identifies:
    /// - Program invocations
    /// - Data logs (base64-encoded event data)
    /// - Instruction logs
    /// - Error messages
    ///
    /// # Arguments
    ///
    /// * `logs` - Transaction log messages
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::DecodingError` if log parsing fails.
    ///
    /// # Returns
    ///
    /// A vector of parsed event logs with their types and data.
    ///
    /// # Example
    ///
    /// ```
    /// # use solana_indexer_sdk::Decoder;
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let decoder = Decoder::new();
    /// let logs = vec![
    ///     "Program 11111111111111111111111111111111 invoke [1]".to_string(),
    ///     "Program log: Instruction: Transfer".to_string(),
    ///     "Program 11111111111111111111111111111111 success".to_string(),
    /// ];
    ///
    /// let events = decoder.parse_event_logs(&logs)?;
    /// println!("Found {} events", events.len());
    /// # Ok(())
    /// # }
    /// ```
    /// Parses event logs from a transaction.
    ///
    /// This method extracts and categorizes event logs from transaction log messages.
    /// It maintains a program ID stack to correctly associate logs and data with
    /// the program that emitted them.
    ///
    /// # Arguments
    ///
    /// * `logs` - Transaction log messages
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::DecodingError` if log parsing fails.
    ///
    /// # Returns
    ///
    /// A vector of parsed event logs with their types and data.
    pub fn parse_event_logs(&self, logs: &[String]) -> Result<Vec<ParsedEvent>> {
        let mut events = Vec::new();
        let mut program_stack: Vec<Pubkey> = Vec::new();

        for log in logs {
            // Check for Program invoke
            if log.contains("invoke [") {
                if let Some(program_id) = Self::extract_program_id(log) {
                    program_stack.push(program_id);
                    events.push(ParsedEvent {
                        event_type: EventType::ProgramInvoke,
                        program_id: Some(program_id),
                        data: None,
                    });
                }
                continue;
            }

            // Check for Program result (success or failed)
            if log.contains(" success") || log.contains(" failed") {
                // We could verify the ID matches, but for now just pop
                if Self::extract_program_id_from_result(log).is_some() {
                    program_stack.pop();
                }
                continue;
            }

            // Program data
            if let Some(stripped) = log.strip_prefix("Program data: ") {
                let current_program = program_stack.last().copied();
                events.push(ParsedEvent {
                    event_type: EventType::ProgramData,
                    program_id: current_program,
                    data: Some(stripped.to_string()),
                });
                continue;
            }

            // Program log
            if let Some(stripped) = log.strip_prefix("Program log: ") {
                let current_program = program_stack.last().copied();
                events.push(ParsedEvent {
                    event_type: EventType::ProgramLog,
                    program_id: current_program,
                    data: Some(stripped.to_string()),
                });
                continue;
            }
        }

        Ok(events)
    }

    /// Extracts program ID from a result log (success/failed).
    fn extract_program_id_from_result(log: &str) -> Option<Pubkey> {
        let parts: Vec<&str> = log.split_whitespace().collect();
        if parts.len() >= 2 && parts[0] == "Program" {
            parts[1].parse().ok()
        } else {
            None
        }
    }

    /// Parses a single log message.
    #[allow(dead_code)]
    fn parse_single_log(log: &str) -> Option<ParsedEvent> {
        // Program invocation
        #[allow(clippy::collapsible_if)]
        if log.contains("invoke [") {
            if let Some(program_id) = Self::extract_program_id(log) {
                return Some(ParsedEvent {
                    event_type: EventType::ProgramInvoke,
                    program_id: Some(program_id),
                    data: None,
                });
            }
        }

        // Program data log (potential event)
        if log.starts_with("Program data: ") {
            let data = log.strip_prefix("Program data: ").unwrap_or("");
            return Some(ParsedEvent {
                event_type: EventType::ProgramData,
                program_id: None,
                data: Some(data.to_string()),
            });
        }

        // Program log (instruction or custom message)
        if log.starts_with("Program log: ") {
            let message = log.strip_prefix("Program log: ").unwrap_or("");
            return Some(ParsedEvent {
                event_type: EventType::ProgramLog,
                program_id: None,
                data: Some(message.to_string()),
            });
        }

        // No event found in this log
        None
    }

    /// Extracts program ID from an invocation log.
    fn extract_program_id(log: &str) -> Option<Pubkey> {
        // Format: "Program <pubkey> invoke [<depth>]"
        let parts: Vec<&str> = log.split_whitespace().collect();
        if parts.len() >= 2 && parts[0] == "Program" {
            parts[1].parse().ok()
        } else {
            None
        }
    }

    /// Decodes a transaction into structured data.
    ///
    /// This method extracts all relevant information from a transaction:
    /// - Instructions and their data
    /// - Account keys involved
    /// - Event logs
    /// - Transaction metadata
    ///
    /// # Arguments
    ///
    /// * `transaction` - The encoded transaction with status metadata
    ///
    /// # Errors
    ///
    /// Returns `SolanaIndexerError::DecodingError` if:
    /// - The transaction format is invalid
    /// - Required fields are missing
    /// - Instruction data cannot be parsed
    ///
    /// # Returns
    ///
    /// A `DecodedTransaction` containing all parsed transaction data.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use solana_indexer_sdk::{Decoder, Fetcher};
    /// # use solana_sdk::signature::Signature;
    /// # use solana_sdk::commitment_config::CommitmentConfig;
    /// # use std::str::FromStr;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let fetcher = Fetcher::new("http://127.0.0.1:8899", CommitmentConfig::confirmed());
    /// let decoder = Decoder::new();
    ///
    /// let sig = Signature::from_str("5j7s6NiJS3JAkvgkoc18WVAsiSaci2pxB2A6ueCJP4tprA2TFg9wSyTLeYouxPBJEMzJinENTkpA52YStRW5Dia7")?;
    /// let transaction = fetcher.fetch_transaction(&sig).await?;
    /// let decoded = decoder.decode_transaction(&transaction)?;
    ///
    /// println!("Transaction has {} instructions", decoded.instructions.len());
    /// # Ok(())
    /// # }
    /// ```
    pub fn decode_transaction(
        &self,
        transaction: &EncodedConfirmedTransactionWithStatusMeta,
    ) -> Result<DecodedTransaction> {
        // Extract logs from metadata
        let logs = transaction
            .transaction
            .meta
            .as_ref()
            .and_then(|meta| {
                // Handle the OptionSerializer type
                match &meta.log_messages {
                    solana_transaction_status::option_serializer::OptionSerializer::Some(logs) => {
                        Some(logs.clone())
                    }
                    _ => None,
                }
            })
            .unwrap_or_default();

        // Parse events from logs
        let events = self.parse_event_logs(&logs)?;

        // Extract instructions
        let instructions = Self::extract_instructions(&transaction.transaction.transaction)?;

        // Extract compute units consumed
        let compute_units_consumed = transaction.transaction.meta.as_ref().and_then(|meta| {
            // Handle the OptionSerializer type
            match meta.compute_units_consumed {
                solana_transaction_status::option_serializer::OptionSerializer::Some(units) => {
                    Some(units)
                }
                _ => None,
            }
        });

        Ok(DecodedTransaction {
            slot: transaction.slot,
            block_time: transaction.block_time,
            instructions,
            events,
            logs,
            compute_units_consumed,
        })
    }

    /// Extracts instructions from an encoded transaction.
    fn extract_instructions(transaction: &EncodedTransaction) -> Result<Vec<InstructionInfo>> {
        let mut instructions = Vec::new();

        // Decode the transaction to access its message
        if let EncodedTransaction::Json(ui_transaction) = transaction {
            match &ui_transaction.message {
                UiMessage::Parsed(parsed_msg) => {
                    for (idx, instruction) in parsed_msg.instructions.iter().enumerate() {
                        match instruction {
                            UiInstruction::Parsed(UiParsedInstruction::Parsed(parsed)) => {
                                instructions.push(InstructionInfo {
                                    program_id: parsed.program_id.clone(),
                                    instruction_type: parsed
                                        .parsed
                                        .get("type")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("unknown")
                                        .to_string(),
                                    index: idx,
                                });
                            }
                            UiInstruction::Compiled(compiled) => {
                                // For compiled instructions, we can't easily get the program ID
                                // without the account keys, so we'll use a placeholder
                                instructions.push(InstructionInfo {
                                    program_id: format!("index:{}", compiled.program_id_index),
                                    instruction_type: "compiled".to_string(),
                                    index: idx,
                                });
                            }
                            UiInstruction::Parsed(_) => {}
                        }
                    }
                }
                UiMessage::Raw(raw_msg) => {
                    for (idx, instruction) in raw_msg.instructions.iter().enumerate() {
                        let program_id_index = instruction.program_id_index as usize;
                        let program_id = raw_msg
                            .account_keys
                            .get(program_id_index)
                            .ok_or_else(|| {
                                SolanaIndexerError::DecodingError(format!(
                                    "Invalid program_id_index: {program_id_index}"
                                ))
                            })?
                            .clone();

                        instructions.push(InstructionInfo {
                            program_id,
                            instruction_type: "raw".to_string(),
                            index: idx,
                        });
                    }
                }
            }
        }

        Ok(instructions)
    }
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Decoded transaction data.
#[derive(Debug, Clone)]
pub struct DecodedTransaction {
    /// Slot number
    pub slot: u64,
    /// Block timestamp
    pub block_time: Option<i64>,
    /// Parsed instructions
    pub instructions: Vec<InstructionInfo>,
    /// Parsed events
    pub events: Vec<ParsedEvent>,
    /// Raw log messages
    pub logs: Vec<String>,
    /// Compute units consumed
    pub compute_units_consumed: Option<u64>,
}

/// Information about a transaction instruction.
#[derive(Debug, Clone)]
pub struct InstructionInfo {
    /// Program ID that executed the instruction
    pub program_id: String,
    /// Type of instruction (e.g., "transfer", "createAccount")
    pub instruction_type: String,
    /// Index of the instruction in the transaction
    pub index: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decoder_creation() {
        let decoder = Decoder::new();
        assert!(decoder.event_discriminators.is_empty());
    }

    #[test]
    fn test_register_event_discriminator() {
        let mut decoder = Decoder::new();
        let discriminator = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        decoder.register_event_discriminator(discriminator, "TestEvent");

        assert_eq!(
            decoder.event_discriminators.get(&discriminator),
            Some(&"TestEvent".to_string())
        );
    }

    #[test]
    fn test_parse_program_invoke_log() {
        let log = "Program 11111111111111111111111111111111 invoke [1]";

        let event = Decoder::parse_single_log(log);
        assert!(event.is_some());

        let event = event.unwrap();
        assert_eq!(event.event_type, EventType::ProgramInvoke);
        assert!(event.program_id.is_some());
    }

    #[test]
    fn test_parse_program_data_log() {
        let log = "Program data: SGVsbG8gV29ybGQ=";

        let event = Decoder::parse_single_log(log);
        assert!(event.is_some());

        let event = event.unwrap();
        assert_eq!(event.event_type, EventType::ProgramData);
        assert_eq!(event.data, Some("SGVsbG8gV29ybGQ=".to_string()));
    }

    #[test]
    fn test_parse_program_log() {
        let log = "Program log: Instruction: Transfer";

        let event = Decoder::parse_single_log(log);
        assert!(event.is_some());

        let event = event.unwrap();
        assert_eq!(event.event_type, EventType::ProgramLog);
        assert_eq!(event.data, Some("Instruction: Transfer".to_string()));
    }

    #[test]
    fn test_parse_event_logs() {
        let decoder = Decoder::new();
        let logs = vec![
            "Program 11111111111111111111111111111111 invoke [1]".to_string(),
            "Program log: Instruction: Transfer".to_string(),
            "Program data: SGVsbG8gV29ybGQ=".to_string(),
            "Program 11111111111111111111111111111111 success".to_string(),
        ];

        let events = decoder.parse_event_logs(&logs).unwrap();
        assert_eq!(events.len(), 3); // invoke, log, data
    }
}
