//! IDL parsing and type generation utilities.
//!
//! This module provides functionality for generating Rust types from Solana
//! program IDL (Interface Definition Language) files.
//!
//! # Usage
//!
//! The recommended way to use IDL parsing is via a `build.rs` script:
//!
//! ```ignore
//! // build.rs
//! use std::env;
//! use std::path::PathBuf;
//!
//! fn main() {
//!     let idl_path = PathBuf::from("idl/my_program.json");
//!     let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
//!     let generated_path = out_dir.join("generated_types.rs");
//!
//!     solana_idl_parser::generate_sdk_types(&idl_path, &generated_path)
//!         .expect("Failed to generate types from IDL");
//!
//!     println!("cargo:rerun-if-changed={}", idl_path.display());
//! }
//! ```
//!
//! Then include the generated types in your code:
//!
//! ```ignore
//! // src/lib.rs or src/main.rs
//! include!(concat!(env!("OUT_DIR"), "/generated_types.rs"));
//! ```

//! Note: The IDL parser functions are available as build-dependencies.
//! Use `solana_idl_parser::generate_sdk_types` directly in your `build.rs` script.
