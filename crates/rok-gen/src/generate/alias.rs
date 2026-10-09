//! File aliases: `posts/service` → `crates/posts/src/post_service.rs`.

use rok_plan::project::Project;
use rok_render::naming;

use super::{GenError, composition_root_file};

/// Resolves a file alias (or path) to an existing file, relative to the
/// project root.
///
/// - `@root`, `@router`, `@apidoc`, `@errors`: the configured shared files.
/// - `<feature>/<role>` with role `module`, `action`, `service`,
///   `repository`, `entity`: `crates/<feature>/src/<singular>_<role>.rs`.
/// - `<feature>/lib`, `<feature>/cargo`, `<feature>/dto/<name>`.
/// - `<feature>/<stem>`: `crates/<feature>/src/<stem>.rs` (e.g.
///   `posts/revision_repository`).
/// - Anything with an extension: a path relative to the root.
///
/// # Errors
///
/// [`GenError::Invalid`] when the alias doesn't resolve to an existing file.
pub fn resolve_file(project: &Project, alias: &str) -> Result<String, GenError> {
    let section = &project.config.project;
    let rel = match alias {
        "@root" => composition_root_file(project)?,
        "@router" => section.router.clone(),
        "@apidoc" => section.apidoc_file().to_string(),
        "@errors" => section.errors.clone(),
        _ if std::path::Path::new(alias).extension().is_some() => alias.to_string(),
        _ => {
            let (feature, rest) = alias.split_once('/').ok_or_else(|| {
                GenError::Invalid(format!(
                    "`{alias}` isn't a file alias; use `<feature>/<role>` (e.g. `posts/service`), \
                     `@root`/`@router`/`@apidoc`/`@errors`, or a path"
                ))
            })?;
            let src = format!("crates/{feature}/src");
            let singular = naming::singular(feature);
            match rest {
                "module" | "action" | "service" | "repository" | "entity" => {
                    format!("{src}/{singular}_{rest}.rs")
                }
                "lib" => format!("{src}/lib.rs"),
                "cargo" => format!("crates/{feature}/Cargo.toml"),
                _ => rest.strip_prefix("dto/").map_or_else(
                    || format!("{src}/{rest}.rs"),
                    |dto| format!("{src}/dto/{dto}.rs"),
                ),
            }
        }
    };
    if project.root.join(&rel).is_file() {
        Ok(rel)
    } else {
        Err(GenError::Invalid(format!(
            "`{alias}` resolves to {rel}, which doesn't exist"
        )))
    }
}
