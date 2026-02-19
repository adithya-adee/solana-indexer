use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let idl_path = PathBuf::from("idl/my_program.json");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Path for OUT_DIR (standard)
    let generated_path = out_dir.join("generated_types.rs");

    // Path for exporting to idl directory (as requested)
    let export_path = PathBuf::from("idl/types.rs");

    if idl_path.exists() {
        println!(
            "cargo:info=Generating types from IDL: {}",
            idl_path.display()
        );

        // Generate for build process
        solana_idl_parser::generate_sdk_types(&idl_path, &generated_path)
            .expect("Failed to generate types from IDL");

        // Export to the idl directory for visibility
        solana_idl_parser::generate_sdk_types(&idl_path, &export_path)
            .expect("Failed to export types to idl directory");

        println!("cargo:rerun-if-changed={}", idl_path.display());
    } else {
        println!(
            "cargo:warning=IDL file not found at {}. Using placeholder types.",
            idl_path.display()
        );
        let placeholder = r#"
            use borsh::{BorshDeserialize, BorshSerialize};
            use solana_sdk::pubkey::Pubkey;
            use solana_indexer_sdk::EventDiscriminator;

            #[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
            pub struct UserInitialized {
                pub user: Pubkey,
                pub name: String,
            }

            impl UserInitialized {
                pub fn discriminator() -> [u8; 8] {
                    [0; 8]
                }
            }

            impl EventDiscriminator for UserInitialized {
                fn discriminator() -> [u8; 8] {
                    Self::discriminator()
                }
            }

            #[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
            pub struct InitializeArgs {
                pub name: String,
                pub age: u8,
            }

            #[derive(Clone, Debug)]
            pub struct InitializeAccounts {
                pub user: Pubkey,
                pub authority: Pubkey,
            }
        "#;
        fs::write(&generated_path, placeholder)
            .expect("Failed to write placeholder types to OUT_DIR");
        fs::write(&export_path, placeholder)
            .expect("Failed to write placeholder types to idl directory");
    }

    println!("cargo:rerun-if-changed=build.rs");
}
