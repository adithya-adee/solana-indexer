use solana_idl_parser::generate_from_idl;
use std::path::PathBuf;

#[test]
fn test_generate_from_idl() {
    let idl_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/solraiser.json");
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/generated");
    let generated_path = out_dir.join("generated_types.rs");

    std::fs::create_dir_all(&out_dir).unwrap();

    generate_from_idl(&idl_path, &generated_path).unwrap();

    let generated_code = std::fs::read_to_string(generated_path).unwrap();

    let expected_code = r#"use anchor_lang::prelude::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct Campaign {
    pub creator_pubkey: Pubkey,
    pub campaign_id: u64,
    pub goal_amount: u64,
    pub amount_raised: u64,
    pub deadline: i64,
    pub metadata_url: String,
    pub is_withdrawn: bool,
    pub withdrawn_amount: u64,
}

#[error_code]
pub enum SolraiserError {
    #[msg("Goal amount must be greater than 0")]
    InvalidGoalAmount,
    #[msg("Deadline must be in the future")]
    InvalidDeadline,
    #[msg("Metadata URL exceeds maximum length")]
    MetadataUrlTooLong,
    #[msg("Amount must be greater than 0")]
    InvalidAmount,
    #[msg("Unauthorized withdrawal - only campaign creator can withdraw")]
    UnauthorizedWithdraw,
    #[msg("Campaign has already reached its goal")]
    CampaignGoalReached,
    #[msg("Campaign deadline has passed")]
    CampaignExpired,
    #[msg("Campaign is still active, cannot withdraw yet")]
    CampaignStillActive,
    #[msg("Campaign goal has not been reached")]
    GoalNotReached,
    #[msg("Arithmetic overflow occurred")]
    ArithmeticOverflow,
    #[msg("Insufficient funds - withdrawal would violate rent exemption")]
    InsufficientFunds,
    #[msg("Campaign has already been withdrawn")]
    AlreadyWithdrawn,
}
"#;

    assert_eq!(generated_code.trim(), expected_code.trim());
}
