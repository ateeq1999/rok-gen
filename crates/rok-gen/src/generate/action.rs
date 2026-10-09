//! `cargo rok make:action`: one documented, registered handler stub.

use std::collections::BTreeSet;

use rok_edit::Transaction;
use rok_plan::make::{DtoKind, Guard, MakeActionArgs};
use rok_plan::project::Project;
use rok_render::SyntaxKind;
use serde_json::json;

use super::dto::{add_dto, file_stem};
use super::names::{FeatureNames, snake_ident};
use super::{
    GenError, add_imports, append_module_list, located, manifest, parse, register_openapi,
    templates,
};

pub fn action_into(
    tx: &mut Transaction,
    project: &Project,
    args: &MakeActionArgs,
) -> Result<(), GenError> {
    let names = FeatureNames::new(&args.feature)?;
    snake_ident(&args.name, "action name")?;
    let path = args.path.trim();
    if !(path.is_empty() || path.starts_with('/')) {
        return Err(GenError::Invalid(format!(
            "path `{path}` must be empty or start with `/` (it's relative to the module prefix)"
        )));
    }
    let src = format!("crates/{}/src", names.feature);
    let action_file = format!("{src}/{}_action.rs", names.s);
    let module_file = format!("{src}/{}_module.rs", names.s);
    for file in [&action_file, &module_file] {
        if !project.root.join(file).is_file() {
            return Err(GenError::Invalid(format!(
                "{file} doesn't exist; is `{}` a feature crate?",
                names.feature
            )));
        }
    }

    let module = parse(tx, &module_file)?;
    let module_args = module
        .module_args()
        .map_err(|err| located(&module_file, err))?;
    let prefix = module_args
        .iter()
        .find(|(key, _)| key == "prefix")
        .map(|(_, value)| value.trim_matches('"').to_string())
        .unwrap_or_default();
    let guarded = module_args.iter().any(|(key, _)| key == "middleware");

    let actions = parse(tx, &action_file)?;
    if actions.has_item(&args.name) {
        return Err(GenError::Conflict(format!(
            "{action_file} already has an item named `{}`",
            args.name
        )));
    }

    if let Some(body) = &args.body {
        let dto_file = format!("{src}/dto/{}.rs", file_stem(body));
        if !tx.exists(&dto_file) {
            add_dto(tx, project, &names.feature, body, DtoKind::Request, &[])?;
        }
    }

    let params = path_params(path);
    let handler = render_handler(args, &names.tag, &prefix, path, guarded, &params)?;
    let mut text = tx.read(&action_file)?;
    text.push_str(&handler);
    tx.update(&action_file, text)?;

    add_requirements(
        tx,
        &action_file,
        &names.feature,
        &actions.imported_names(),
        args,
        &params,
    )?;

    append_module_list(
        tx,
        &module_file,
        "controllers",
        std::slice::from_ref(&args.name),
    )?;
    if !module.imported_names().contains(&args.name) {
        add_imports(
            tx,
            &module_file,
            &[format!("crate::{}_action::{}", names.s, args.name)],
        )?;
    }
    register_openapi(
        tx,
        project.config.project.apidoc_file(),
        &[format!(
            "{}::{}_action::{}",
            names.feature, names.s, args.name
        )],
        &[],
        &[],
    )?;

    Ok(())
}

fn render_handler(
    args: &MakeActionArgs,
    tag: &str,
    prefix: &str,
    path: &str,
    guarded: bool,
    params: &[(String, String)],
) -> Result<String, GenError> {
    let path_type = match params {
        [] => String::new(),
        [(_, ty)] => ty.clone(),
        many => format!(
            "({})",
            many.iter()
                .map(|(_, ty)| ty.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let admin = args.guard == Guard::Admin;
    let path_params: Vec<_> = params
        .iter()
        .map(|(name, ty)| json!({ "name": name, "ty": ty }))
        .collect();
    let context = json!({
        "name": args.name,
        "method": args.method.name(),
        "method_upper": args.method.name().to_ascii_uppercase(),
        "path": path,
        "full_path": format!("{prefix}{path}"),
        "tag": tag,
        "guarded": guarded || admin,
        "admin": admin,
        "body": args.body,
        "path_params": path_params,
        "path_type": path_type,
    });
    Ok(templates::renderer()?.render_rust("action/handler.rs", &context, SyntaxKind::Item)?)
}

/// The imports the new handler needs that `action_file` doesn't have yet,
/// and the crate dependencies behind them.
fn add_requirements(
    tx: &mut Transaction,
    action_file: &str,
    feature: &str,
    imported: &BTreeSet<String>,
    args: &MakeActionArgs,
    params: &[(String, String)],
) -> Result<(), GenError> {
    let admin = args.guard == Guard::Admin;
    let uuid = params.iter().any(|(_, ty)| ty == "Uuid");

    let mut uses = Vec::new();
    let mut want = |name: &str, path: String| {
        if !imported.contains(name) {
            uses.push(path);
        }
    };
    want("HttpResponse", "actix_web::HttpResponse".to_string());
    want(
        args.method.name(),
        format!("actix_web::{}", args.method.name()),
    );
    if !params.is_empty() {
        want("web", "actix_web::web".to_string());
    }
    if uuid {
        want("Uuid", "uuid::Uuid".to_string());
    }
    if let Some(body) = &args.body {
        want("ValidatedJson", "validation::ValidatedJson".to_string());
        want(body, format!("crate::dto::{body}"));
    }
    if admin {
        want("AdminUser", "session::AdminUser".to_string());
    }
    add_imports(tx, action_file, &uses)?;

    let mut deps: BTreeSet<&str> = BTreeSet::from(["actix-web"]);
    if args.body.is_some() {
        deps.insert("validation");
    }
    if admin {
        deps.insert("session");
    }
    if uuid {
        deps.insert("uuid");
    }
    let internal = manifest::internal_crates(&tx.read("Cargo.toml")?);
    let crate_manifest = format!("crates/{feature}/Cargo.toml");
    let mut text = tx.read(&crate_manifest)?;
    for dep in deps {
        text = manifest::add_dependency(
            &text,
            "dependencies",
            dep,
            &format!("{dep}.workspace = true"),
            &internal,
        );
    }
    tx.update(&crate_manifest, text)?;
    Ok(())
}

/// `{name}` segments of a route path, typed `Uuid` for `id`/`*_id` and
/// `String` otherwise.
fn path_params(path: &str) -> Vec<(String, String)> {
    path.split('/')
        .filter_map(|segment| segment.strip_prefix('{')?.strip_suffix('}'))
        .map(|name| {
            let ty = if name == "id" || name.ends_with("_id") {
                "Uuid"
            } else {
                "String"
            };
            (name.to_string(), ty.to_string())
        })
        .collect()
}
