//! The `make:*` generators.
//!
//! Every generator builds one [`rok_edit::Transaction`]: new files rendered
//! from the embedded atom-engine templates, plus structural edits to the
//! shared wiring (workspace manifest, router, `ApiDoc`, error crate,
//! composition root) located by parsing. Nothing is written until every
//! file is rendered, every anchor is found and every Rust file formats.

mod action;
mod alias;
mod batch;
mod dto;
mod entity;
mod error;
mod feature;
mod integrate;
mod journal;
mod manifest;
mod migration;
mod names;
mod presets;
mod repository;
mod service;
mod templates;

use std::fmt::Write as _;
use std::path::Path;

use rok_edit::{EditError, Transaction};
use rok_locate::{Edit, LocateError, Source};
use rok_plan::outcome::Outcome;
use rok_plan::project::Project;
use rok_render::RenderError;
use serde_json::json;

pub use batch::apply;
pub use integrate::{Permit, inspect, integrate};
pub use journal::{list as journal_list, undo, unlink};

#[derive(Debug, thiserror::Error)]
pub enum GenError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{file}: {source}")]
    Locate {
        file: String,
        #[source]
        source: LocateError,
    },
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error(transparent)]
    Render(#[from] RenderError),
}

/// How to finish a transaction.
#[derive(Clone, Copy, Debug)]
pub struct Apply {
    pub dry_run: bool,
    pub verify: bool,
}

/// What a generator produced, for output.
pub struct Generated {
    pub outcome: Outcome,
    pub human: String,
}

/// Formats, then shows (`dry_run`) or writes (optionally verified) `tx`.
fn finish(command: &str, tx: Transaction, apply: Apply) -> Result<Generated, GenError> {
    finish_labeled(command, None, tx, apply)
}

/// [`finish`], recording the committed transaction in the journal under
/// `label` (what `unlink` matches).
fn finish_labeled(
    command: &str,
    label: Option<String>,
    mut tx: Transaction,
    apply: Apply,
) -> Result<Generated, GenError> {
    tx.format()?;
    let mut outcome = Outcome::new(command, true);
    outcome.files_created = tx.created();
    outcome.files_changed = tx.changed();
    outcome.warnings = tx.warnings().to_vec();

    let mut human = String::new();
    if apply.dry_run {
        let diff = tx.diff();
        outcome.data = json!({ "dry_run": true, "diff": diff });
        human.push_str(&diff);
        human.push_str("dry run: nothing was written\n");
    } else {
        if apply.verify {
            tx.commit_verified()?;
        } else {
            tx.commit()?;
        }
        let transaction = journal::record(&tx, command, label);
        if let Err(warning) = &transaction {
            outcome.warnings.push(warning.clone());
        }
        let id = transaction.ok();
        outcome.data = json!({ "dry_run": false, "verified": apply.verify, "transaction": id });
        for file in &outcome.files_created {
            let _ = writeln!(human, "created  {file}");
        }
        for file in &outcome.files_changed {
            let _ = writeln!(human, "changed  {file}");
        }
        if let Some(id) = id {
            let _ = writeln!(human, "transaction {id} (undo with `cargo rok undo {id}`)");
        }
    }
    for warning in &outcome.warnings {
        let _ = writeln!(human, "warning: {warning}");
    }
    Ok(Generated { outcome, human })
}

/// Parses `rel`'s current content in `tx`.
fn parse(tx: &mut Transaction, rel: &str) -> Result<Source, GenError> {
    let text = tx.read(rel)?;
    Source::parse(rel, &text).map_err(|source| located(rel, source))
}

fn located(file: &str, source: LocateError) -> GenError {
    GenError::Locate {
        file: file.to_string(),
        source,
    }
}

/// Applies `edits` to `rel` in `tx`.
fn edit(
    tx: &mut Transaction,
    rel: &str,
    source: &Source,
    edits: Vec<Edit>,
) -> Result<(), GenError> {
    if edits.is_empty() {
        return Ok(());
    }
    tx.update(rel, rok_locate::apply(source.text(), edits))?;
    Ok(())
}

/// Appends `items` to `#[module(key = [...])]` in `rel`, adding the key if
/// the attribute doesn't have it yet.
fn append_module_list(
    tx: &mut Transaction,
    rel: &str,
    key: &str,
    items: &[String],
) -> Result<(), GenError> {
    let source = parse(tx, rel)?;
    let edit_ = if let Some(list) = source.module_list(key).map_err(|err| located(rel, err))? {
        list.append(items)
    } else {
        let end = source.module_args_end().map_err(|err| located(rel, err))?;
        let before = source.text()[..end].trim_end();
        let separator = if before.ends_with(',') || before.ends_with('(') {
            " "
        } else {
            ", "
        };
        Some(Edit::new(
            before.len(),
            format!("{separator}{key} = [{}]", items.join(", ")),
        ))
    };
    edit(tx, rel, &source, edit_.into_iter().collect())
}

/// Appends to `ApiDoc`'s `paths(...)` and `components(schemas(...))`, and
/// optionally `tags(...)`.
fn register_openapi(
    tx: &mut Transaction,
    rel: &str,
    paths: &[String],
    schemas: &[String],
    tags: &[String],
) -> Result<(), GenError> {
    let source = parse(tx, rel)?;
    let mut edits = Vec::new();
    for (path, items) in [
        (&["paths"][..], paths),
        (&["components", "schemas"][..], schemas),
        (&["tags"][..], tags),
    ] {
        if items.is_empty() {
            continue;
        }
        let list = source.openapi_list(path).map_err(|err| located(rel, err))?;
        edits.extend(list.append(items));
    }
    edit(tx, rel, &source, edits)
}

/// Inserts `lines` after the last top-level item matching `pred` (or at
/// the top, after inner attributes/docs, when none does), skipping lines
/// already present verbatim.
fn insert_after_items(
    tx: &mut Transaction,
    rel: &str,
    pred: impl Fn(&syn::Item) -> bool,
    lines: &[String],
) -> Result<(), GenError> {
    let source = parse(tx, rel)?;
    let missing: Vec<&String> = lines
        .iter()
        .filter(|line| {
            !source
                .text()
                .lines()
                .any(|existing| existing.trim() == line.trim())
        })
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let block = missing.iter().fold(String::new(), |mut block, line| {
        block.push('\n');
        block.push_str(line);
        block
    });
    let edit_ = source.last_item_end(pred).map_or_else(
        || {
            Edit::new(
                after_inner_docs(source.text()),
                format!("{}\n", block.trim_start()),
            )
        },
        |offset| Edit::new(offset, block.clone()),
    );
    edit(tx, rel, &source, vec![edit_])
}

/// Offset just past a file's leading `//!` docs and `#![...]` attributes.
fn after_inner_docs(text: &str) -> usize {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("//!") || trimmed.starts_with("#![") || trimmed.is_empty() {
            offset += line.len();
        } else {
            break;
        }
    }
    offset
}

/// The manifest of the crate containing `rel`.
fn crate_manifest_of(tx: &Transaction, rel: &str) -> Option<String> {
    let mut dir = Path::new(rel).parent();
    while let Some(current) = dir {
        let manifest = current
            .join("Cargo.toml")
            .to_string_lossy()
            .replace('\\', "/");
        if tx.exists(&manifest) {
            return Some(manifest);
        }
        dir = current.parent();
    }
    tx.exists("Cargo.toml").then(|| "Cargo.toml".to_string())
}

/// The composition root's `lib.rs` (or `main.rs`).
fn composition_root_file(project: &Project) -> Result<String, GenError> {
    let dir = project.config.project.composition_root.trim_matches('/');
    ["lib.rs", "main.rs"]
        .iter()
        .map(|name| format!("{dir}/{name}"))
        .find(|rel| project.root.join(rel).is_file())
        .ok_or_else(|| {
            GenError::Invalid(format!(
                "no lib.rs or main.rs in the composition root `{dir}` (see [project] in rok.toml)"
            ))
        })
}

fn require_file(tx: &Transaction, rel: &str, what: &str) -> Result<(), GenError> {
    if tx.exists(rel) {
        Ok(())
    } else {
        Err(GenError::Invalid(format!(
            "{what} `{rel}` doesn't exist; set it under [project] in rok.toml"
        )))
    }
}

/// Adds `use <path>;` for each path not already imported verbatim, keeping
/// the file's grouping: `crate::`/`super::`/`self::` paths after the last
/// such `use`, everything else after the last other `use` (rustfmt sorts
/// within a group but never moves imports between groups).
fn add_imports(tx: &mut Transaction, rel: &str, paths: &[String]) -> Result<(), GenError> {
    let local = |path: &str| {
        ["crate::", "super::", "self::"]
            .iter()
            .any(|prefix| path.starts_with(prefix))
    };
    let is_local_use = |item: &syn::Item| match item {
        syn::Item::Use(item) => match &item.tree {
            syn::UseTree::Path(path) => {
                path.ident == "crate" || path.ident == "super" || path.ident == "self"
            }
            _ => false,
        },
        _ => false,
    };
    let (local_paths, external_paths): (Vec<&String>, Vec<&String>) =
        paths.iter().partition(|path| local(path));
    for (group, wants_local) in [(external_paths, false), (local_paths, true)] {
        if group.is_empty() {
            continue;
        }
        let lines: Vec<String> = group.iter().map(|path| format!("use {path};")).collect();
        let source = parse(tx, rel)?;
        let has_group = source
            .file
            .items
            .iter()
            .any(|item| matches!(item, syn::Item::Use(_)) && is_local_use(item) == wants_local);
        if has_group {
            insert_after_items(
                tx,
                rel,
                |item| matches!(item, syn::Item::Use(_)) && is_local_use(item) == wants_local,
                &lines,
            )?;
        } else if wants_local {
            // A new local group goes after every import, separated.
            let mut separated = lines;
            separated[0] = format!("\n{}", separated[0]);
            insert_after_items(
                tx,
                rel,
                |item| matches!(item, syn::Item::Use(_)),
                &separated,
            )?;
        } else {
            // A new external group goes before the local imports.
            let mut separated = lines;
            if let Some(last) = separated.last_mut() {
                last.push('\n');
            }
            insert_before_first_use(tx, rel, &separated)?;
        }
    }
    Ok(())
}

/// Inserts `lines` before the first top-level `use` (or after the file's
/// inner docs when there is none).
fn insert_before_first_use(
    tx: &mut Transaction,
    rel: &str,
    lines: &[String],
) -> Result<(), GenError> {
    let source = parse(tx, rel)?;
    let offset = source
        .file
        .items
        .iter()
        .find(|item| matches!(item, syn::Item::Use(_)))
        .map_or_else(
            || after_inner_docs(source.text()),
            |item| source.start(syn::spanned::Spanned::span(item)),
        );
    let block = lines.iter().fold(String::new(), |mut block, line| {
        block.push_str(line);
        block.push('\n');
        block
    });
    edit(tx, rel, &source, vec![Edit::new(offset, block)])
}

/// The render context for an entity's templates (entity, DTOs,
/// repository, service, actions, migration).
fn entity_context(
    names: &names::FeatureNames,
    fields: &[names::Field],
    guard: rok_plan::make::Guard,
    generator: &str,
) -> serde_json::Value {
    use rok_plan::make::Guard;

    let columns: Vec<&str> = std::iter::once("id")
        .chain(fields.iter().map(|field| field.name.as_str()))
        .chain(std::iter::once("created_at"))
        .collect();
    let insert_columns = &columns[..columns.len() - 1];
    let insert_params: Vec<String> = (1..=insert_columns.len())
        .map(|n| format!("${n}"))
        .collect();
    let update_sets: Vec<String> = fields
        .iter()
        .enumerate()
        .map(|(index, field)| format!("{0} = COALESCE(${1}, {0})", field.name, index + 2))
        .collect();
    let any = |base: &str| fields.iter().any(|field| field.base == base);
    let mut context = serde_json::to_value(names).unwrap_or_else(|_| json!({}));
    if let Some(object) = context.as_object_mut() {
        for (key, value) in [
            ("generator", json!(generator)),
            ("fields", json!(fields)),
            ("columns", json!(columns.join(", "))),
            ("insert_columns", json!(insert_columns.join(", "))),
            ("insert_params", json!(insert_params.join(", "))),
            ("update_sets", json!(update_sets.join(", "))),
            ("guarded", json!(guard != Guard::None)),
            ("admin", json!(guard == Guard::Admin)),
            ("has_date", json!(any("NaiveDate"))),
            ("has_datetime_field", json!(any("DateTime<Utc>"))),
            ("has_uuid_field", json!(any("Uuid"))),
            ("has_json", json!(any("serde_json::Value"))),
        ] {
            object.insert(key.to_string(), value);
        }
    }
    context
}

/// Declares `mod <module>;` in a crate root and re-exports `items` from it.
fn export(tx: &mut Transaction, lib: &str, module: &str, items: &str) -> Result<(), GenError> {
    insert_after_items(
        tx,
        lib,
        |item| matches!(item, syn::Item::Mod(_)),
        &[format!("mod {module};")],
    )?;
    insert_after_items(
        tx,
        lib,
        |item| matches!(item, syn::Item::Use(item) if matches!(item.vis, syn::Visibility::Public(_))),
        &[format!("pub use {module}::{items};")],
    )
}

/// Adds `<dep>.workspace = true` for each of `deps` missing from a crate's
/// manifest.
fn add_crate_deps(tx: &mut Transaction, manifest_rel: &str, deps: &[&str]) -> Result<(), GenError> {
    let internal = manifest::internal_crates(&tx.read("Cargo.toml")?);
    let mut text = tx.read(manifest_rel)?;
    for dep in deps {
        text = manifest::add_dependency(
            &text,
            "dependencies",
            dep,
            &format!("{dep}.workspace = true"),
            &internal,
        );
    }
    tx.update(manifest_rel, text)?;
    Ok(())
}

/// Requires `crates/<feature>/src/lib.rs`, returning the crate's `src` dir.
fn feature_src(tx: &Transaction, feature: &str) -> Result<String, GenError> {
    let src = format!("crates/{feature}/src");
    if tx.exists(&format!("{src}/lib.rs")) {
        Ok(src)
    } else {
        Err(GenError::Invalid(format!(
            "no feature crate `{feature}` (expected {src}/lib.rs)"
        )))
    }
}

/// Declares the crate's `fake` feature (which exposes its in-memory
/// repository fakes to other crates' tests) if it isn't already.
fn ensure_fake_feature(tx: &mut Transaction, manifest_rel: &str) -> Result<(), GenError> {
    let text = tx.read(manifest_rel)?;
    let updated = manifest::add_dependency(
        &text,
        "features",
        "fake",
        "fake = []",
        &std::collections::BTreeSet::new(),
    );
    tx.update(manifest_rel, updated)?;
    Ok(())
}

/// One generator's work, applied to a transaction (what a plan's `make`
/// step runs), and its command form.
type Into<A> = fn(&mut Transaction, &Project, &A) -> Result<(), GenError>;

fn run_make<A>(
    command: &str,
    project: &Project,
    args: &A,
    apply: Apply,
    into: Into<A>,
) -> Result<Generated, GenError> {
    let mut tx = Transaction::new(&project.root);
    into(&mut tx, project, args)?;
    finish(command, tx, apply)
}

macro_rules! generator {
    ($name:ident, $command:literal, $args:ty, $into:path) => {
        pub fn $name(project: &Project, args: &$args) -> Result<Generated, GenError> {
            run_make(
                $command,
                project,
                args,
                Apply {
                    dry_run: args.dry_run,
                    verify: args.verify,
                },
                $into,
            )
        }
    };
}

generator!(
    make_feature,
    "make:feature",
    rok_plan::make::MakeFeatureArgs,
    feature::feature_into
);
generator!(
    make_action,
    "make:action",
    rok_plan::make::MakeActionArgs,
    action::action_into
);
generator!(
    make_dto,
    "make:dto",
    rok_plan::make::MakeDtoArgs,
    dto::dto_into
);
generator!(
    make_entity,
    "make:entity",
    rok_plan::make::MakeEntityArgs,
    entity::entity_into
);
generator!(
    make_error,
    "make:error",
    rok_plan::make::MakeErrorArgs,
    error::error_into
);
generator!(
    make_repository,
    "make:repository",
    rok_plan::make::MakeRepositoryArgs,
    repository::repository_into
);
generator!(
    make_service,
    "make:service",
    rok_plan::make::MakeServiceArgs,
    service::service_into
);

pub fn make_migration(
    project: &Project,
    args: &rok_plan::make::MakeMigrationArgs,
) -> Result<Generated, GenError> {
    run_make(
        "make:migration",
        project,
        args,
        Apply {
            dry_run: args.dry_run,
            verify: false,
        },
        migration::migration_into,
    )
}

/// Runs a generator inside an existing transaction, for a plan's `make`
/// step. The generator's own `dry_run`/`verify` are ignored: the plan's
/// apply.
///
/// # Errors
///
/// [`GenError::Invalid`] for an unknown command or arguments that don't fit
/// it, plus anything the generator returns.
pub fn make_into(
    tx: &mut Transaction,
    project: &Project,
    command: &str,
    args: &serde_json::Value,
) -> Result<(), GenError> {
    fn parse_args<A: serde::de::DeserializeOwned>(
        command: &str,
        args: &serde_json::Value,
    ) -> Result<A, GenError> {
        serde_json::from_value(args.clone())
            .map_err(|err| GenError::Invalid(format!("`{command}` arguments: {err}")))
    }
    match command {
        "make:feature" => feature::feature_into(tx, project, &parse_args(command, args)?),
        "make:action" => action::action_into(tx, project, &parse_args(command, args)?),
        "make:dto" => dto::dto_into(tx, project, &parse_args(command, args)?),
        "make:entity" => entity::entity_into(tx, project, &parse_args(command, args)?),
        "make:error" => error::error_into(tx, project, &parse_args(command, args)?),
        "make:migration" => migration::migration_into(tx, project, &parse_args(command, args)?),
        "make:repository" => repository::repository_into(tx, project, &parse_args(command, args)?),
        "make:service" => service::service_into(tx, project, &parse_args(command, args)?),
        other => Err(GenError::Invalid(format!(
            "`{other}` isn't a generator; use make:feature, make:action, make:dto, make:entity, \
             make:error, make:migration, make:repository or make:service"
        ))),
    }
}

/// A preset's steps (and its label), for the wizard.
///
/// # Errors
///
/// Anything [`presets::expand`] returns.
pub fn preset_steps(
    project: &Project,
    preset: &str,
    vars: serde_json::Value,
) -> Result<(Option<String>, Vec<rok_plan::integrate::Step>), GenError> {
    let expanded = presets::expand(
        &mut Transaction::new(&project.root),
        &rok_plan::integrate::IntegrateArgs {
            preset: Some(preset.to_string()),
            vars,
            ..rok_plan::integrate::IntegrateArgs::default()
        },
    )?;
    Ok((expanded.name, expanded.steps))
}

/// Resolves a file alias (`posts/service`, `@root`, a path) to a path that
/// exists, relative to the project root.
///
/// # Errors
///
/// [`GenError::Invalid`].
pub fn resolve_alias(project: &Project, alias: &str) -> Result<String, GenError> {
    alias::resolve_file(project, alias)
}
