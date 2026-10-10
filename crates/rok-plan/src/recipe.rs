//! Arguments for the recipe commands: `cargo rok add` and `cargo rok remove`.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Arguments for `cargo rok add`: install a recipe (and whatever it
/// requires that isn't installed yet).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AddArgs {
    /// The recipe, e.g. `mail`.
    pub recipe: String,
    /// The recipe's driver, e.g. `resend`. Default: the recipe's default
    /// driver.
    pub driver: Option<String>,
    /// Drivers for the recipes this one requires, when they get installed
    /// along with it, e.g. `{"mail": "smtp"}`.
    pub drivers: BTreeMap<String, String>,
    /// Also add the driver's Terraform module under `infra/terraform/modules/`.
    pub terraform: bool,
    /// Print the diff; write nothing.
    pub dry_run: bool,
    /// Run `cargo check` after writing; roll everything back if it fails.
    pub verify: bool,
}

/// Arguments for `cargo rok remove`: uninstall a recipe nothing depends on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RemoveArgs {
    /// The recipe, e.g. `mail`.
    pub recipe: String,
    /// Print the diff; write nothing.
    pub dry_run: bool,
    /// Run `cargo check` after writing; roll everything back if it fails.
    pub verify: bool,
}
