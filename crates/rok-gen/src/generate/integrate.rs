//! `cargo rok inspect` and `cargo rok integrate`.
//!
//! A plan's steps are applied in order to one transaction, each against the
//! text the previous steps produced: every selector is resolved, every
//! snippet rendered and checked, and every file formatted before anything
//! is written.

use rok_check::arch;
use rok_edit::Transaction;
use rok_locate::{Edit, Inspection};
use rok_plan::integrate::{InspectArgs, IntegrateArgs, Step};
use rok_plan::project::Project;
use rok_render::{Renderer, Trust};
use serde_json::{Value, json};

use super::alias::{resolve_file, resolve_in};
use super::{
    Apply, GenError, Generated, add_crate_deps, add_imports, composition_root_file, edit,
    finish_labeled, located, manifest, parse,
};

/// `cargo rok inspect`.
///
/// # Errors
///
/// [`GenError`] for an unknown alias or function, or a file that doesn't
/// parse.
pub fn inspect(project: &Project, args: &InspectArgs) -> Result<(String, Inspection), GenError> {
    let rel = resolve_file(project, &args.file)?;
    let mut tx = Transaction::new(&project.root);
    let source = parse(&mut tx, &rel)?;
    let inspection = source
        .inspect(args.function.as_deref())
        .map_err(|err| located(&rel, err))?;
    Ok((rel, inspection))
}

/// `cargo rok integrate`. `force` comes from the command line only: a plan
/// can't authorize a `replace` by itself.
///
/// # Errors
///
/// [`GenError`] for any step that can't be applied; nothing is written.
pub fn integrate(
    project: &Project,
    args: &IntegrateArgs,
    force: bool,
    allow_dirty: bool,
) -> Result<Generated, GenError> {
    let mut tx = Transaction::new(&project.root);
    if let Some(path) = &args.emit_plan {
        let expanded = super::presets::expand(&mut tx, args)?;
        return super::presets::emit(project, &expanded, path);
    }
    let applied = integrate_into(
        &mut tx,
        project,
        args,
        Permit {
            force,
            // A dry run writes nothing, so there is nothing to recover.
            allow_dirty: allow_dirty || args.dry_run,
        },
    )?;
    let mut generated = finish_labeled(
        "integrate",
        applied.label,
        tx,
        Apply {
            dry_run: args.dry_run,
            verify: args.verify,
        },
    )?;
    for warning in applied.warnings {
        generated.human.push_str("warning: ");
        generated.human.push_str(&warning);
        generated.human.push('\n');
        generated.outcome.warnings.push(warning);
    }
    Ok(generated)
}

/// What the command line allows a plan to do.
#[derive(Clone, Copy, Debug)]
pub struct Permit {
    /// Allow `replace` steps.
    pub force: bool,
    /// Allow replacing files with uncommitted changes.
    pub allow_dirty: bool,
}

/// What [`integrate_into`] did besides editing the transaction.
pub struct Integrated {
    /// The label the journal records (the plan's `name`, or derived).
    pub label: Option<String>,
    pub warnings: Vec<String>,
}

/// Applies a plan's steps to `tx` (what `integrate` and `apply` share).
/// The plan's `dry_run` and `verify` are left to the caller.
///
/// # Errors
///
/// [`GenError`] for a plan that can't be applied.
pub fn integrate_into(
    tx: &mut Transaction,
    project: &Project,
    args: &IntegrateArgs,
    permit: Permit,
) -> Result<Integrated, GenError> {
    let expanded = super::presets::expand(tx, args)?;
    let args = &expanded;
    if args.steps.is_empty() {
        return Err(GenError::Invalid("the plan has no `steps`".to_string()));
    }
    if !permit.force
        && args
            .steps
            .iter()
            .any(|step| matches!(step, Step::Replace { .. }))
    {
        return Err(GenError::Invalid(
            "the plan replaces code; pass --force on the command line to allow it".to_string(),
        ));
    }
    let vars = if args.vars.is_null() {
        json!({})
    } else {
        args.vars.clone()
    };
    let steps = render_steps(&args.steps, &vars)?;
    if !permit.allow_dirty {
        require_clean_replace_targets(project, tx, &steps)?;
    }
    let mut warnings = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        apply_step(project, tx, step, &mut warnings).map_err(|err| {
            GenError::Invalid(format!("step {} ({}): {err}", index + 1, op_name(step)))
        })?;
    }
    let label = args
        .name
        .clone()
        .or_else(|| derive_label(project, tx, &steps));
    Ok(Integrated { label, warnings })
}

/// `<dep crate> into <target crate>` from the first `inject_dependency` or
/// `inject_port`.
fn derive_label(project: &Project, tx: &Transaction, steps: &[Step]) -> Option<String> {
    steps.iter().find_map(|step| {
        let (into, dep) = match step {
            Step::InjectDependency { into, dep, .. } => (into, dep),
            Step::InjectPort {
                into, implementor, ..
            } => (into, implementor),
            _ => return None,
        };
        let target = crate_of(&resolve_in(tx, project, into).ok()?)?;
        let dep_crate = dep.split("::").next().filter(|first| *first != "crate")?;
        (dep_crate != target).then(|| format!("{dep_crate} into {target}"))
    })
}

/// `replace` overwrites code; with `--force` it still refuses files that
/// have uncommitted changes, so git can always bring them back.
fn require_clean_replace_targets(
    project: &Project,
    tx: &Transaction,
    steps: &[Step],
) -> Result<(), GenError> {
    let files: Vec<String> = steps
        .iter()
        .filter_map(|step| match step {
            Step::Replace { at, .. } => Some(resolve_in(tx, project, &at.file)),
            _ => None,
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        // Files the batch creates have nothing uncommitted to lose.
        .filter(|rel| project.root.join(rel).is_file())
        .collect();
    if files.is_empty() {
        return Ok(());
    }
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain", "--"])
        .args(&files)
        .current_dir(&project.root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .ok_or_else(|| {
            GenError::Invalid(
                "can't confirm the replaced files are committed (not a git repository?); \
                 pass --allow-dirty to replace anyway"
                    .to_string(),
            )
        })?;
    let dirty = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if dirty.is_empty() {
        Ok(())
    } else {
        Err(GenError::Invalid(format!(
            "these files have uncommitted changes, so a replace couldn't be recovered with git:\n{dirty}\n\
             commit them first, or pass --allow-dirty"
        )))
    }
}

const fn op_name(step: &Step) -> &'static str {
    match step {
        Step::Insert { .. } => "insert",
        Step::Replace { .. } => "replace",
        Step::AddField { .. } => "add_field",
        Step::InjectDependency { .. } => "inject_dependency",
        Step::AddMethod { .. } => "add_method",
        Step::Migration { .. } => "migration",
        Step::Make { .. } => "make",
        Step::NestRoutes { .. } => "nest_routes",
        Step::InjectPort { .. } => "inject_port",
    }
}

fn apply_step(
    project: &Project,
    tx: &mut Transaction,
    step: &Step,
    warnings: &mut Vec<String>,
) -> Result<(), GenError> {
    match step {
        Step::Insert { at, code } => insert_or_replace(project, tx, at, code, false),
        Step::Replace { at, code } => insert_or_replace(project, tx, at, code, true),
        Step::AddField {
            file,
            struct_name,
            field,
            default,
        } => add_field(project, tx, file, struct_name, field, default.as_deref()),
        Step::Make { command, args } => super::make_into(tx, project, command, args),
        Step::NestRoutes {
            module,
            under,
            param,
        } => nest_routes(project, tx, module, under, param),
        Step::InjectPort {
            into,
            service,
            port,
            name,
            implementor,
            methods,
            uses,
        } => inject_port(
            project,
            tx,
            &PortSpec {
                into,
                service,
                port,
                name,
                implementor,
                methods,
                uses,
            },
            warnings,
        ),
        Step::InjectDependency {
            into,
            service,
            dep,
            name,
        } => inject_dependency(project, tx, into, service, dep, name, warnings),
        Step::AddMethod {
            file,
            trait_name,
            signature,
            doc,
            bodies,
        } => add_method(project, tx, file, trait_name, signature, doc, bodies),
        Step::Migration { name, sql } => {
            super::names::snake_ident(name, "migration name")?;
            super::migration::create_next(
                tx,
                &project.config.project.migrations,
                name,
                &format!("{}\n", sql.trim_end()),
            )
            .map(drop)
        }
    }
}

/// `insert` and `replace`: check the snippet parses and only reads names in
/// scope at each place, then edit.
fn insert_or_replace(
    project: &Project,
    tx: &mut Transaction,
    at: &rok_plan::integrate::At,
    code: &str,
    replace: bool,
) -> Result<(), GenError> {
    rok_locate::check_stmts(code).map_err(|message| {
        GenError::Invalid(format!("the code isn't valid Rust statements: {message}"))
    })?;
    let free = rok_locate::free_variables(code).map_err(GenError::Invalid)?;
    let rel = resolve_in(tx, project, &at.file)?;
    let source = parse(tx, &rel)?;
    let places = source.resolve(at).map_err(|err| located(&rel, err))?;
    let mut edits = Vec::new();
    for place in &places {
        let change = if replace {
            source.replace_edit(place, code)
        } else {
            Ok(source.insert_edit(place, code))
        }
        .map_err(|err| located(&rel, err))?;
        let scope = source
            .names_in_scope(&at.function, change.offset)
            .map_err(|err| located(&rel, err))?;
        let missing: Vec<&String> = free.iter().filter(|name| !scope.contains(*name)).collect();
        if !missing.is_empty() {
            let line = source.text()[..change.offset].matches('\n').count() + 1;
            let visible: Vec<&str> = scope
                .iter()
                .map(String::as_str)
                .filter(|name| name.starts_with(char::is_lowercase))
                .collect();
            return Err(GenError::Invalid(format!(
                "the code reads {}, which isn't in scope at {rel}:{line} in `{}` (in scope: {})",
                missing
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", "),
                at.function,
                visible.join(", ")
            )));
        }
        edits.push(change);
    }
    edit(tx, &rel, &source, edits)
}

/// A struct field, and with `default`, its initializer in every literal of
/// the struct in the crate.
fn add_field(
    project: &Project,
    tx: &mut Transaction,
    file: &str,
    struct_name: &str,
    field: &str,
    default: Option<&str>,
) -> Result<(), GenError> {
    let parsed = syn::parse::Parser::parse_str(syn::Field::parse_named, field)
        .map_err(|err| GenError::Invalid(format!("`{field}` isn't a valid named field: {err}")))?;
    let rel = resolve_in(tx, project, file)?;
    let source = parse(tx, &rel)?;
    let fields = source
        .struct_fields(struct_name)
        .map_err(|err| located(&rel, err))?;
    edit(
        tx,
        &rel,
        &source,
        fields.append(&[field.to_string()]).into_iter().collect(),
    )?;
    let (Some(default), Some(ident)) = (default, &parsed.ident) else {
        return Ok(());
    };
    syn::parse_str::<syn::Expr>(default).map_err(|err| {
        GenError::Invalid(format!("`default` `{default}` isn't an expression: {err}"))
    })?;
    let initializer = format!("{ident}: {default}");
    let crate_name = crate_of(&rel)
        .ok_or_else(|| GenError::Invalid(format!("{rel} isn't inside a crates/<feature> crate")))?;
    for file in crate_files(tx, &crate_name) {
        let source = parse(tx, &file)?;
        let edits = source
            .struct_literal_sites(struct_name)
            .iter()
            .filter_map(|literal| literal.append(std::slice::from_ref(&initializer)))
            .collect();
        edit(tx, &file, &source, edits)?;
    }
    Ok(())
}

/// Moves `module`'s routes under `under`'s: its `#[module(prefix)]` and every
/// documented path in its crate get `<under prefix>/{param}` in front.
fn nest_routes(
    project: &Project,
    tx: &mut Transaction,
    module: &str,
    under: &str,
    param: &str,
) -> Result<(), GenError> {
    super::names::snake_ident(param, "param")?;
    let under_rel = resolve_in(tx, project, under)?;
    let under_prefix = parse(tx, &under_rel)?
        .module_prefix()
        .map_err(|err| located(&under_rel, err))?
        .map(|literal| literal.value)
        .ok_or_else(|| GenError::Invalid(format!("{under_rel}'s #[module] has no `prefix`")))?;
    let rel = resolve_in(tx, project, module)?;
    let source = parse(tx, &rel)?;
    let own = source
        .module_prefix()
        .map_err(|err| located(&rel, err))?
        .ok_or_else(|| GenError::Invalid(format!("{rel}'s #[module] has no `prefix`")))?;
    let nest = format!("{}/{{{param}}}", under_prefix.trim_end_matches('/'));
    if own.value.starts_with(&nest) {
        return Err(GenError::Conflict(format!(
            "`{}` is already nested under `{nest}`",
            own.value
        )));
    }
    let nested = format!("{nest}{}", own.value);
    edit(
        tx,
        &rel,
        &source,
        vec![Edit::replace(own.start, own.end, format!("\"{nested}\""))],
    )?;

    let crate_name = crate_of(&rel)
        .ok_or_else(|| GenError::Invalid(format!("{rel} isn't inside a crates/<feature> crate")))?;
    for file in crate_files(tx, &crate_name) {
        let source = parse(tx, &file)?;
        let edits = source
            .openapi_paths()
            .into_iter()
            .filter(|literal| literal.value.starts_with(&own.value))
            .map(|literal| {
                let rest = &literal.value[own.value.len()..];
                Edit::replace(literal.start, literal.end, format!("\"{nested}{rest}\""))
            })
            .collect();
        edit(tx, &file, &source, edits)?;
    }
    Ok(())
}

/// A method on a trait, with a body in every implementor in its crate.
fn add_method(
    project: &Project,
    tx: &mut Transaction,
    file: &str,
    trait_name: &str,
    signature: &str,
    doc: &[String],
    bodies: &std::collections::BTreeMap<String, String>,
) -> Result<(), GenError> {
    let parsed: syn::Signature = syn::parse_str(signature).map_err(|err| {
        GenError::Invalid(format!("`{signature}` isn't a function signature: {err}"))
    })?;
    let method = parsed.ident.to_string();
    for (owner, body) in bodies {
        syn::parse_str::<syn::Block>(&format!("{{\n{body}\n}}")).map_err(|err| {
            GenError::Invalid(format!("the body for `{owner}` isn't valid Rust: {err}"))
        })?;
    }

    let rel = resolve_in(tx, project, file)?;
    let source = parse(tx, &rel)?;
    let site = source
        .trait_site(trait_name)
        .map_err(|err| located(&rel, err))?;
    if site.functions.contains(&method) {
        return Err(GenError::Conflict(format!(
            "`{trait_name}` already has `{method}`"
        )));
    }
    let docs = doc.iter().fold(String::new(), |mut text, line| {
        text.push_str("///");
        if !line.is_empty() {
            text.push(' ');
            text.push_str(line);
        }
        text.push('\n');
        text
    });
    edit(
        tx,
        &rel,
        &source,
        vec![Edit::new(site.close, format!("\n{docs}{signature};\n"))],
    )?;

    let crate_name = crate_of(&rel)
        .ok_or_else(|| GenError::Invalid(format!("{rel} isn't inside a crates/<feature> crate")))?;
    let mut implementors: Vec<(String, bool)> = Vec::new();
    for crate_file in crate_files(tx, &crate_name) {
        let source = parse(tx, &crate_file)?;
        let mut edits = Vec::new();
        for implementation in source.impls_of(trait_name) {
            let covered = bodies.get(&implementation.owner).map(|body| {
                edits.push(Edit::new(
                    implementation.close,
                    format!("\n{signature} {{\n{body}\n}}\n"),
                ));
            });
            implementors.push((implementation.owner, covered.is_some()));
        }
        edit(tx, &crate_file, &source, edits)?;
    }
    let missing: Vec<&str> = implementors
        .iter()
        .filter(|(_, covered)| !covered)
        .map(|(owner, _)| owner.as_str())
        .collect();
    if !missing.is_empty() {
        return Err(GenError::Invalid(format!(
            "`{trait_name}` is also implemented by {}; give each a body in `bodies`",
            missing.join(", ")
        )));
    }
    let unknown: Vec<String> = bodies
        .keys()
        .filter(|owner| {
            !implementors
                .iter()
                .any(|(implementor, _)| implementor == *owner)
        })
        .map(|owner| format!("`{owner}`"))
        .collect();
    if !unknown.is_empty() {
        return Err(GenError::Invalid(format!(
            "no `impl {trait_name} for` {} in crate `{crate_name}`",
            unknown.join(", ")
        )));
    }
    Ok(())
}

/// Renders every string in every step as an atom template with `vars`, so
/// a plan is parameterizable throughout (file aliases, names, snippets).
fn render_steps(steps: &[Step], vars: &Value) -> Result<Vec<Step>, GenError> {
    fn walk(value: &mut Value, renderer: &mut Renderer, vars: &Value) -> Result<(), GenError> {
        match value {
            Value::String(text) if text.contains("{{") || text.contains("{%") => {
                renderer.add_template("snippet", text)?;
                *text = renderer.render("snippet", vars)?;
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, renderer, vars)?;
                }
            }
            Value::Object(object) => {
                for item in object.values_mut() {
                    walk(item, renderer, vars)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut renderer = Renderer::new(Trust::BuiltIn);
    let mut value = serde_json::to_value(steps)
        .map_err(|err| GenError::Invalid(format!("could not read the plan: {err}")))?;
    walk(&mut value, &mut renderer, vars)?;
    serde_json::from_value(value).map_err(|err| {
        GenError::Invalid(format!("the plan is invalid after rendering `vars`: {err}"))
    })
}

/// Constructor injection of the port `dep` (e.g.
/// `comments::CommentRepository`) into `service` as field `name`.
fn inject_dependency(
    project: &Project,
    tx: &mut Transaction,
    into: &str,
    service: &str,
    dep: &str,
    name: &str,
    warnings: &mut Vec<String>,
) -> Result<(), GenError> {
    super::names::snake_ident(name, "`as`")?;
    let rel = resolve_in(tx, project, into)?;
    let target = crate_of(&rel)
        .ok_or_else(|| GenError::Invalid(format!("{rel} isn't inside a crates/<feature> crate")))?;
    let (dep_crate, trait_name) = match dep.rsplit_once("::") {
        Some((path, trait_name)) => {
            let first = path.split("::").next().unwrap_or(path);
            let dep_crate = if first == "crate" {
                target.clone()
            } else {
                first.to_string()
            };
            (dep_crate, trait_name.to_string())
        }
        None => (target.clone(), dep.to_string()),
    };
    let cross_crate = dep_crate != target;
    let adapter = format!("Pg{trait_name}");
    let fake = format!("Fake{trait_name}");
    let dep_lib = format!("crates/{dep_crate}/src/lib.rs");
    let dep_exports = tx
        .read(&dep_lib)
        .map_err(|_| GenError::Invalid(format!("no crate `{dep_crate}` (expected {dep_lib})")))?;
    if !dep_exports.contains(&trait_name) || !dep_exports.contains(&adapter) {
        return Err(GenError::Invalid(format!(
            "`{dep_crate}` must export `{trait_name}` and its adapter `{adapter}`"
        )));
    }

    if cross_crate {
        let graph = arch::dependency_graph_from(&project.root, &Staged(tx))
            .map_err(|err| GenError::Invalid(err.to_string()))?;
        if arch::reaches(&graph, &dep_crate, &target) {
            return Err(GenError::Invalid(format!(
                "`{target}` depending on `{dep_crate}` would create a dependency cycle \
                 (`{dep_crate}` already depends on `{target}`); use an `inject_port` step \
                 instead: `{target}` owns a trait and `{dep_crate}` implements it"
            )));
        }
    }

    let trait_path = if cross_crate {
        format!("{dep_crate}::{trait_name}")
    } else {
        dep.replacen(&dep_crate, "crate", 1)
    };
    edit_service(tx, &rel, service, name, &trait_name, &trait_path)?;

    // Every other `Service::new(..)` in the crate is a test: pass the fake.
    let fake_path = if cross_crate {
        format!("{dep_crate}::{fake}")
    } else {
        format!("crate::{fake}")
    };
    if dep_exports.contains(&fake) {
        update_test_call_sites(
            tx,
            &target,
            service,
            &format!("std::sync::Arc::new({fake_path}::default())"),
        )?;
    } else {
        warnings.push(format!(
            "`{dep_crate}` exports no `{fake}`; update the tests that build `{service}` by hand"
        ));
    }

    wire_root(project, tx, service, &dep_crate, &adapter, warnings)?;

    if cross_crate {
        wire_crates(
            project,
            tx,
            &target,
            &dep_crate,
            dep_exports.contains(&fake),
        )?;
    }
    Ok(())
}

/// The service itself: field, `new` parameter, `Self { .. }` initializer
/// and imports.
fn edit_service(
    tx: &mut Transaction,
    rel: &str,
    service: &str,
    name: &str,
    trait_name: &str,
    trait_path: &str,
) -> Result<(), GenError> {
    let source = parse(tx, rel)?;
    let field = format!("{name}: Arc<dyn {trait_name}>");
    let fields = source
        .struct_fields(service)
        .map_err(|err| located(rel, err))?;
    let constructor = source
        .find_fn(&format!("{service}::new"))
        .map_err(|err| located(rel, err))?;
    let mut edits: Vec<Edit> = fields
        .append(std::slice::from_ref(&field))
        .into_iter()
        .collect();
    edits.extend(
        source
            .fn_params(&constructor)
            .append(std::slice::from_ref(&field)),
    );
    let literals = source.struct_literals(&constructor, service);
    if literals.is_empty() {
        return Err(GenError::Invalid(format!(
            "`{service}::new` doesn't build `Self {{ .. }}`; can't add the field initializer"
        )));
    }
    for literal in literals {
        edits.extend(literal.append(&[name.to_string()]));
    }
    edit(tx, rel, &source, edits)?;
    let mut uses = vec!["std::sync::Arc".to_string(), trait_path.to_string()];
    let imported = parse(tx, rel)?.imported_names();
    uses.retain(|path| !imported.contains(path.rsplit("::").next().unwrap_or(path)));
    add_imports(tx, rel, &uses)
}

/// The composition root passes the Postgres adapter to every
/// `service::new(..)` it makes.
fn wire_root(
    project: &Project,
    tx: &mut Transaction,
    service: &str,
    dep_crate: &str,
    adapter: &str,
    warnings: &mut Vec<String>,
) -> Result<(), GenError> {
    let root = composition_root_file(project)?;
    let root_source = parse(tx, &root)?;
    let calls: Vec<_> = root_source
        .call_sites(&format!("{service}::new"))
        .into_iter()
        .filter(|call| !call.in_test)
        .collect();
    if calls.is_empty() {
        warnings.push(format!(
            "{root} doesn't build `{service}`; nothing to wire there"
        ));
        return Ok(());
    }
    let argument = format!("Arc::new({adapter}::new(pool.clone()))");
    let root_edits = calls
        .iter()
        .filter_map(|call| call.args.append(std::slice::from_ref(&argument)))
        .collect();
    edit(tx, &root, &root_source, root_edits)?;
    let imported = parse(tx, &root)?.imported_names();
    let mut uses = Vec::new();
    if !imported.contains(adapter) {
        uses.push(format!("{dep_crate}::{adapter}"));
    }
    if !imported.contains("Arc") {
        uses.push("std::sync::Arc".to_string());
    }
    add_imports(tx, &root, &uses)
}

/// An `inject_port` step's fields.
struct PortSpec<'a> {
    into: &'a str,
    service: &'a str,
    port: &'a str,
    name: &'a str,
    implementor: &'a str,
    methods: &'a [rok_plan::integrate::PortMethod],
    uses: &'a [String],
}

/// The `port` strategy: the target owns the trait (and a fake), the source
/// implements it for its adapter, the composition root passes the adapter.
fn inject_port(
    project: &Project,
    tx: &mut Transaction,
    spec: &PortSpec<'_>,
    warnings: &mut Vec<String>,
) -> Result<(), GenError> {
    use heck::ToSnakeCase;

    super::names::snake_ident(spec.name, "`as`")?;
    super::names::pascal_ident(spec.port, "port")?;
    if spec.methods.is_empty() {
        return Err(GenError::Invalid(
            "a port needs at least one method".to_string(),
        ));
    }
    for method in spec.methods {
        syn::parse_str::<syn::Signature>(&method.signature).map_err(|err| {
            GenError::Invalid(format!("`{}` isn't a signature: {err}", method.signature))
        })?;
        for (what, body) in [("body", &method.body), ("fake", &method.fake)] {
            syn::parse_str::<syn::Block>(&format!("{{\n{body}\n}}")).map_err(|err| {
                GenError::Invalid(format!(
                    "the {what} of `{}` isn't valid Rust: {err}",
                    method.signature
                ))
            })?;
        }
    }

    let rel = resolve_in(tx, project, spec.into)?;
    let target = crate_of(&rel)
        .ok_or_else(|| GenError::Invalid(format!("{rel} isn't inside a crates/<feature> crate")))?;
    let (source, adapter) = spec.implementor.split_once("::").ok_or_else(|| {
        GenError::Invalid(format!(
            "`implementor` is `{}`; write it as `<crate>::<Type>`",
            spec.implementor
        ))
    })?;
    if source == target {
        return Err(GenError::Invalid(format!(
            "`{source}` is the target crate itself; use `inject_dependency`"
        )));
    }
    let graph = arch::dependency_graph_from(&project.root, &Staged(tx))
        .map_err(|err| GenError::Invalid(err.to_string()))?;
    if arch::reaches(&graph, &target, source) {
        return Err(GenError::Invalid(format!(
            "`{target}` already depends on `{source}`, so `{source}` can't implement a port of \
             `{target}` without a cycle; use `inject_dependency`"
        )));
    }

    // The port, in the target crate.
    let module = spec.port.to_snake_case();
    let src = format!("crates/{target}/src");
    let port_file = format!("{src}/{module}.rs");
    if tx.exists(&port_file) {
        return Err(GenError::Conflict(format!("{port_file} already exists")));
    }
    let context = serde_json::json!({
        "target": target,
        "port": spec.port,
        "methods": spec.methods,
        "uses": spec.uses,
    });
    let renderer = super::templates::renderer()?;
    tx.create(
        &port_file,
        renderer.render_rust("port/port.rs", &context, rok_render::SyntaxKind::File)?,
    )?;
    super::export(tx, &format!("{src}/lib.rs"), &module, spec.port)?;
    super::insert_after_items(
        tx,
        &format!("{src}/lib.rs"),
        |item| matches!(item, syn::Item::Use(item) if matches!(item.vis, syn::Visibility::Public(_))),
        &[format!(
            "#[cfg(any(test, feature = \"fake\"))]\npub use {module}::fake::Fake{};",
            spec.port
        )],
    )?;
    let target_manifest = format!("crates/{target}/Cargo.toml");
    add_crate_deps(tx, &target_manifest, &["async-trait"])?;
    super::ensure_fake_feature(tx, &target_manifest)?;

    // The service depends on the port; its tests get the port's fake.
    let trait_path = format!("crate::{module}::{}", spec.port);
    edit_service(tx, &rel, spec.service, spec.name, spec.port, &trait_path)?;
    update_test_call_sites(
        tx,
        &target,
        spec.service,
        &format!("std::sync::Arc::new(crate::Fake{}::default())", spec.port),
    )?;

    implement_port(tx, &target, source, adapter, spec)?;
    wire_root(project, tx, spec.service, source, adapter, warnings)?;

    // `source` now depends on `target`.
    add_crate_deps(
        tx,
        &format!("crates/{source}/Cargo.toml"),
        &[target.as_str(), "async-trait"],
    )?;
    let features = arch::feature_crate_names_from(project, &Staged(tx))
        .map_err(|err| GenError::Invalid(err.to_string()))?;
    if features.contains(source) && features.contains(&target) {
        allow_edge(tx, source, &target)?;
    }
    Ok(())
}

/// `impl target::Port for Adapter { .. }`, appended to the file declaring
/// the adapter in the source crate.
fn implement_port(
    tx: &mut Transaction,
    target: &str,
    source: &str,
    adapter: &str,
    spec: &PortSpec<'_>,
) -> Result<(), GenError> {
    let declaration = format!("pub struct {adapter}");
    let file = crate_files(tx, source)
        .into_iter()
        .find(|file| tx.read(file).is_ok_and(|text| text.contains(&declaration)))
        .ok_or_else(|| GenError::Invalid(format!("no `{declaration}` in crate `{source}`")))?;
    let methods = spec.methods.iter().fold(String::new(), |mut text, method| {
        text.push_str(&method.signature);
        text.push_str(" {\n");
        text.push_str(&method.body);
        text.push_str("\n}\n");
        text
    });
    let text = tx.read(&file)?;
    let port = spec.port;
    tx.update(
        &file,
        format!(
            "{text}\n#[async_trait::async_trait]\nimpl {target}::{port} for {adapter} {{\n{methods}}}\n"
        ),
    )?;
    let imported = parse(tx, &file)?.imported_names();
    let uses: Vec<String> = spec
        .uses
        .iter()
        .filter(|path| !imported.contains(path.rsplit("::").next().unwrap_or(path)))
        .cloned()
        .collect();
    add_imports(tx, &file, &uses)
}

/// Appends `argument` to every `service::new(..)` inside the target
/// crate (its own tests).
fn update_test_call_sites(
    tx: &mut Transaction,
    target: &str,
    service: &str,
    argument: &str,
) -> Result<(), GenError> {
    for rel in crate_files(tx, target) {
        let source = parse(tx, &rel)?;
        // `Arc` unqualified where the file imports it (the project may lint
        // unnecessary qualifications).
        let argument = if source.imported_names().contains("Arc") {
            argument.replacen("std::sync::Arc", "Arc", 1)
        } else {
            argument.to_string()
        };
        let edits: Vec<Edit> = source
            .call_sites(&format!("{service}::new"))
            .iter()
            .filter_map(|call| call.args.append(std::slice::from_ref(&argument)))
            .collect();
        edit(tx, &rel, &source, edits)?;
    }
    Ok(())
}

/// Every `.rs` file of `crates/<name>/src`, relative to the root, sorted.
fn crate_files(tx: &Transaction, name: &str) -> Vec<String> {
    let src = tx.root().join(format!("crates/{name}/src"));
    let mut files: Vec<String> = walkdir::WalkDir::new(&src)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "rs"))
        .filter_map(|entry| {
            entry
                .path()
                .strip_prefix(tx.root())
                .ok()
                .map(|path| path.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    // Files staged earlier in the transaction (e.g. by a `make` step or an
    // earlier command of a batch) count; staged deletions don't.
    let prefix = format!("crates/{name}/src/");
    files.extend(tx.created().into_iter().filter(|rel| {
        rel.starts_with(&prefix)
            && std::path::Path::new(rel)
                .extension()
                .is_some_and(|ext| ext == "rs")
    }));
    let deleted = tx.deleted();
    files.retain(|rel| !deleted.contains(rel));
    files.sort();
    files.dedup();
    files
}

/// The cross-crate edge: dependency, test dependency with the dep's fakes,
/// and the architecture allowlist entry.
fn wire_crates(
    project: &Project,
    tx: &mut Transaction,
    target: &str,
    dep_crate: &str,
    has_fake: bool,
) -> Result<(), GenError> {
    let manifest_rel = format!("crates/{target}/Cargo.toml");
    add_crate_deps(tx, &manifest_rel, &[dep_crate])?;
    if has_fake {
        let internal = manifest::internal_crates(&tx.read("Cargo.toml")?);
        let text = tx.read(&manifest_rel)?;
        let updated = manifest::add_dependency(
            &text,
            "dev-dependencies",
            dep_crate,
            &format!("{dep_crate} = {{ workspace = true, features = [\"fake\"] }}"),
            &internal,
        );
        tx.update(&manifest_rel, updated)?;
    }
    let features = arch::feature_crate_names_from(project, &Staged(tx))
        .map_err(|err| GenError::Invalid(err.to_string()))?;
    if features.contains(target) && features.contains(dep_crate) {
        allow_edge(tx, target, dep_crate)?;
    }
    Ok(())
}

/// Adds `"<from> -> <to>"` to `[architecture].allow` in `rok.toml`.
fn allow_edge(tx: &mut Transaction, from: &str, to: &str) -> Result<(), GenError> {
    let entry = format!("\"{from} -> {to}\"");
    let text = tx.read("rok.toml")?;
    let config: Option<rok_plan::project::ProjectConfig> = toml::from_str(&text).ok();
    let already = config.is_some_and(|config| {
        config
            .architecture
            .allowed_edges()
            .is_ok_and(|edges| edges.iter().any(|(a, b)| a == from && b == to))
    });
    if already {
        return Ok(());
    }
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let allow_line = lines
        .iter()
        .position(|line| line.trim_start().starts_with("allow"));
    match allow_line {
        Some(start) => {
            let close = (start..lines.len())
                .find(|index| lines[*index].contains(']'))
                .unwrap_or(start);
            let line = &mut lines[close];
            let bracket = line.rfind(']').unwrap_or(line.len());
            let before = line[..bracket].trim_end();
            let separator = if before.ends_with('[') || before.ends_with(',') {
                ""
            } else {
                ", "
            };
            if close == start {
                line.insert_str(bracket, &format!("{separator}{entry}"));
            } else {
                lines.insert(close, format!("    {entry},"));
            }
        }
        None => {
            if let Some(header) = lines
                .iter()
                .position(|line| line.trim() == "[architecture]")
            {
                lines.insert(header + 1, format!("allow = [{entry}]"));
            } else {
                lines.push(String::new());
                lines.push("[architecture]".to_string());
                lines.push(format!("allow = [{entry}]"));
            }
        }
    }
    let mut updated = lines.join("\n");
    updated.push('\n');
    tx.update("rok.toml", updated)?;
    Ok(())
}

/// `crates/<name>/...` → `<name>`.
fn crate_of(rel: &str) -> Option<String> {
    rel.strip_prefix("crates/")?
        .split('/')
        .next()
        .map(str::to_string)
}

/// The workspace as `tx` will leave it, for the architecture reader, so
/// crates and edges staged earlier (by a `make` step or an earlier command
/// of a batch) count.
struct Staged<'a>(&'a Transaction);

impl Staged<'_> {
    fn rel(&self, path: &std::path::Path) -> String {
        path.strip_prefix(self.0.root())
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }
}

impl rok_check::Files for Staged<'_> {
    fn read(&self, path: &std::path::Path) -> std::io::Result<String> {
        self.0
            .peek(&self.rel(path))
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
    }

    fn children(&self, dir: &std::path::Path) -> std::io::Result<Vec<std::path::PathBuf>> {
        Ok(self
            .0
            .children(&self.rel(dir))
            .into_iter()
            .map(|name| dir.join(name))
            .collect())
    }

    fn is_file(&self, path: &std::path::Path) -> bool {
        self.0.peek(&self.rel(path)).is_some()
    }
}
