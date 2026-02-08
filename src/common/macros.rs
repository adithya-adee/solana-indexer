//! Macro utilities for `SolanaIndexer`.
//!
//! This module provides utilities for IDL-based code generation, including
//! parsing IDL files and generating Rust types. In a full implementation,
//! this would include procedural macros for automatic code generation.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents a Solana program IDL (Interface Definition Language).
///
/// This structure mirrors the Anchor IDL format, allowing for parsing
/// and processing of program interfaces.
///
/// # Example
///
/// ```
/// use solana_indexer::Idl;
///
/// let idl_json = r#"{
///     "version": "0.1.0",
///     "name": "my_program",
///     "instructions": [],
///     "accounts": [],
///     "events": []
/// }"#;
///
/// let idl: Idl = serde_json::from_str(idl_json).unwrap();
/// assert_eq!(idl.name, "my_program");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Idl {
    /// IDL version
    pub version: String,
    /// Program name
    pub name: String,
    /// Program instructions
    #[serde(default)]
    pub instructions: Vec<IdlInstruction>,
    /// Program accounts
    #[serde(default)]
    pub accounts: Vec<IdlAccount>,
    /// Program events
    #[serde(default)]
    pub events: Vec<IdlEvent>,
}

/// Represents an instruction in the IDL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlInstruction {
    /// Instruction name
    pub name: String,
    /// Instruction arguments
    #[serde(default)]
    pub args: Vec<IdlField>,
    /// Instruction accounts
    #[serde(default)]
    pub accounts: Vec<IdlAccountItem>,
}

/// Represents an account in the IDL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlAccount {
    /// Account name
    pub name: String,
    /// Account type definition
    #[serde(rename = "type")]
    pub ty: IdlTypeDefinition,
}

/// Represents an event in the IDL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlEvent {
    /// Event name
    pub name: String,
    /// Event fields
    pub fields: Vec<IdlField>,
}

/// Represents a field in an instruction, account, or event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlField {
    /// Field name
    pub name: String,
    /// Field type
    #[serde(rename = "type")]
    pub ty: IdlType,
}

/// Represents an account item in an instruction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlAccountItem {
    /// Account name
    pub name: String,
    /// Whether the account is mutable
    #[serde(rename = "isMut")]
    pub is_mut: bool,
    /// Whether the account is a signer
    #[serde(rename = "isSigner")]
    pub is_signer: bool,
}

/// Represents a type definition for accounts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdlTypeDefinition {
    /// Type kind
    pub kind: String,
    /// Type fields
    #[serde(default)]
    pub fields: Vec<IdlField>,
}

/// Represents a type in the IDL.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IdlType {
    /// Simple type (e.g., "u64", "string")
    Simple(String),
    /// Complex type with additional metadata
    Complex(HashMap<String, serde_json::Value>),
}

impl Idl {
    /// Parses an IDL from JSON string.
    ///
    /// # Arguments
    ///
    /// * `json` - IDL JSON string
    ///
    /// # Errors
    ///
    /// Returns error if JSON parsing fails.
    ///
    /// # Example
    ///
    /// ```
    /// use solana_indexer::Idl;
    ///
    /// let idl_json = r#"{
    ///     "version": "0.1.0",
    ///     "name": "my_program",
    ///     "instructions": [],
    ///     "accounts": [],
    ///     "events": []
    /// }"#;
    ///
    /// let idl = Idl::parse(idl_json).unwrap();
    /// assert_eq!(idl.name, "my_program");
    /// ```
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Gets all event names from the IDL.
    ///
    /// # Example
    ///
    /// ```
    /// use solana_indexer::Idl;
    ///
    /// let idl_json = r#"{
    ///     "version": "0.1.0",
    ///     "name": "my_program",
    ///     "instructions": [],
    ///     "accounts": [],
    ///     "events": [
    ///         {"name": "TransferEvent", "fields": []},
    ///         {"name": "DepositEvent", "fields": []}
    ///     ]
    /// }"#;
    ///
    /// let idl = Idl::parse(idl_json).unwrap();
    /// let event_names = idl.event_names();
    /// assert_eq!(event_names, vec!["TransferEvent", "DepositEvent"]);
    /// ```
    #[must_use]
    pub fn event_names(&self) -> Vec<String> {
        self.events.iter().map(|e| e.name.clone()).collect()
    }

    /// Gets all instruction names from the IDL.
    ///
    /// # Example
    ///
    /// ```
    /// use solana_indexer::Idl;
    ///
    /// let idl_json = r#"{
    ///     "version": "0.1.0",
    ///     "name": "my_program",
    ///     "instructions": [
    ///         {"name": "transfer", "args": [], "accounts": []},
    ///         {"name": "deposit", "args": [], "accounts": []}
    ///     ],
    ///     "accounts": [],
    ///     "events": []
    /// }"#;
    ///
    /// let idl = Idl::parse(idl_json).unwrap();
    /// let instruction_names = idl.instruction_names();
    /// assert_eq!(instruction_names, vec!["transfer", "deposit"]);
    /// ```
    #[must_use]
    pub fn instruction_names(&self) -> Vec<String> {
        self.instructions.iter().map(|i| i.name.clone()).collect()
    }
}

/// Generates Rust type name from IDL type.
///
/// Converts IDL type strings to their Rust equivalents.
///
/// # Arguments
///
/// * `idl_type` - IDL type string
///
/// # Returns
///
/// Rust type string
///
/// # Example
///
/// ```
/// use solana_indexer::idl_type_to_rust;
///
/// assert_eq!(idl_type_to_rust("u64"), "u64");
/// assert_eq!(idl_type_to_rust("string"), "String");
/// assert_eq!(idl_type_to_rust("publicKey"), "Pubkey");
/// ```
#[must_use]
pub fn idl_type_to_rust(idl_type: &str) -> String {
    match idl_type {
        "u8" | "u16" | "u32" | "u64" | "u128" | "i8" | "i16" | "i32" | "i64" | "i128" => {
            idl_type.to_string()
        }
        "bool" => "bool".to_string(),
        "string" => "String".to_string(),
        "publicKey" => "Pubkey".to_string(),
        "bytes" => "Vec<u8>".to_string(),
        _ => format!("/* Unknown type: {idl_type} */"),
    }
}

/// Generates a Rust struct definition from an IDL event.
///
/// # Arguments
///
/// * `event` - IDL event definition
///
/// # Returns
///
/// Rust struct code as a string
///
/// # Example
///
/// ```
/// use solana_indexer::{IdlEvent, IdlField, IdlType, generate_event_struct};
///
/// let event = IdlEvent {
///     name: "TransferEvent".to_string(),
///     fields: vec![
///         IdlField {
///             name: "from".to_string(),
///             ty: IdlType::Simple("publicKey".to_string()),
///         },
///         IdlField {
///             name: "amount".to_string(),
///             ty: IdlType::Simple("u64".to_string()),
///         },
///     ],
/// };
///
/// let struct_code = generate_event_struct(&event);
/// assert!(struct_code.contains("pub struct TransferEvent"));
/// assert!(struct_code.contains("pub from: Pubkey"));
/// assert!(struct_code.contains("pub amount: u64"));
/// ```
#[must_use]
#[allow(clippy::format_push_string, clippy::useless_format)]
pub fn generate_event_struct(event: &IdlEvent) -> String {
    let mut code = String::new();

    code.push_str(&format!(
        "#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]\n"
    ));
    code.push_str(&format!("pub struct {} {{\n", event.name));

    for field in &event.fields {
        let rust_type = match &field.ty {
            IdlType::Simple(s) => idl_type_to_rust(s),
            IdlType::Complex(_) => "/* Complex type */".to_string(),
        };
        code.push_str(&format!("    pub {}: {},\n", field.name, rust_type));
    }

    code.push_str("}\n");
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_idl_parse() {
        let idl_json = r#"{
            "version": "0.1.0",
            "name": "test_program",
            "instructions": [],
            "accounts": [],
            "events": []
        }"#;

        let idl = Idl::parse(idl_json).unwrap();
        assert_eq!(idl.version, "0.1.0");
        assert_eq!(idl.name, "test_program");
    }

    #[test]
    fn test_idl_with_events() {
        let idl_json = r#"{
            "version": "0.1.0",
            "name": "test_program",
            "instructions": [],
            "accounts": [],
            "events": [
                {
                    "name": "TransferEvent",
                    "fields": [
                        {"name": "from", "type": "publicKey"},
                        {"name": "to", "type": "publicKey"},
                        {"name": "amount", "type": "u64"}
                    ]
                }
            ]
        }"#;

        let idl = Idl::parse(idl_json).unwrap();
        assert_eq!(idl.events.len(), 1);
        assert_eq!(idl.events[0].name, "TransferEvent");
        assert_eq!(idl.events[0].fields.len(), 3);
    }

    #[test]
    fn test_event_names() {
        let idl_json = r#"{
            "version": "0.1.0",
            "name": "test_program",
            "instructions": [],
            "accounts": [],
            "events": [
                {"name": "Event1", "fields": []},
                {"name": "Event2", "fields": []}
            ]
        }"#;

        let idl = Idl::parse(idl_json).unwrap();
        let names = idl.event_names();
        assert_eq!(names, vec!["Event1", "Event2"]);
    }

    #[test]
    fn test_instruction_names() {
        let idl_json = r#"{
            "version": "0.1.0",
            "name": "test_program",
            "instructions": [
                {"name": "transfer", "args": [], "accounts": []},
                {"name": "deposit", "args": [], "accounts": []}
            ],
            "accounts": [],
            "events": []
        }"#;

        let idl = Idl::parse(idl_json).unwrap();
        let names = idl.instruction_names();
        assert_eq!(names, vec!["transfer", "deposit"]);
    }

    #[test]
    fn test_idl_type_to_rust() {
        assert_eq!(idl_type_to_rust("u64"), "u64");
        assert_eq!(idl_type_to_rust("string"), "String");
        assert_eq!(idl_type_to_rust("publicKey"), "Pubkey");
        assert_eq!(idl_type_to_rust("bool"), "bool");
        assert_eq!(idl_type_to_rust("bytes"), "Vec<u8>");
    }

    #[test]
    fn test_generate_event_struct() {
        let event = IdlEvent {
            name: "TestEvent".to_string(),
            fields: vec![
                IdlField {
                    name: "value".to_string(),
                    ty: IdlType::Simple("u64".to_string()),
                },
                IdlField {
                    name: "user".to_string(),
                    ty: IdlType::Simple("publicKey".to_string()),
                },
            ],
        };

        let code = generate_event_struct(&event);
        assert!(code.contains("pub struct TestEvent"));
        assert!(code.contains("pub value: u64"));
        assert!(code.contains("pub user: Pubkey"));
    }
}
