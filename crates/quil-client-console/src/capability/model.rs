use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityGroup {
    Node,
    Prover,
    Wallet,
    Identity,
    Data,
    Diagnostics,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityFieldKind {
    Text,
    Integer,
    Boolean,
    Path,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityField {
    pub id: String,
    pub label: String,
    pub help: String,
    pub kind: CapabilityFieldKind,
    pub required: bool,
    pub repeated: bool,
    pub sensitive: bool,
    pub positional: Option<usize>,
    pub long: Option<String>,
    pub short: Option<char>,
    pub default_values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CapabilityTarget {
    Native { route: String },
    EmbeddedCli { command_path: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySpec {
    pub id: String,
    pub group: CapabilityGroup,
    pub title: String,
    pub description: String,
    pub read_only: bool,
    pub confirmation: Option<String>,
    pub fields: Vec<CapabilityField>,
    pub target: CapabilityTarget,
}
