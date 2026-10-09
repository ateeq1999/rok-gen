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
    GenError, add_crate_deps, ensure_fake_feature, entity_context, export, feature_src,
    insert_after_items, migration, templates,
};

pub fn repository_into(
    tx: &mut Transaction,
    project: &Project,
    args: &MakeRepositoryArgs,
) -> Result<(), GenError> {
    let names = FeatureNames::for_entity(&args.feature, &args.name, args.table.as_deref())?;
    let src = feature_src(tx, &names.feature)?;
    let file = format!("{src}/{}_repository.rs", names.s);
    if tx.exists(&file) {
        return Err(GenError::Conflict(format!("{file} already exists")));
    }
    require_error(tx, project, &names)?;

    let renderer = templates::renderer()?;
    add_entity(tx, &names, &args.fields, &renderer)?;
    let fields = names::fields(&args.fields, true)?;
    let context = entity_context(&names, &fields, Guard::None, "make:repository");
    tx.create(
        &file,
        renderer.render_rust("feature/repository.rs", &context, SyntaxKind::File)?,
    )?;
    let big = &names.S;
    export(
        tx,
        &format!("{src}/lib.rs"),
        &format!("{}_repository", names.s),
        &format!("{{Pg{big}Repository, {big}Repository}}"),
    )?;
    insert_after_items(
        tx,
        &format!("{src}/lib.rs"),
        |item| matches!(item, syn::Item::Use(item) if matches!(item.vis, syn::Visibility::Public(_))),
        &[format!(
            "#[cfg(any(test, feature = \"fake\"))]\npub use {}_repository::fake::Fake{big}Repository;",
            names.s
        )],
    )?;
    let manifest = format!("crates/{}/Cargo.toml", names.feature);
    add_crate_deps(
        tx,
        &manifest,
        &[
            "actix-web",
            "async-trait",
            "db",
            "error",
            "pagination",
            "sqlx",
        ],
    )?;
    ensure_fake_feature(tx, &manifest)?;
    migration::create_next(
        tx,
        &project.config.project.migrations,
        &format!("create_{}", names.table),
        &renderer.render("feature/migration.sql", &context)?,
    )?;
    Ok(())
}
