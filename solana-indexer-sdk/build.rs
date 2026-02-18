//! Build script for solana-indexer-sdk
//!
//! This build script can automatically generate Rust types from IDL files
//! if the `IDL_PATH` environment variable is set.

use std::env;
use std::path::PathBuf;

fn main() {
    // Check if IDL_PATH is set
    if let Ok(idl_path_str) = env::var("IDL_PATH") {
        let idl_path = PathBuf::from(idl_path_str);
        let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
        let generated_path = out_dir.join("generated_types.rs");

        println!("cargo:warning=Generating types from IDL: {:?}", idl_path);

        solana_idl_parser::generate_sdk_types(&idl_path, &generated_path)
            .expect("Failed to generate types from IDL");

        println!("cargo:rerun-if-changed={}", idl_path.display());
        println!(
            "cargo:warning=Generated types written to: {:?}",
            generated_path
        );
    }
}
