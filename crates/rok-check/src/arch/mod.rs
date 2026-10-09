//! The architecture checker (`cargo rok check arch`).

mod deps;
mod source;

use std::collections::BTreeSet;
use std::path::PathBuf;

use rok_plan::project::{Project, ProjectError};

use crate::finding::Finding;
use crate::workspace::Workspace;

/// Why the checker couldn't run at all (as opposed to finding violations).
#[derive(Debug, thiserror::Error)]
pub enum ArchError {
    #[error("could not read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid manifest {}: {message}", path.display())]
    Manifest { path: PathBuf, message: String },
    #[error(transparent)]
    Config(#[from] ProjectError),
    #[error("[architecture].features names `{name}`, which isn't a workspace member")]
    UnknownFeature { name: String },
}

/// Runs every architecture rule over `project`, returning findings sorted
/// by file and line.
///
/// # Errors
///
/// [`ArchError`] if the workspace or config can't be read.
pub fn check(project: &Project) -> Result<Vec<Finding>, ArchError> {
    let workspace = Workspace::load(&project.root)?;
    let allowed = project.config.architecture.allowed_edges()?;
    let features = feature_crates(&workspace, project)?;

    let mut findings = deps::check(&workspace, &features, &allowed);
    findings.extend(source::check(&workspace, &features, project));
    findings.sort();
    findings.dedup();
    Ok(findings)
}

/// `[architecture].features` when set, otherwise every member with a
/// `src/*_module.rs` file.
fn feature_crates(workspace: &Workspace, project: &Project) -> Result<BTreeSet<String>, ArchError> {
    if let Some(names) = &project.config.architecture.features {
        return names
            .iter()
            .map(|name| {
                workspace
                    .member(name)
                    .map(|member| member.name.clone())
                    .ok_or_else(|| ArchError::UnknownFeature { name: name.clone() })
            })
            .collect();
    }

    Ok(workspace
        .members
        .iter()
        .filter(|member| {
            std::fs::read_dir(member.dir.join("src")).is_ok_and(|entries| {
                entries.filter_map(Result::ok).any(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.ends_with("_module.rs"))
                })
            })
        })
        .map(|member| member.name.clone())
        .collect())
}
