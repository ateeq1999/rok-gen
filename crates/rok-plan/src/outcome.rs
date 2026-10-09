//! The `--output json` result every command returns.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::FORMAT_VERSION;

/// The stable, machine-readable result of one command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub version: u32,
    pub command: String,
    pub ok: bool,
    #[serde(default)]
    pub files_created: Vec<String>,
    #[serde(default)]
    pub files_changed: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    /// Command-specific detail (e.g. gate results and findings for `check`).
    #[serde(default)]
    pub data: Value,
}

impl Outcome {
    #[must_use]
    pub fn new(command: impl Into<String>, ok: bool) -> Self {
        Self {
            version: FORMAT_VERSION,
            command: command.into(),
            ok,
            files_created: Vec::new(),
            files_changed: Vec::new(),
            warnings: Vec::new(),
            data: Value::Null,
        }
    }
}
