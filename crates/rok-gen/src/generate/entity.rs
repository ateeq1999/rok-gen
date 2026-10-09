//! `cargo rok make:entity`: an entity (stored shape, insert shape, partial
//! update) in an existing feature.

use rok_edit::Transaction;
use rok_plan::make::{FieldSpec, Guard, MakeEntityArgs};
use rok_plan::project::Project;
use rok_render::{Renderer, SyntaxKind};

use super::names::{self, FeatureNames};
use super::{
    Apply, GenError, Generated, add_crate_deps, entity_context, export, feature_src, finish,
    templates,
};

pub fn make_entity(project: &Project, args: &MakeEntityArgs) -> Result<Generated, GenError> {
    let names = FeatureNames::for_entity(&args.feature, &args.name, None)?;
    let mut tx = Transaction::new(&project.root);
    add_entity(
        &mut tx,
        project,
        &names,
        &args.fields,
        &templates::renderer()?,
    )?;
    finish(
        "make:entity",
        tx,
        Apply {
            dry_run: args.dry_run,
            verify: args.verify,
        },
    )
}

/// Adds `<entity>_entity.rs` with `<Entity>`, `New<Entity>` and
/// `<Entity>Update`, exported from the crate root.
pub fn add_entity(
    tx: &mut Transaction,
    project: &Project,
    names: &FeatureNames,
    specs: &[FieldSpec],
    renderer: &Renderer,
) -> Result<(), GenError> {
    let src = feature_src(project, &names.feature)?;
    if specs.is_empty() {
        return Err(GenError::Invalid(
            "give the entity at least one field, e.g. `--field title:string`".to_string(),
        ));
    }
    let fields = names::fields(specs, true)?;
    let file = format!("{src}/{}_entity.rs", names.s);
    if tx.exists(&file) {
        return Err(GenError::Conflict(format!("{file} already exists")));
    }
    let context = entity_context(names, &fields, Guard::None, "make:entity");
    tx.create(
        &file,
        renderer.render_rust("feature/entity.rs", &context, SyntaxKind::File)?,
    )?;
    let big = &names.S;
    export(
        tx,
        &format!("{src}/lib.rs"),
        &format!("{}_entity", names.s),
        &format!("{{New{big}, {big}, {big}Update}}"),
    )?;
    let mut deps = vec!["chrono", "serde", "utoipa", "uuid"];
    if fields.iter().any(|field| field.base == "serde_json::Value") {
        deps.push("serde_json");
    }
    add_crate_deps(tx, &format!("crates/{}/Cargo.toml", names.feature), &deps)
}
