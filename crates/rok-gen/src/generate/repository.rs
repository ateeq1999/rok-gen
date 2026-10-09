//! `cargo rok make:repository`: a new entity with its repository port,
//! Postgres adapter, in-memory fake and table, in an existing feature.

use rok_edit::Transaction;
use rok_plan::make::{Guard, MakeRepositoryArgs};
use rok_plan::project::Project;
use rok_render::SyntaxKind;

use super::entity::add_entity;
use super::error::require_error;
use super::names::{self, FeatureNames};
use super::{
    Apply, GenError, Generated, add_crate_deps, entity_context, export, feature_src, finish,
    migration, templates,
};

pub fn make_repository(
    project: &Project,
    args: &MakeRepositoryArgs,
) -> Result<Generated, GenError> {
    let names = FeatureNames::for_entity(&args.feature, &args.name, args.table.as_deref())?;
    let src = feature_src(project, &names.feature)?;
    let file = format!("{src}/{}_repository.rs", names.s);
    let mut tx = Transaction::new(&project.root);
    if tx.exists(&file) {
        return Err(GenError::Conflict(format!("{file} already exists")));
    }
    require_error(&mut tx, project, &names)?;

    let renderer = templates::renderer()?;
    add_entity(&mut tx, project, &names, &args.fields, &renderer)?;
    let fields = names::fields(&args.fields, true)?;
    let context = entity_context(&names, &fields, Guard::None, "make:repository");
    tx.create(
        &file,
        renderer.render_rust("feature/repository.rs", &context, SyntaxKind::File)?,
    )?;
    let big = &names.S;
    export(
        &mut tx,
        &format!("{src}/lib.rs"),
        &format!("{}_repository", names.s),
        &format!("{{Pg{big}Repository, {big}Repository}}"),
    )?;
    add_crate_deps(
        &mut tx,
        &format!("crates/{}/Cargo.toml", names.feature),
        &[
            "actix-web",
            "async-trait",
            "db",
            "error",
            "pagination",
            "sqlx",
        ],
    )?;
    migration::create_next(
        &mut tx,
        &project.config.project.migrations,
        &format!("create_{}", names.table),
        &renderer.render("feature/migration.sql", &context)?,
    )?;
    finish(
        "make:repository",
        tx,
        Apply {
            dry_run: args.dry_run,
            verify: args.verify,
        },
    )
}
