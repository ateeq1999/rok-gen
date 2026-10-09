//! `cargo rok schema <command>`.

use serde_json::Value;

use crate::check::CheckArgs;
use crate::hooks::HooksInstallArgs;
use crate::make::{
    MakeActionArgs, MakeDtoArgs, MakeEntityArgs, MakeErrorArgs, MakeFeatureArgs, MakeMigrationArgs,
    MakeRepositoryArgs, MakeServiceArgs,
};
use crate::project::ProjectConfig;

/// Every name `schema_for` accepts.
pub const SCHEMA_NAMES: [&str; 11] = [
    "check",
    "hooks-install",
    "make:action",
    "make:dto",
    "make:entity",
    "make:error",
    "make:feature",
    "make:migration",
    "make:repository",
    "make:service",
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
        "make:entity" => schemars::schema_for!(MakeEntityArgs),
        "make:error" => schemars::schema_for!(MakeErrorArgs),
        "make:feature" => schemars::schema_for!(MakeFeatureArgs),
        "make:migration" => schemars::schema_for!(MakeMigrationArgs),
        "make:repository" => schemars::schema_for!(MakeRepositoryArgs),
        "make:service" => schemars::schema_for!(MakeServiceArgs),
        "project" => schemars::schema_for!(ProjectConfig),
        _ => return None,
    };
    serde_json::to_value(schema).ok()
}
