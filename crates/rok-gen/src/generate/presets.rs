//! Built-in integration presets: atom templates that render to a plan.
//!
//! `has-many` (`source` has many per `target`, e.g. comments per post):
//! a `delete_by_<target>` method on the source repository (Postgres and
//! fake), the source port injected into the target service, a cascade
//! call at the start of the target's `delete`, and an index on the
//! foreign key.

use heck::ToUpperCamelCase;
use rok_plan::integrate::IntegrateArgs;
use rok_plan::outcome::Outcome;
use rok_plan::project::Project;
use rok_render::naming;
use serde_json::{Value, json};

use super::names::snake_ident;
use super::{GenError, Generated};

const PRESETS: &[(&str, &str)] = &[(
    "has-many",
    include_str!("../../templates/presets/has-many.plan.json.atom"),
)];

/// `args` with its preset (if any) rendered into `steps`.
///
/// # Errors
///
/// [`GenError::Invalid`] for an unknown preset, missing or invalid `vars`,
/// or a plan that already has steps.
pub fn expand(project: &Project, args: &IntegrateArgs) -> Result<IntegrateArgs, GenError> {
    let Some(preset) = &args.preset else {
        return Ok(args.clone());
    };
    if !args.steps.is_empty() {
        return Err(GenError::Invalid(
            "a plan sets either `preset` or `steps`, not both".to_string(),
        ));
    }
    let template = PRESETS
        .iter()
        .find(|(name, _)| name == preset)
        .map(|(_, template)| *template)
        .ok_or_else(|| {
            let names: Vec<&str> = PRESETS.iter().map(|(name, _)| *name).collect();
            GenError::Invalid(format!(
                "no preset `{preset}`; available: {}",
                names.join(", ")
            ))
        })?;
    let context = has_many_context(project, &args.vars)?;
    let mut renderer = rok_render::Renderer::new(rok_render::Trust::BuiltIn);
    renderer.add_template("preset", template)?;
    let plan_json = renderer.render("preset", &context)?;
    let plan: IntegrateArgs = serde_json::from_str(&plan_json).map_err(|err| {
        GenError::Invalid(format!("preset `{preset}` rendered an invalid plan: {err}"))
    })?;
    Ok(IntegrateArgs {
        name: args.name.clone().or(plan.name),
        preset: None,
        emit_plan: args.emit_plan.clone(),
        vars: Value::Null,
        steps: plan.steps,
        dry_run: args.dry_run,
        verify: args.verify,
    })
}

/// Writes the expanded plan as JSON (relative to the project root) instead
/// of applying it.
///
/// # Errors
///
/// [`GenError::Invalid`] when it can't be written.
pub fn emit(project: &Project, args: &IntegrateArgs, path: &str) -> Result<Generated, GenError> {
    let plan = IntegrateArgs {
        emit_plan: None,
        dry_run: false,
        verify: false,
        ..args.clone()
    };
    let mut text = serde_json::to_string_pretty(&plan)
        .map_err(|err| GenError::Invalid(format!("could not serialize the plan: {err}")))?;
    text.push('\n');
    let target = project.root.join(path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| GenError::Invalid(format!("could not write {path}: {err}")))?;
    }
    std::fs::write(&target, text)
        .map_err(|err| GenError::Invalid(format!("could not write {path}: {err}")))?;
    let mut outcome = Outcome::new("integrate", true);
    outcome.files_created = vec![path.to_string()];
    outcome.data = json!({ "emitted": path, "steps": plan.steps.len() });
    Ok(Generated {
        outcome,
        human: format!(
            "wrote {path} ({} steps); review it, then `cargo rok integrate -f {path}`\n",
            plan.steps.len()
        ),
    })
}

/// Names for `has-many` from `vars.source` / `vars.target` (and optional
/// `vars.fk`, default `<target singular>_id`).
fn has_many_context(project: &Project, vars: &Value) -> Result<Value, GenError> {
    let get = |key: &str| vars.get(key).and_then(Value::as_str).map(str::to_string);
    let source = get("source").ok_or_else(|| {
        GenError::Invalid("`has-many` needs `vars.source` (e.g. `comments`)".to_string())
    })?;
    let target = get("target").ok_or_else(|| {
        GenError::Invalid("`has-many` needs `vars.target` (e.g. `posts`)".to_string())
    })?;
    snake_ident(&source, "vars.source")?;
    snake_ident(&target, "vars.target")?;
    let source_s = naming::singular(&source);
    let target_s = naming::singular(&target);
    let fk = get("fk").unwrap_or_else(|| format!("{target_s}_id"));
    snake_ident(&fk, "vars.fk")?;

    // The foreign key must be a required `Uuid` on the source entity.
    let entity_file = format!("crates/{source}/src/{source_s}_entity.rs");
    let entity = std::fs::read_to_string(project.root.join(&entity_file)).map_err(|_| {
        GenError::Invalid(format!(
            "`has-many` needs {entity_file} (a feature made by `make:feature {source}`)"
        ))
    })?;
    if !entity.contains(&format!("pub {fk}: Uuid,")) {
        return Err(GenError::Invalid(format!(
            "`{}` has no required `{fk}: Uuid` field; add it (or pass `vars.fk`)",
            source_s.to_upper_camel_case()
        )));
    }

    Ok(json!({
        "source": source,
        "target": target,
        "source_s": source_s,
        "target_s": target_s,
        "Source": source_s.to_upper_camel_case(),
        "Target": target_s.to_upper_camel_case(),
        "SourceE": format!("{}Error", source.to_upper_camel_case()),
        "TargetE": format!("{}Error", target.to_upper_camel_case()),
        "source_table": source,
        "fk": fk,
        "source_words": source_s.replace('_', " "),
        "target_words": target_s.replace('_', " "),
    }))
}
