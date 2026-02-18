use solana_idl_parser::{
    generate_sdk_types, generator::generate_types_with_mode, generator::GenerationMode, model::Idl,
};
use std::path::PathBuf;

#[test]
fn test_generate_sdk_types_from_idl() {
    // Create a valid IDL JSON for testing
    let idl_json = r#"{
        "version": "0.1.0",
        "name": "test_program",
        "address": "11111111111111111111111111111111",
        "metadata": {
            "name": "test",
            "version": "0.1.0",
            "spec": "0.1.0",
            "description": "test"
        },
        "instructions": [
            {
                "name": "initialize",
                "discriminator": [0, 0, 0, 0, 0, 0, 0, 0],
                "accounts": [],
                "args": []
            }
        ],
        "events": [
            {
                "name": "MyEvent",
                "fields": [
                    {
                        "name": "data",
                        "type": "u64"
                    }
                ]
            }
        ],
        "types": [],
        "errors": []
    }"#;

    let temp_dir = std::env::temp_dir();
    let idl_path = temp_dir.join("test_idl.json");
    std::fs::write(&idl_path, idl_json).unwrap();

    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/generated_sdk");
    let generated_path = out_dir.join("generated_types.rs");

    std::fs::create_dir_all(&out_dir).unwrap();

    generate_sdk_types(&idl_path, &generated_path).unwrap();

    let generated_code = std::fs::read_to_string(&generated_path).unwrap();

    // Verify SDK-specific imports (formatting may vary due to quote! macro)
    assert!(generated_code.contains("BorshDeserialize"));
    assert!(generated_code.contains("BorshSerialize"));
    assert!(generated_code.contains("solana_sdk"));
    assert!(generated_code.contains("Pubkey"));
    assert!(generated_code.contains("sha2") || generated_code.contains("Sha256"));

    // Verify Borsh traits are used (not Anchor)
    assert!(generated_code.contains("BorshSerialize"));
    assert!(generated_code.contains("BorshDeserialize"));
    assert!(!generated_code.contains("AnchorSerialize"));
    assert!(!generated_code.contains("AnchorDeserialize"));

    // Verify EventDiscriminator implementation
    assert!(generated_code.contains("EventDiscriminator"));
    assert!(generated_code.contains("impl") && generated_code.contains("EventDiscriminator"));

    // Verify event struct has discriminator method (formatting may vary)
    assert!(generated_code.contains("discriminator"));
    assert!(generated_code.contains("u8") || generated_code.contains("[u8"));
}

#[test]
fn test_sdk_mode_generates_correct_event_structure() {
    let idl_json = r#"{
        "version": "0.1.0",
        "name": "test_program",
        "address": "11111111111111111111111111111111",
        "metadata": {
            "name": "test",
            "version": "0.1.0",
            "spec": "0.1.0",
            "description": "test"
        },
        "instructions": [],
        "events": [
            {
                "name": "TransferEvent",
                "fields": [
                    {
                        "name": "from",
                        "type": "publicKey"
                    },
                    {
                        "name": "to",
                        "type": "publicKey"
                    },
                    {
                        "name": "amount",
                        "type": "u64"
                    }
                ]
            }
        ],
        "types": [],
        "errors": []
    }"#;

    let idl: Idl = serde_json::from_str(idl_json).unwrap();
    let generated_code = generate_types_with_mode(&idl, GenerationMode::Sdk).unwrap();

    // Verify event struct
    assert!(generated_code.contains("struct TransferEvent"));
    assert!(generated_code.contains("from") && generated_code.contains("Pubkey"));
    assert!(generated_code.contains("to") && generated_code.contains("Pubkey"));
    assert!(generated_code.contains("amount") && generated_code.contains("u64"));

    // Verify Borsh traits
    assert!(generated_code.contains("BorshSerialize"));
    assert!(generated_code.contains("BorshDeserialize"));

    // Verify EventDiscriminator implementation
    assert!(
        generated_code.contains("EventDiscriminator") && generated_code.contains("TransferEvent")
    );
    assert!(generated_code.contains("impl TransferEvent"));
    assert!(generated_code.contains("discriminator"));
    assert!(generated_code.contains("u8") || generated_code.contains("[u8"));
}

#[test]
fn test_sdk_mode_generates_custom_types() {
    let idl_json = r#"{
        "version": "0.1.0",
        "name": "test_program",
        "address": "11111111111111111111111111111111",
        "metadata": {
            "name": "test",
            "version": "0.1.0",
            "spec": "0.1.0",
            "description": "test"
        },
        "instructions": [],
        "events": [],
        "types": [
            {
                "name": "User",
                "type": {
                    "kind": "struct",
                    "fields": [
                        {
                            "name": "name",
                            "type": "string"
                        },
                        {
                            "name": "age",
                            "type": "u8"
                        }
                    ]
                }
            }
        ],
        "errors": []
    }"#;

    let idl: Idl = serde_json::from_str(idl_json).unwrap();
    let generated_code = generate_types_with_mode(&idl, GenerationMode::Sdk).unwrap();

    // Verify type struct
    assert!(generated_code.contains("struct User"));
    // Note: Field names are preserved as-is from IDL
    assert!(generated_code.contains("name"));
    assert!(generated_code.contains("age"));

    // Verify Borsh traits
    assert!(generated_code.contains("BorshSerialize"));
    assert!(generated_code.contains("BorshDeserialize"));
}

#[test]
fn test_sdk_mode_generates_error_enum() {
    let idl_json = r#"{
        "version": "0.1.0",
        "name": "test_program",
        "address": "11111111111111111111111111111111",
        "metadata": {
            "name": "test",
            "version": "0.1.0",
            "spec": "0.1.0",
            "description": "test"
        },
        "instructions": [],
        "events": [],
        "types": [],
        "errors": [
            {
                "code": 6000,
                "name": "InvalidInput",
                "msg": "Invalid input provided"
            },
            {
                "code": 6001,
                "name": "Unauthorized",
                "msg": "Unauthorized access"
            }
        ]
    }"#;

    let idl: Idl = serde_json::from_str(idl_json).unwrap();
    let generated_code = generate_types_with_mode(&idl, GenerationMode::Sdk).unwrap();

    // Verify error enum
    assert!(generated_code.contains("pub enum ProgramError"));
    assert!(generated_code.contains("InvalidInput = 6000"));
    assert!(generated_code.contains("Unauthorized = 6001"));
    assert!(generated_code.contains("// Invalid input provided"));
    assert!(generated_code.contains("// Unauthorized access"));
}

#[test]
fn test_sdk_mode_uses_correct_pubkey_type() {
    let idl_json = r#"{
        "version": "0.1.0",
        "name": "test_program",
        "address": "11111111111111111111111111111111",
        "metadata": {
            "name": "test",
            "version": "0.1.0",
            "spec": "0.1.0",
            "description": "test"
        },
        "instructions": [],
        "events": [
            {
                "name": "TestEvent",
                "fields": [
                    {
                        "name": "pubkey",
                        "type": "publicKey"
                    }
                ]
            }
        ],
        "types": [],
        "errors": []
    }"#;

    let idl: Idl = serde_json::from_str(idl_json).unwrap();
    let generated_code = generate_types_with_mode(&idl, GenerationMode::Sdk).unwrap();

    // Verify Pubkey type is used (not Anchor's Pubkey)
    assert!(generated_code.contains("pubkey"));
    assert!(generated_code.contains("Pubkey"));
    assert!(generated_code.contains("solana_sdk"));
}

#[test]
fn test_discriminator_calculation() {
    use sha2::{Digest, Sha256};

    let event_name = "MyEvent";
    let preimage = format!("event:{}", event_name);
    let hash = Sha256::digest(preimage.as_bytes());
    let mut discriminator = [0u8; 8];
    discriminator.copy_from_slice(&hash[..8]);

    // Verify discriminator is 8 bytes
    assert_eq!(discriminator.len(), 8);

    // Verify discriminator is deterministic
    let hash2 = Sha256::digest(preimage.as_bytes());
    let mut discriminator2 = [0u8; 8];
    discriminator2.copy_from_slice(&hash2[..8]);
    assert_eq!(discriminator, discriminator2);
}
