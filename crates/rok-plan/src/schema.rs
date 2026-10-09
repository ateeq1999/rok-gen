//! `cargo rok schema <command>`.

use serde_json::Value;

use crate::check::CheckArgs;
use crate::hooks::HooksInstallArgs;
use crate::make::{MakeActionArgs, MakeDtoArgs, MakeFeatureArgs, MakeMigrationArgs};
use crate::project::ProjectConfig;

/// Every name `schema_for` accepts.
pub const SCHEMA_NAMES: [&str; 7] = [
    "check",
    "hooks-install",
    "make:action",
    "make:dto",
    "make:feature",
    "make:migration",
    "project",
];

/// The JSON Schema for a command's payload (or `project` for `rok.toml`).
#[must_use]
pub fn schema_for(name: &str) -> Option<Value> {
    let schema = match name {
        "check" => schemars::schema_for!(CheckArgs),
        "hooks-install" => schemars::schema_for!(HooksInstallArgs),
        "make:action" => schemars::schema_for!(MakeActionArgs),
        "make:dto" => schemars::schema_for!(MakeDtoArgs),
        "make:feature" => schemars::schema_for!(MakeFeatureArgs),
        "make:migration" => schemars::schema_for!(MakeMigrationArgs),
        "project" => schemars::schema_for!(ProjectConfig),
        _ => return None,
    };
    serde_json::to_value(schema).ok()
}
