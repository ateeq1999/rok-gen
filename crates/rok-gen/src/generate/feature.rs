//! `cargo rok make:feature`: a complete CRUD feature crate, wired in.

use rok_edit::Transaction;
use rok_locate::Edit;
use rok_plan::make::MakeFeatureArgs;
use rok_plan::project::Project;
use rok_render::SyntaxKind;

use super::names::{self, FeatureNames};
use super::{
    GenError, add_imports, append_module_list, composition_root_file, crate_manifest_of, edit,
    entity_context, error, located, manifest, migration, parse, register_openapi, require_file,
    templates,
};

pub fn feature_into(
    tx: &mut Transaction,
    project: &Project,
    args: &MakeFeatureArgs,
) -> Result<(), GenError> {
    let names = FeatureNames::new(&args.name)?;
    if args.fields.is_empty() {
        return Err(GenError::Invalid(
            "give the entity at least one field, e.g. `--field title:string`".to_string(),
        ));
    }
    let fields = names::fields(&args.fields, true)?;
    let section = &project.config.project;
    let crate_dir = format!("crates/{}", names.feature);
    let root_file = composition_root_file(project)?;
    require_file(tx, &section.router, "router file")?;
    require_file(tx, section.apidoc_file(), "ApiDoc file")?;
    require_file(tx, &section.errors, "error crate lib.rs")?;
    if project.root.join(&crate_dir).exists() || tx.exists(&format!("{crate_dir}/Cargo.toml")) {
        return Err(GenError::Conflict(format!(
            "`{crate_dir}` already exists; pick another feature name"
        )));
    }
    let root_manifest = tx.read("Cargo.toml")?;
    if !root_manifest.contains("\"crates/*\"") {
        return Err(GenError::Invalid(
            "workspace.members must include \"crates/*\" so the new crate is a member".to_string(),
        ));
    }

    let context = entity_context(&names, &fields, args.guard, "make:feature");
    let renderer = templates::renderer()?;

    let s = &names.s;
    let rust_files = [
        ("feature/lib.rs", "src/lib.rs".to_string()),
        ("feature/entity.rs", format!("src/{s}_entity.rs")),
        ("feature/dto/mod.rs", "src/dto/mod.rs".to_string()),
        ("feature/dto/create.rs", format!("src/dto/create_{s}.rs")),
        ("feature/dto/update.rs", format!("src/dto/update_{s}.rs")),
        ("feature/repository.rs", format!("src/{s}_repository.rs")),
        ("feature/service.rs", format!("src/{s}_service.rs")),
        ("feature/action.rs", format!("src/{s}_action.rs")),
        ("feature/module.rs", format!("src/{s}_module.rs")),
    ];
    for (template, path) in rust_files {
        let file_text = renderer.render_rust(template, &context, SyntaxKind::File)?;
        tx.create(&format!("{crate_dir}/{path}"), file_text)?;
    }
    tx.create(
        &format!("{crate_dir}/Cargo.toml"),
        renderer.render("feature/Cargo.toml", &context)?,
    )?;

    error::add_error(tx, project, &names, &renderer)?;

    migration::create_next(
        tx,
        &section.migrations,
        &format!("create_{}", names.table),
        &renderer.render("feature/migration.sql", &context)?,
    )?;

    wire_manifests(project, tx, &names, &root_manifest, &root_file)?;
    append_module_list(
        tx,
        &section.router,
        "imports",
        &[format!("{}::{}Module", names.feature, names.S)],
    )?;
    register_openapi(
        tx,
        section.apidoc_file(),
        &handler_paths(&names),
        &[
            format!("{}::{}", names.feature, names.S),
            format!("{}::Create{}Request", names.feature, names.S),
            format!("{}::Update{}Request", names.feature, names.S),
            format!("envelope::ApiResponse<{}::{}>", names.feature, names.S),
            format!("envelope::ApiResponse<Vec<{}::{}>>", names.feature, names.S),
        ],
        &[format!(
            "(name = \"{}\", description = \"{} CRUD\")",
            names.tag, names.P
        )],
    )?;
    wire_composition_root(project, tx, &names, &root_file)?;

    Ok(())
}

fn handler_paths(names: &FeatureNames) -> Vec<String> {
    let (feature, s) = (&names.feature, &names.s);
    vec![
        format!("{feature}::{s}_action::list_{feature}"),
        format!("{feature}::{s}_action::get_{s}"),
        format!("{feature}::{s}_action::create_{s}"),
        format!("{feature}::{s}_action::update_{s}"),
        format!("{feature}::{s}_action::delete_{s}"),
    ]
}

/// The workspace dependency, plus a dependency from every crate that now
/// names the feature: the composition root's, the router's and `ApiDoc`'s.
fn wire_manifests(
    project: &Project,
    tx: &mut Transaction,
    names: &FeatureNames,
    root_manifest: &str,
    root_file: &str,
) -> Result<(), GenError> {
    let feature = &names.feature;
    let internal = manifest::internal_crates(root_manifest);
    let updated = manifest::add_dependency(
        root_manifest,
        "workspace.dependencies",
        feature,
        &format!("{feature} = {{ path = \"crates/{feature}\" }}"),
        &internal,
    );
    tx.update("Cargo.toml", updated)?;

    let section = &project.config.project;
    let mut dependents: Vec<String> = [root_file, &section.router, section.apidoc_file()]
        .iter()
        .filter_map(|file| crate_manifest_of(tx, file))
        .collect();
    dependents.sort();
    dependents.dedup();
    for dependent in dependents {
        let text = tx.read(&dependent)?;
        let updated = manifest::add_dependency(
            &text,
            "dependencies",
            feature,
            &format!("{feature}.workspace = true"),
            &internal,
        );
        tx.update(&dependent, updated)?;
    }
    Ok(())
}

/// Constructs the adapter and service in the composition function, next to
/// the existing ones, and registers the service as app data.
fn wire_composition_root(
    project: &Project,
    tx: &mut Transaction,
    names: &FeatureNames,
    root_file: &str,
) -> Result<(), GenError> {
    let (feature, s, big) = (&names.feature, &names.s, &names.S);
    let function_name = &project.config.project.composition_fn;

    let source = parse(tx, root_file)?;
    let function = source
        .function(function_name)
        .map_err(|err| located(root_file, err))?;
    let body = source.snippet(&*function.block);
    if !body.contains("let pool") {
        return Err(GenError::Invalid(format!(
            "`{function_name}` in {root_file} has no `pool` to build the Postgres adapter from"
        )));
    }

    let statements = format!(
        "\n\n    let {s}_repository: Arc<dyn {big}Repository> =\n        Arc::new(Pg{big}Repository::new(pool.clone()));\n    let {s}_service = {big}Service::new({s}_repository);"
    );
    let after_services = source.last_stmt_end(function, |_, text| {
        text.trim_start().starts_with("let ") && text.contains("Service::new(")
    });
    let statements_edit = if let Some(offset) = after_services {
        Edit::new(offset, statements)
    } else {
        let offset = source
            .first_stmt_start_containing(function, "HttpServer::new")
            .ok_or_else(|| {
                GenError::Invalid(format!(
                    "couldn't find where `{function_name}` builds services (no `let x = \
                     XService::new(..)` and no `HttpServer::new`)"
                ))
            })?;
        Edit::new(offset, format!("{}\n\n    ", statements.trim()))
    };

    let app_data = source.method_calls(function, "app_data", "_service");
    let Some(&last_app_data) = app_data.last() else {
        return Err(GenError::Invalid(format!(
            "couldn't find where `{function_name}` registers services (`.app_data(web::Data::new(x_service.clone()))`)"
        )));
    };
    let app_data_edit = Edit::new(
        last_app_data,
        format!("\n            .app_data(web::Data::new({s}_service.clone()))"),
    );
    edit(tx, root_file, &source, vec![statements_edit, app_data_edit])?;

    let imported = source.imported_names();
    let mut uses = vec![format!(
        "{feature}::{{Pg{big}Repository, {big}Repository, {big}Service}}"
    )];
    if !imported.contains("Arc") {
        uses.push("std::sync::Arc".to_string());
    }
    add_imports(tx, root_file, &uses)
}
