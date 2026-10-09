//! `cargo rok make:service`: a service depending on repository ports,
//! constructed and registered in the composition root.

use heck::ToUpperCamelCase;
use rok_edit::Transaction;
use rok_locate::Edit;
use rok_plan::make::MakeServiceArgs;
use rok_plan::project::Project;
use rok_render::SyntaxKind;
use serde_json::json;

use super::names::{FeatureNames, snake_ident};
use super::{
    Apply, GenError, Generated, add_imports, composition_root_file, edit, export, feature_src,
    finish, located, parse, templates,
};

pub fn make_service(project: &Project, args: &MakeServiceArgs) -> Result<Generated, GenError> {
    let names = FeatureNames::new(&args.feature)?;
    snake_ident(&args.name, "service name")?;
    let src = feature_src(project, &names.feature)?;
    let file = format!("{src}/{}_service.rs", args.name);
    let big = args.name.to_upper_camel_case();
    let mut tx = Transaction::new(&project.root);
    if tx.exists(&file) {
        return Err(GenError::Conflict(format!("{file} already exists")));
    }

    let mut repositories = Vec::new();
    for entity in &args.repositories {
        snake_ident(entity, "repository entity")?;
        let repo_file = format!("{src}/{entity}_repository.rs");
        let trait_name = format!("{}Repository", entity.to_upper_camel_case());
        let source = parse(&mut tx, &repo_file).map_err(|_| {
            GenError::Invalid(format!(
                "{repo_file} doesn't exist; create it with `cargo rok make:repository {} {entity}`",
                names.feature
            ))
        })?;
        if !source.has_item(&trait_name) {
            return Err(GenError::Invalid(format!(
                "{repo_file} has no `{trait_name}`"
            )));
        }
        repositories.push(json!({
            "s": entity,
            "S": entity.to_upper_camel_case(),
            "field": rok_render::naming::plural(entity),
        }));
    }

    let context = json!({ "S": big, "repositories": repositories });
    tx.create(
        &file,
        templates::renderer()?.render_rust("service/service.rs", &context, SyntaxKind::File)?,
    )?;
    export(
        &mut tx,
        &format!("{src}/lib.rs"),
        &format!("{}_service", args.name),
        &format!("{big}Service"),
    )?;
    wire(
        project,
        &mut tx,
        &names.feature,
        &args.name,
        &big,
        &args.repositories,
    )?;

    finish(
        "make:service",
        tx,
        Apply {
            dry_run: args.dry_run,
            verify: args.verify,
        },
    )
}

/// `let <name>_service = <Name>Service::new(Arc::new(Pg<R>Repository::new(pool.clone())), ...);`
/// after the last service the composition function builds, plus
/// `.app_data(...)` and the imports.
fn wire(
    project: &Project,
    tx: &mut Transaction,
    feature: &str,
    name: &str,
    big: &str,
    entities: &[String],
) -> Result<(), GenError> {
    let root_file = composition_root_file(project)?;
    let function_name = &project.config.project.composition_fn;
    let source = parse(tx, &root_file)?;
    let function = source
        .function(function_name)
        .map_err(|err| located(&root_file, err))?;
    let body = source.snippet(&*function.block);
    if body.contains(&format!("let {name}_service")) {
        return Err(GenError::Conflict(format!(
            "`{function_name}` already builds a `{name}_service`"
        )));
    }
    if !entities.is_empty() && !body.contains("let pool") {
        return Err(GenError::Invalid(format!(
            "`{function_name}` in {root_file} has no `pool` to build the Postgres adapters from"
        )));
    }

    let adapters: Vec<String> = entities
        .iter()
        .map(|entity| {
            format!(
                "Arc::new(Pg{}Repository::new(pool.clone()))",
                entity.to_upper_camel_case()
            )
        })
        .collect();
    let statement = format!(
        "\n\n    let {name}_service = {big}Service::new({});",
        adapters.join(", ")
    );
    let statement_edit = source
        .last_stmt_end(function, |_, text| {
            text.trim_start().starts_with("let ") && text.contains("Service::new(")
        })
        .or_else(|| source.first_stmt_start_containing(function, "HttpServer::new"))
        .map(|offset| Edit::new(offset, statement))
        .ok_or_else(|| {
            GenError::Invalid(format!(
                "couldn't find where `{function_name}` builds services"
            ))
        })?;
    let app_data = source.method_calls(function, "app_data", "_service");
    let Some(&last_app_data) = app_data.last() else {
        return Err(GenError::Invalid(format!(
            "couldn't find where `{function_name}` registers services (`.app_data(web::Data::new(x_service.clone()))`)"
        )));
    };
    let app_data_edit = Edit::new(
        last_app_data,
        format!("\n            .app_data(web::Data::new({name}_service.clone()))"),
    );
    edit(tx, &root_file, &source, vec![statement_edit, app_data_edit])?;

    let imported = source.imported_names();
    let mut items: Vec<String> = entities
        .iter()
        .map(|entity| format!("Pg{}Repository", entity.to_upper_camel_case()))
        .filter(|item| !imported.contains(item))
        .collect();
    items.push(format!("{big}Service"));
    let mut uses = vec![if items.len() == 1 {
        format!("{feature}::{}", items[0])
    } else {
        format!("{feature}::{{{}}}", items.join(", "))
    }];
    if !entities.is_empty() && !imported.contains("Arc") {
        uses.push("std::sync::Arc".to_string());
    }
    add_imports(tx, &root_file, &uses)
}
