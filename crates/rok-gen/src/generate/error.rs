//! `cargo rok make:error`: a feature's error type in the shared `error`
//! crate, with its HTTP mapping.

use rok_edit::Transaction;
use rok_plan::make::MakeErrorArgs;
use rok_plan::project::Project;
use rok_render::{Renderer, SyntaxKind};

use super::names::FeatureNames;
use super::{
    Apply, GenError, Generated, entity_context, finish, insert_after_items, require_file, templates,
};

pub fn make_error(project: &Project, args: &MakeErrorArgs) -> Result<Generated, GenError> {
    let names = FeatureNames::new(&args.feature)?;
    let mut tx = Transaction::new(&project.root);
    add_error(&mut tx, project, &names, &templates::renderer()?)?;
    finish(
        "make:error",
        tx,
        Apply {
            dry_run: args.dry_run,
            verify: args.verify,
        },
    )
}

/// Adds `<errors dir>/<feature>.rs` declaring `<Feature>Error`, plus its
/// `mod` and `pub use` in the error crate's root.
pub fn add_error(
    tx: &mut Transaction,
    project: &Project,
    names: &FeatureNames,
    renderer: &Renderer,
) -> Result<(), GenError> {
    let lib = &project.config.project.errors;
    require_file(project, lib, "error crate lib.rs")?;
    let dir = lib.rsplit_once('/').map_or("", |(dir, _)| dir);
    let file = format!("{dir}/{}.rs", names.feature);
    if tx.exists(&file) {
        return Err(GenError::Conflict(format!(
            "{file} already exists; `{}` already has an error type",
            names.feature
        )));
    }
    let context = entity_context(names, &[], rok_plan::make::Guard::None, "make:error");
    tx.create(
        &file,
        renderer.render_rust("feature/error.rs", &context, SyntaxKind::File)?,
    )?;
    insert_after_items(
        tx,
        lib,
        |item| matches!(item, syn::Item::Mod(module) if module.content.is_none()),
        &[format!("mod {};", names.feature)],
    )?;
    insert_after_items(
        tx,
        lib,
        |item| matches!(item, syn::Item::Use(item) if matches!(item.vis, syn::Visibility::Public(_))),
        &[format!("pub use {}::{};", names.feature, names.E)],
    )
}

/// Requires the feature's error type to be exported by the error crate.
pub fn require_error(
    tx: &mut Transaction,
    project: &Project,
    names: &FeatureNames,
) -> Result<(), GenError> {
    let lib = &project.config.project.errors;
    let text = tx.read(lib)?;
    let exported = format!("pub use {}::{};", names.feature, names.E);
    if text.lines().any(|line| line.trim() == exported) {
        Ok(())
    } else {
        Err(GenError::Invalid(format!(
            "the error crate doesn't export `{}`; create it with `cargo rok make:error {}`",
            names.E, names.feature
        )))
    }
}
