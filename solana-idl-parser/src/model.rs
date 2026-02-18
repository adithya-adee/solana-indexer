use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Idl {
    pub address: String,
    pub metadata: IdlMetadata,
    pub instructions: Vec<IdlInstruction>,
    #[serde(default)]
    pub accounts: Vec<IdlAccount>,
    #[serde(default)]
    pub events: Vec<IdlEvent>,
    #[serde(default)]
    pub types: Vec<IdlType>,
    #[serde(default)]
    pub errors: Vec<IdlErrorCode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdlMetadata {
    pub name: String,
    pub version: String,
    pub spec: String,
    pub description: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdlInstruction {
    pub name: String,
    #[serde(default)]
    pub docs: Vec<String>,
    pub discriminator: Vec<u8>,
    pub accounts: Vec<IdlAccountItem>,
    pub args: Vec<IdlField>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(untagged)]
pub enum IdlAccountItem {
    IdlAccount(IdlInstructionAccount),
    IdlAccounts(IdlAccounts),
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IdlInstructionAccount {
    pub name: String,
    #[serde(default)]
    pub is_mut: bool,
    #[serde(default)]
    pub is_signer: bool,
    #[serde(default)]
    pub pda: Option<IdlPda>,
    #[serde(default)]
    pub address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdlAccounts {
    pub name: String,
    pub accounts: Vec<IdlAccountItem>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IdlPda {
    pub seeds: Vec<IdlSeed>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub enum IdlSeed {
    Const(IdlSeedConst),
    Account(IdlSeedAccount),
    Arg(IdlSeedArg),
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IdlSeedConst {
    pub kind: String,
    pub value: Vec<u8>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IdlSeedAccount {
    pub kind: String,
    pub path: String,
    #[serde(default)]
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IdlSeedArg {
    pub kind: String,
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdlField {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: IdlTypeDefinition,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdlEvent {
    pub name: String,
    pub fields: Vec<IdlField>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdlAccount {
    pub name: String,
    pub discriminator: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdlType {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: IdlTypeDefinitionInner,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
#[serde(untagged)]
pub enum IdlTypeDefinition {
    Simple(String),
    Complex(IdlTypeDefinitionComplex),
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub enum IdlTypeDefinitionComplex {
    Defined(String),
    Option(Box<IdlTypeDefinition>),
    Vec(Box<IdlTypeDefinition>),
    Array(Vec<Value>),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdlTypeDefinitionInner {
    pub kind: String,
    #[serde(default)]
    pub fields: Vec<IdlField>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdlErrorCode {
    pub code: u32,
    pub name: String,
    pub msg: String,
}
