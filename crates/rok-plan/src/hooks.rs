//! `cargo rok hooks install` arguments.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::check::Gate;

/// Arguments for `cargo rok hooks install`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct HooksInstallArgs {
    /// Gates the pre-commit hook runs.
    pub gates: Vec<Gate>,
    /// Replace an existing pre-commit hook that `cargo rok` didn't install.
    pub overwrite: bool,
}

impl Default for HooksInstallArgs {
    fn default() -> Self {
        Self {
            gates: Gate::PRE_COMMIT.to_vec(),
            overwrite: false,
        }
    }
}
