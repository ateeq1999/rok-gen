//! `cargo rok schema <command>`.

use serde_json::Value;

use crate::apply::ApplyArgs;
use crate::check::CheckArgs;
use crate::hooks::HooksInstallArgs;
use crate::integrate::{InspectArgs, IntegrateArgs};
use crate::make::{
    MakeActionArgs, MakeDtoArgs, MakeEntityArgs, MakeErrorArgs, MakeFeatureArgs, MakeMigrationArgs,
    MakeRepositoryArgs, MakeServiceArgs, NewArgs,
};
use crate::project::ProjectConfig;

/// Every name `schema_for` accepts.
pub const SCHEMA_NAMES: [&str; 15] = [
    "apply",
    "check",
    "hooks-install",
    "inspect",
    "integrate",
    "make:action",
    "make:dto",
    "make:entity",
    "make:error",
    "make:feature",
    "make:migration",
    "make:repository",
    "make:service",
    "new",
    "project",
];

/// The JSON Schema for a command's payload (or `project` for `rok.toml`).
#[must_use]
pub fn schema_for(name: &str) -> Option<Value> {
    let schema = match name {
        "apply" => schemars::schema_for!(ApplyArgs),
        "check" => schemars::schema_for!(CheckArgs),
        "hooks-install" => schemars::schema_for!(HooksInstallArgs),
        "inspect" => schemars::schema_for!(InspectArgs),
        "integrate" => schemars::schema_for!(IntegrateArgs),
        "make:action" => schemars::schema_for!(MakeActionArgs),
        "make:dto" => schemars::schema_for!(MakeDtoArgs),
        "make:entity" => schemars::schema_for!(MakeEntityArgs),
        "make:error" => schemars::schema_for!(MakeErrorArgs),
        "make:feature" => schemars::schema_for!(MakeFeatureArgs),
        "make:migration" => schemars::schema_for!(MakeMigrationArgs),
        "make:repository" => schemars::schema_for!(MakeRepositoryArgs),
        "make:service" => schemars::schema_for!(MakeServiceArgs),
        "new" => schemars::schema_for!(NewArgs),
        "project" => schemars::schema_for!(ProjectConfig),
        _ => return None,
    };
    serde_json::to_value(schema).ok()
}
