use solana_idl_parser::{
    generator::generate_types_with_mode, generator::GenerationMode, model::Idl,
};
use std::fs;
use std::path::PathBuf;

fn load_idl(file_name: &str) -> Idl {
    let idl_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(file_name);
    let idl_content = fs::read_to_string(idl_path).unwrap();
    serde_json::from_str(&idl_content).unwrap()
}

#[test]
fn test_sdk_generation_with_idl2() {
    let idl = load_idl("idl2.json");
    let generated_code = generate_types_with_mode(&idl, GenerationMode::Sdk).unwrap();

    let normalized_code = generated_code.replace(|c: char| c.is_whitespace(), "");

    // Test custom types
    assert!(normalized_code.contains("pubstructUserProfile"));
    assert!(normalized_code.contains("pubname:String"));
    assert!(normalized_code.contains("pubage:u8"));
    assert!(normalized_code.contains("pubcountry:Option<String>"));

    // Test events
    assert!(normalized_code.contains("pubstructUserInitialized"));
    assert!(normalized_code.contains("implEventDiscriminatorforUserInitialized"));

    // Test instruction args
    assert!(normalized_code.contains("pubstructInitializeArgs"));
    assert!(normalized_code.contains("pubname:String"));
    assert!(normalized_code.contains("pubage:u8"));

    assert!(normalized_code.contains("pubstructProcessDataArgs"));
    assert!(normalized_code.contains("pubdata:Vec<u8>"));
    assert!(normalized_code.contains("pubuser_profile:UserProfile"));
    assert!(normalized_code.contains("pubstructProcessArrayArgs{pubdata:[u8;32],}"));

    // Test instruction accounts
    assert!(normalized_code.contains("pubstructInitializeAccounts"));
    assert!(normalized_code.contains("pubuser:Pubkey"));
    assert!(normalized_code.contains("pubauthority:Pubkey"));

    assert!(normalized_code.contains("pubstructProcessDataAccounts"));
    assert!(normalized_code.contains("pubdata_account:Pubkey"));
    assert!(normalized_code.contains("pubnested_account:Pubkey"));

    // Test error enum
    assert!(normalized_code.contains("pubenumProgramError"));
    assert!(normalized_code.contains("InvalidUserData=6000"));
}

#[test]
fn test_anchor_generation_with_idl2() {
    let idl = load_idl("idl2.json");
    let generated_code = generate_types_with_mode(&idl, GenerationMode::Anchor).unwrap();

    let normalized_code = generated_code.replace(|c: char| c.is_whitespace(), "");

    // Test custom types
    assert!(normalized_code.contains("pubstructUserProfile"));
    assert!(normalized_code.contains("#[derive(AnchorSerialize,AnchorDeserialize,Clone,Debug)]"));

    // Test events
    assert!(normalized_code.contains("#[event]pubstructUserInitialized"));

    // Test error enum
    assert!(normalized_code.contains("pubenumComprehensiveTestProgramError"));
    assert!(normalized_code.contains("#[msg(\"Invaliduserdataprovided\")]InvalidUserData"));
}

#[test]
fn test_sdk_generation_with_idl() {
    let idl = load_idl("idl.json");
    let generated_code = generate_types_with_mode(&idl, GenerationMode::Sdk).unwrap();
    let normalized_code = generated_code.replace(|c: char| c.is_whitespace(), "");
    assert!(normalized_code.contains("pubstructMyEvent"));
}

#[test]
fn test_anchor_generation_with_idl() {
    let idl = load_idl("idl.json");
    let generated_code = generate_types_with_mode(&idl, GenerationMode::Anchor).unwrap();
    let normalized_code = generated_code.replace(|c: char| c.is_whitespace(), "");
    assert!(normalized_code.contains("#[event]pubstructMyEvent"));
}
