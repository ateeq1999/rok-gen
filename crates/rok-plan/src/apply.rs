//! `cargo rok apply`: a list of commands run as one batch.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Arguments for `cargo rok apply`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ApplyArgs {
    /// The label the batch is recorded under in the journal.
    pub name: Option<String>,
    /// Run in order. Every generator and integration step edits one shared
    /// transaction, so later steps see earlier ones' files. `check` steps
    /// run once the batch is written, and a failing gate rolls the whole
    /// batch back.
    pub steps: Vec<BatchStep>,
    pub dry_run: bool,
    /// `cargo check` the workspace after writing; roll back on failure.
    pub verify: bool,
}

/// One command of a batch: its name and its JSON payload (the same payload
/// `-j` takes for that command; see `cargo rok schema <command>`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchStep {
    /// `make:feature`, `make:action`, …, `integrate` or `check`.
    pub command: String,
    #[serde(default)]
    pub args: Value,
}

/// Commands a batch step may run.
pub const BATCH_COMMANDS: [&str; 10] = [
    "make:feature",
    "make:action",
    "make:dto",
    "make:entity",
    "make:error",
    "make:migration",
    "make:repository",
    "make:service",
    "integrate",
    "check",
];
