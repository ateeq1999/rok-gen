use std::path::PathBuf;

use rok_plan::input::InputError;
use rok_plan::project::ProjectError;
use rok_plan::schema::SCHEMA_NAMES;

/// Why a command couldn't run. Always exits with [`crate::EXIT_USAGE`].
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error(transparent)]
    Input(#[from] InputError),
    #[error(transparent)]
    Project(#[from] ProjectError),
    #[error("no schema named `{name}`; available: {}", SCHEMA_NAMES.join(", "))]
    UnknownSchema { name: String },
    #[error("not a git repository ({message}); run `git init` first")]
    NotGit { message: String },
    #[error(
        "{} already exists and wasn't installed by `cargo rok`; pass --overwrite to replace it",
        path.display()
    )]
    HookExists { path: PathBuf },
    #[error("{0}")]
    Flag(String),
    #[error(transparent)]
    Generate(#[from] crate::generate::GenError),
    #[error("could not write {}: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
