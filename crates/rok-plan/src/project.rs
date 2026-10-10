//! `rok.toml`: what makes a directory a rok-managed project.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The project config's file name, at the workspace root.
pub const CONFIG_FILE: &str = "rok.toml";

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error(
        "not a rok-managed project: no {CONFIG_FILE} in {} or any parent directory",
        start.display()
    )]
    NotManaged { start: PathBuf },
    #[error("could not read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid {}: {message}", path.display())]
    Parse { path: PathBuf, message: String },
    #[error("invalid [rok].cli version requirement `{value}`: {message}")]
    BadVersionReq { value: String, message: String },
    #[error(
        "this project requires rok-gen {required}, but this is rok-gen {actual}; \
         install a matching version with `cargo install rok-gen --version '{required}'`"
    )]
    Incompatible { required: String, actual: String },
    #[error("invalid [architecture].allow entry `{entry}`: expected `<from> -> <to>`")]
    BadAllow { entry: String },
}

/// The parsed `rok.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub rok: RokSection,
    #[serde(default)]
    pub project: ProjectSection,
    #[serde(default)]
    pub architecture: ArchitectureSection,
    /// Installed recipes by name (`[recipes.<name>]`), maintained by
    /// `cargo rok add` and `cargo rok remove`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub recipes: BTreeMap<String, InstalledRecipe>,
}

/// One installed recipe.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstalledRecipe {
    /// The recipe version that was installed.
    pub version: String,
    /// The chosen driver, e.g. `resend`.
    pub driver: String,
    /// Whether the driver's Terraform module was added too.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub terraform: bool,
}

/// Version pins.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RokSection {
    /// Semver requirement on the rok-gen CLI, e.g. `^0.1`.
    pub cli: String,
    /// The template set version this project was generated with.
    pub templates: String,
}

/// Where the project's shared wiring lives.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectSection {
    /// Directory (relative to the root) of the composition root: the only
    /// place allowed to construct concrete adapters.
    pub composition_root: String,
    /// The function in the composition root's `lib.rs` that wires adapters
    /// and services.
    pub composition_fn: String,
    /// File holding `#[derive(OpenApi)]`. The architecture checker finds it
    /// automatically when unset; generators fall back to
    /// `crates/router/src/openapi.rs`.
    pub apidoc: Option<String>,
    /// File holding the app's root `#[module]` (`AppModule`).
    pub router: String,
    /// The `error` crate's `lib.rs`, where feature error types are exported.
    pub errors: String,
    /// Directory of the numbered SQL migrations.
    pub migrations: String,
}

impl Default for ProjectSection {
    fn default() -> Self {
        Self {
            composition_root: "src".to_string(),
            composition_fn: "run".to_string(),
            apidoc: None,
            router: "crates/router/src/lib.rs".to_string(),
            errors: "crates/error/src/lib.rs".to_string(),
            migrations: "crates/db/migrations".to_string(),
        }
    }
}

impl ProjectSection {
    /// The `ApiDoc` file generators edit.
    #[must_use]
    pub fn apidoc_file(&self) -> &str {
        self.apidoc
            .as_deref()
            .unwrap_or("crates/router/src/openapi.rs")
    }
}

/// Architecture rules configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ArchitectureSection {
    /// Sanctioned feature-to-feature dependencies, as `"<from> -> <to>"`.
    pub allow: Vec<String>,
    /// Feature crates by package name. Inferred when unset: every workspace
    /// crate with a `src/*_module.rs` file.
    pub features: Option<Vec<String>>,
}

impl ArchitectureSection {
    /// The allowlist as `(from, to)` pairs.
    ///
    /// # Errors
    ///
    /// [`ProjectError::BadAllow`] for an entry not shaped `from -> to`.
    pub fn allowed_edges(&self) -> Result<Vec<(String, String)>, ProjectError> {
        self.allow
            .iter()
            .map(|entry| {
                entry
                    .split_once("->")
                    .map(|(from, to)| (from.trim().to_string(), to.trim().to_string()))
                    .filter(|(from, to)| !from.is_empty() && !to.is_empty())
                    .ok_or_else(|| ProjectError::BadAllow {
                        entry: entry.clone(),
                    })
            })
            .collect()
    }
}

/// A rok-managed project: its root directory and parsed config.
#[derive(Clone, Debug)]
pub struct Project {
    pub root: PathBuf,
    pub config: ProjectConfig,
}

impl Project {
    /// Finds the nearest `rok.toml` at or above `start` and loads it.
    ///
    /// # Errors
    ///
    /// [`ProjectError::NotManaged`] if there is none, plus anything
    /// [`Project::load`] returns.
    pub fn discover(start: &Path) -> Result<Self, ProjectError> {
        start
            .ancestors()
            .find(|dir| dir.join(CONFIG_FILE).is_file())
            .map_or_else(
                || {
                    Err(ProjectError::NotManaged {
                        start: start.to_path_buf(),
                    })
                },
                Self::load,
            )
    }

    /// Loads `root/rok.toml`.
    ///
    /// # Errors
    ///
    /// [`ProjectError::Read`] or [`ProjectError::Parse`].
    pub fn load(root: &Path) -> Result<Self, ProjectError> {
        let path = root.join(CONFIG_FILE);
        let text = std::fs::read_to_string(&path).map_err(|source| ProjectError::Read {
            path: path.clone(),
            source,
        })?;
        let config = toml::from_str(&text).map_err(|err| ProjectError::Parse {
            path: path.clone(),
            message: err.to_string(),
        })?;
        Ok(Self {
            root: root.to_path_buf(),
            config,
        })
    }

    /// Refuses to operate on a project pinned to a different CLI version.
    ///
    /// # Errors
    ///
    /// [`ProjectError::BadVersionReq`] or [`ProjectError::Incompatible`].
    pub fn ensure_compatible(&self, cli_version: &str) -> Result<(), ProjectError> {
        let required = &self.config.rok.cli;
        let requirement =
            semver::VersionReq::parse(required).map_err(|err| ProjectError::BadVersionReq {
                value: required.clone(),
                message: err.to_string(),
            })?;
        let actual =
            semver::Version::parse(cli_version).map_err(|err| ProjectError::BadVersionReq {
                value: cli_version.to_string(),
                message: err.to_string(),
            })?;
        if requirement.matches(&actual) {
            Ok(())
        } else {
            Err(ProjectError::Incompatible {
                required: required.clone(),
                actual: cli_version.to_string(),
            })
        }
    }
}
