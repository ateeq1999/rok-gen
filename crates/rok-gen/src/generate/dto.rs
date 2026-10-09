//! `cargo rok make:dto`: one DTO file, exported and documented.

use heck::ToSnakeCase;
use rok_edit::Transaction;
use rok_plan::make::{DtoKind, FieldSpec, MakeDtoArgs};
use rok_plan::project::Project;
use rok_render::SyntaxKind;
use serde_json::json;

use super::names::{self, pascal_ident, snake_ident};
use super::{GenError, insert_after_items, register_openapi, templates};

pub fn dto_into(
    tx: &mut Transaction,
    project: &Project,
    args: &MakeDtoArgs,
) -> Result<(), GenError> {
    add_dto(
        tx,
        project,
        &args.feature,
        &args.name,
        args.kind,
        &args.fields,
    )?;
    Ok(())
}

/// The DTO's file stem: `PublishPostRequest` → `publish_post`,
/// `PostSummaryResponse` → `post_summary_response` (the reference
/// project's naming).
pub fn file_stem(name: &str) -> String {
    name.strip_suffix("Request")
        .filter(|stem| !stem.is_empty())
        .unwrap_or(name)
        .to_snake_case()
}

/// Adds `crates/<feature>/src/dto/<stem>.rs`, its `mod`/`pub use` in
/// `dto/mod.rs`, a `pub use dto::Name` in the crate root, and the schema in
/// `ApiDoc`.
pub fn add_dto(
    tx: &mut Transaction,
    project: &Project,
    feature: &str,
    name: &str,
    kind: DtoKind,
    specs: &[FieldSpec],
) -> Result<(), GenError> {
    snake_ident(feature, "feature")?;
    pascal_ident(name, "DTO name")?;
    let fields = names::fields(specs, false)?;
    let src = format!("crates/{feature}/src");
    if !tx.exists(&format!("{src}/lib.rs")) {
        return Err(GenError::Invalid(format!(
            "no feature crate `{feature}` (expected crates/{feature}/src/lib.rs)"
        )));
    }
    let stem = file_stem(name);
    let file = format!("{src}/dto/{stem}.rs");
    if tx.exists(&file) {
        return Err(GenError::Conflict(format!("{file} already exists")));
    }

    let any = |base: &str| fields.iter().any(|field| field.base == base);
    let context = json!({
        "name": name,
        "request": kind == DtoKind::Request,
        "fields": fields,
        "has_date": any("NaiveDate"),
        "has_datetime_field": any("DateTime<Utc>"),
        "has_uuid_field": any("Uuid"),
    });
    let renderer = templates::renderer()?;
    tx.create(
        &file,
        renderer.render_rust("dto/dto.rs", &context, SyntaxKind::File)?,
    )?;

    let dto_mod = format!("{src}/dto/mod.rs");
    if !tx.exists(&dto_mod) {
        tx.create(&dto_mod, "")?;
        insert_after_items(
            tx,
            &format!("{src}/lib.rs"),
            |item| matches!(item, syn::Item::Mod(_)),
            &["mod dto;".to_string()],
        )?;
    }
    insert_after_items(
        tx,
        &dto_mod,
        |item| matches!(item, syn::Item::Mod(_)),
        &[format!("mod {stem};")],
    )?;
    insert_after_items(
        tx,
        &dto_mod,
        |item| matches!(item, syn::Item::Use(_)),
        &[format!("pub use {stem}::{name};")],
    )?;
    insert_after_items(
        tx,
        &format!("{src}/lib.rs"),
        |item| matches!(item, syn::Item::Use(item) if matches!(item.vis, syn::Visibility::Public(_))),
        &[format!("pub use dto::{name};")],
    )?;
    register_openapi(
        tx,
        project.config.project.apidoc_file(),
        &[],
        &[format!("{feature}::{name}")],
        &[],
    )
}
