pub mod generator;
pub mod model;

// Re-export for convenience
pub use generator::{generate_types_with_mode, GenerationMode};

use anyhow::Result;
use std::path::Path;

/// Generate Anchor-compatible Rust types from an IDL file.
///
/// # Arguments
///
/// * `idl_path` - Path to the IDL JSON file
/// * `output_path` - Path where the generated Rust code will be written
///
/// # Errors
///
/// Returns an error if the IDL file cannot be read, parsed, or if code generation fails.
pub fn generate_from_idl(idl_path: &Path, output_path: &Path) -> Result<()> {
    let idl_content = std::fs::read_to_string(idl_path)?;
    let idl: model::Idl = serde_json::from_str(&idl_content)
        .map_err(|e| anyhow::anyhow!("Failed to parse IDL JSON: {}", e))?;

    let generated_code = generator::generate_types(&idl)?;

    std::fs::write(output_path, generated_code)
        .map_err(|e| anyhow::anyhow!("Failed to write generated code: {}", e))?;

    Ok(())
}

/// Generate SDK-compatible Rust types from an IDL file.
///
/// This function generates types that are compatible with `solana-indexer-sdk`:
/// - Uses `BorshSerialize` and `BorshDeserialize` instead of Anchor traits
/// - Implements `EventDiscriminator` trait for events
/// - Uses `solana_sdk::pubkey::Pubkey` for public keys
///
/// # Arguments
///
/// * `idl_path` - Path to the IDL JSON file
/// * `output_path` - Path where the generated Rust code will be written
///
/// # Errors
///
/// Returns an error if the IDL file cannot be read, parsed, or if code generation fails.
///
/// # Example
///
/// ```no_run
/// use std::path::PathBuf;
/// use solana_idl_parser::generate_sdk_types;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let idl_path = PathBuf::from("idl/my_program.json");
/// let output_path = PathBuf::from("src/generated_types.rs");
/// generate_sdk_types(&idl_path, &output_path)?;
/// # Ok(())
/// # }
/// ```
pub fn generate_sdk_types(idl_path: &Path, output_path: &Path) -> Result<()> {
    let idl_content = std::fs::read_to_string(idl_path)
        .map_err(|e| anyhow::anyhow!("Failed to read IDL file at {:?}: {}", idl_path, e))?;

    let idl: model::Idl = serde_json::from_str(&idl_content).map_err(|e| {
        anyhow::anyhow!("Failed to parse IDL JSON: {}. Content: {}", e, idl_content)
    })?;

    let generated_code = generator::generate_types_with_mode(&idl, generator::GenerationMode::Sdk)?;

    std::fs::write(output_path, generated_code).map_err(|e| {
        anyhow::anyhow!("Failed to write generated code to {:?}: {}", output_path, e)
    })?;

    Ok(())
}
