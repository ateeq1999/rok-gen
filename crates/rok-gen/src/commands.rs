//! Executing a parsed command: resolve the payload, find the project, run.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use rok_plan::check::{CheckArgs, Gate};
use rok_plan::hooks::HooksInstallArgs;
use rok_plan::input::{self, Layers, Resolved};
use rok_plan::make::NewArgs;
use rok_plan::outcome::Outcome;
use rok_plan::project::Project;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::cli::{Cli, Command, HooksCmd, InputFlags, OutputFormat};
use crate::{CliError, EXIT_FAILED, EXIT_OK, Execution, output};

/// First line of every hook `cargo rok hooks install` writes, so a re-run
/// can tell its own hook from someone else's.
const HOOK_SIGNATURE: &str = "# Installed by `cargo rok hooks install`.";

pub fn execute(
    cli: Cli,
    cwd: &Path,
    stdin: &mut dyn Read,
    prompts: &mut dyn std::io::Write,
) -> Execution {
    let format = cli.output;
    let (name, result) = match cli.command {
        Command::Check(cmd) => {
            let flags = cmd.flag_layer();
            ("check", check(format, &cmd.input, flags, cwd, stdin))
        }
        Command::Hooks(HooksCmd::Install(cmd)) => {
            let flags = cmd.flag_layer();
            (
                "hooks install",
                hooks_install(format, &cmd.input, flags, cwd, stdin),
            )
        }
        Command::Schema { name } => ("schema", schema(&name)),
        Command::Routes => ("routes", routes(format, cwd)),
        Command::Inspect(cmd) => {
            let flags = cmd.flag_layer();
            ("inspect", inspect(format, &cmd.input, flags, cwd, stdin))
        }
        Command::Integrate(cmd) if cmd.wizard => (
            "integrate",
            wizard_integrate(format, &cmd, cwd, stdin, prompts),
        ),
        Command::Integrate(cmd) => {
            let (force, allow_dirty) = (cmd.force, cmd.allow_dirty);
            (
                "integrate",
                cmd.flag_layer().map_err(CliError::Flag).and_then(|flags| {
                    generate(format, &cmd.input, flags, cwd, stdin, |project, args| {
                        crate::generate::integrate(project, args, force, allow_dirty)
                    })
                }),
            )
        }
        Command::Apply(cmd) => ("apply", apply(format, &cmd, cwd, stdin)),
        Command::Undo(cmd) => (
            "undo",
            journal_command(format, cwd, |project| {
                crate::generate::undo(project, cmd.id.as_deref(), cmd.dry_run)
            }),
        ),
        Command::Unlink(cmd) => (
            "unlink",
            journal_command(format, cwd, |project| {
                crate::generate::unlink(project, &cmd.source, &cmd.target, cmd.dry_run)
            }),
        ),
        Command::Journal => (
            "journal",
            journal_command(format, cwd, crate::generate::journal_list),
        ),
        Command::New(cmd) => ("new", new(format, &cmd, cwd, stdin)),
        Command::Adopt(cmd) => ("adopt", adopt(format, &cmd, cwd, stdin)),
        make => run_generator(make, format, cwd, stdin),
    };
    result.unwrap_or_else(|err| output::error(format, name, &err))
}

/// The `make:*` commands.
fn run_generator(
    command: Command,
    format: OutputFormat,
    cwd: &Path,
    stdin: &mut dyn Read,
) -> (&'static str, Result<Execution, CliError>) {
    match command {
        Command::MakeFeature(cmd) => (
            "make:feature",
            cmd.flag_layer().map_err(CliError::Flag).and_then(|flags| {
                generate(format, &cmd.input, flags, cwd, stdin, |project, args| {
                    crate::generate::make_feature(project, args)
                })
            }),
        ),
        Command::MakeAction(cmd) => {
            let flags = cmd.flag_layer();
            (
                "make:action",
                generate(format, &cmd.input, flags, cwd, stdin, |project, args| {
                    crate::generate::make_action(project, args)
                }),
            )
        }
        Command::MakeDto(cmd) => (
            "make:dto",
            cmd.flag_layer().map_err(CliError::Flag).and_then(|flags| {
                generate(format, &cmd.input, flags, cwd, stdin, |project, args| {
                    crate::generate::make_dto(project, args)
                })
            }),
        ),
        Command::MakeMigration(cmd) => {
            let flags = cmd.flag_layer();
            (
                "make:migration",
                generate(format, &cmd.input, flags, cwd, stdin, |project, args| {
                    crate::generate::make_migration(project, args)
                }),
            )
        }
        Command::MakeEntity(cmd) => (
            "make:entity",
            cmd.flag_layer().map_err(CliError::Flag).and_then(|flags| {
                generate(format, &cmd.input, flags, cwd, stdin, |project, args| {
                    crate::generate::make_entity(project, args)
                })
            }),
        ),
        Command::MakeRepository(cmd) => (
            "make:repository",
            cmd.flag_layer().map_err(CliError::Flag).and_then(|flags| {
                generate(format, &cmd.input, flags, cwd, stdin, |project, args| {
                    crate::generate::make_repository(project, args)
                })
            }),
        ),
        Command::MakeService(cmd) => {
            let flags = cmd.flag_layer();
            (
                "make:service",
                generate(format, &cmd.input, flags, cwd, stdin, |project, args| {
                    crate::generate::make_service(project, args)
                }),
            )
        }
        Command::MakeError(cmd) => {
            let flags = cmd.flag_layer();
            (
                "make:error",
                generate(format, &cmd.input, flags, cwd, stdin, |project, args| {
                    crate::generate::make_error(project, args)
                }),
            )
        }
        _ => (
            "cargo rok",
            Err(CliError::Flag("not a generator command".to_string())),
        ),
    }
}

fn journal_command(
    format: OutputFormat,
    cwd: &Path,
    run: impl FnOnce(&Project) -> Result<crate::generate::Generated, crate::generate::GenError>,
) -> Result<Execution, CliError> {
    let project = project(cwd)?;
    let generated = run(&project)?;
    Ok(output::outcome(
        format,
        &generated.outcome,
        generated.human,
        EXIT_OK,
    ))
}

fn inspect(
    format: OutputFormat,
    input: &InputFlags,
    flags: Value,
    cwd: &Path,
    stdin: &mut dyn Read,
) -> Result<Execution, CliError> {
    let resolved: Resolved<rok_plan::integrate::InspectArgs> = resolve(input, flags, cwd, stdin)?;
    if input.print_payload {
        return Ok(output::json_value(&resolved.payload));
    }
    let project = project(cwd)?;
    let (file, inspection) = crate::generate::inspect(&project, &resolved.args)?;
    let mut outcome = Outcome::new("inspect", true);
    outcome.data = serde_json::json!({ "file": file, "inspection": inspection });
    let human = format!("{file}\n{}", inspection.render());
    Ok(output::outcome(format, &outcome, human, EXIT_OK))
}

fn routes(format: OutputFormat, cwd: &Path) -> Result<Execution, CliError> {
    let project = project(cwd)?;
    let routes = crate::routes::routes(&project);
    let mut outcome = Outcome::new("routes", true);
    outcome.data = serde_json::json!({ "routes": routes });
    Ok(output::outcome(
        format,
        &outcome,
        crate::routes::table(&routes),
        EXIT_OK,
    ))
}

/// Resolves a generator's payload, finds the project and runs it.
fn generate<T>(
    format: OutputFormat,
    input: &InputFlags,
    flags: Value,
    cwd: &Path,
    stdin: &mut dyn Read,
    run: impl FnOnce(&Project, &T) -> Result<crate::generate::Generated, crate::generate::GenError>,
) -> Result<Execution, CliError>
where
    T: DeserializeOwned + Serialize + Default,
{
    let resolved: Resolved<T> = resolve(input, flags, cwd, stdin)?;
    if input.print_payload {
        return Ok(output::json_value(&resolved.payload));
    }
    let project = project(cwd)?;
    let generated = run(&project, &resolved.args)?;
    Ok(output::outcome(
        format,
        &generated.outcome,
        generated.human,
        EXIT_OK,
    ))
}

fn resolve<T>(
    input: &InputFlags,
    flags: Value,
    cwd: &Path,
    stdin: &mut dyn Read,
) -> Result<Resolved<T>, CliError>
where
    T: DeserializeOwned + Serialize + Default,
{
    let file = input
        .file
        .as_deref()
        .map(|path| input::read_file_layer(path, cwd, stdin))
        .transpose()?;
    let inline = input
        .json
        .as_deref()
        .map(|text| input::parse(text, input::Format::Json, "--json"))
        .transpose()?;
    Ok(input::resolve(Layers {
        file,
        inline,
        flags: Some(flags),
    })?)
}

fn project(cwd: &Path) -> Result<Project, CliError> {
    let project = Project::discover(cwd)?;
    project.ensure_compatible(env!("CARGO_PKG_VERSION"))?;
    Ok(project)
}

fn check(
    format: OutputFormat,
    input: &InputFlags,
    flags: Value,
    cwd: &Path,
    stdin: &mut dyn Read,
) -> Result<Execution, CliError> {
    let resolved: Resolved<CheckArgs> = resolve(input, flags, cwd, stdin)?;
    if input.print_payload {
        return Ok(output::json_value(&resolved.payload));
    }
    let args = resolved.args;
    let project = project(cwd)?;

    let report = rok_check::gates::run(&project, &args);
    let ok = report.ok();
    let mut outcome = Outcome::new("check", ok);
    outcome.warnings = report
        .gates
        .iter()
        .filter(|result| result.status == rok_check::GateStatus::Skipped)
        .map(|result| {
            format!(
                "{} skipped: {}",
                result.gate,
                result.detail.as_deref().unwrap_or_default()
            )
        })
        .collect();
    outcome.data = serde_json::to_value(&report).unwrap_or_default();

    let code = if ok { EXIT_OK } else { EXIT_FAILED };
    let mut execution = output::outcome(format, &outcome, output::check_report(&report), code);
    if args.ci {
        let annotations = output::annotations(&report);
        match format {
            OutputFormat::Human => execution.stdout.insert_str(0, &annotations),
            // Keep stdout a single parseable JSON document.
            OutputFormat::Json => execution.stderr.push_str(&annotations),
        }
    }
    Ok(execution)
}

fn hooks_install(
    format: OutputFormat,
    input: &InputFlags,
    flags: Value,
    cwd: &Path,
    stdin: &mut dyn Read,
) -> Result<Execution, CliError> {
    let resolved: Resolved<HooksInstallArgs> = resolve(input, flags, cwd, stdin)?;
    if input.print_payload {
        return Ok(output::json_value(&resolved.payload));
    }
    let args = resolved.args;
    let project = project(cwd)?;

    let hooks_dir = git_hooks_dir(&project.root)?;
    let hook = hooks_dir.join("pre-commit");
    let existed = hook.is_file();
    if existed && !args.overwrite {
        let current = std::fs::read_to_string(&hook).unwrap_or_default();
        if !current.contains(HOOK_SIGNATURE) {
            return Err(CliError::HookExists { path: hook });
        }
    }

    let gates = if args.gates.is_empty() {
        Gate::PRE_COMMIT.to_vec()
    } else {
        args.gates
    };
    let gate_list = gates
        .iter()
        .map(|gate| gate.name())
        .collect::<Vec<_>>()
        .join(" ");
    let script = format!(
        "#!/bin/sh\n{HOOK_SIGNATURE} Re-run it to change the gates.\nexec cargo rok check {gate_list}\n"
    );
    std::fs::create_dir_all(&hooks_dir)
        .and_then(|()| std::fs::write(&hook, script))
        .map_err(|source| CliError::Write {
            path: hook.clone(),
            source,
        })?;
    make_executable(&hook)?;

    let shown = hook
        .strip_prefix(&project.root)
        .unwrap_or(&hook)
        .to_string_lossy()
        .replace('\\', "/");
    let mut outcome = Outcome::new("hooks install", true);
    if existed {
        outcome.files_changed.push(shown.clone());
    } else {
        outcome.files_created.push(shown.clone());
    }
    let human =
        format!("installed {shown}: runs `cargo rok check {gate_list}` before each commit\n");
    Ok(output::outcome(format, &outcome, human, EXIT_OK))
}

fn schema(name: &str) -> Result<Execution, CliError> {
    rok_plan::schema::schema_for(name)
        .map(|schema| output::json_value(&schema))
        .ok_or_else(|| CliError::UnknownSchema {
            name: name.to_string(),
        })
}

fn git_hooks_dir(root: &Path) -> Result<PathBuf, CliError> {
    let output = Process::new("git")
        .args(["rev-parse", "--git-path", "hooks"])
        .current_dir(root)
        .output()
        .map_err(|err| CliError::NotGit {
            message: format!("could not run git: {err}"),
        })?;
    if !output.status.success() {
        return Err(CliError::NotGit {
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    Ok(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), CliError> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(|source| {
        CliError::Write {
            path: path.to_path_buf(),
            source,
        }
    })
}

/// Git for Windows runs hooks through its own `sh`; no mode bit needed.
#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
const fn make_executable(_path: &Path) -> Result<(), CliError> {
    Ok(())
}

/// `cargo rok integrate --wizard`.
fn wizard_integrate(
    format: OutputFormat,
    cmd: &crate::cli::IntegrateCmd,
    cwd: &Path,
    stdin: &mut dyn Read,
    prompts: &mut dyn std::io::Write,
) -> Result<Execution, CliError> {
    let project = project(cwd)?;
    let mut reader = std::io::BufReader::new(stdin);
    let (mut args, finish) = crate::wizard::plan(&project, &mut reader, prompts)?;
    args.verify = cmd.apply.verify;
    match finish {
        crate::wizard::Finish::Apply => args.dry_run = cmd.apply.dry_run,
        crate::wizard::Finish::DryRun => args.dry_run = true,
        crate::wizard::Finish::Save(path) => args.emit_plan = Some(path),
    }
    let generated = crate::generate::integrate(&project, &args, cmd.force, cmd.allow_dirty)?;
    Ok(output::outcome(
        format,
        &generated.outcome,
        generated.human,
        EXIT_OK,
    ))
}

/// `cargo rok apply`: exits 1 when a `check` step failed (the batch was
/// rolled back).
fn apply(
    format: OutputFormat,
    cmd: &crate::cli::ApplyCmd,
    cwd: &Path,
    stdin: &mut dyn Read,
) -> Result<Execution, CliError> {
    let resolved: Resolved<rok_plan::apply::ApplyArgs> =
        resolve(&cmd.input, cmd.flag_layer(), cwd, stdin)?;
    if cmd.input.print_payload {
        return Ok(output::json_value(&resolved.payload));
    }
    let project = project(cwd)?;
    let permit = crate::generate::Permit {
        force: cmd.force,
        allow_dirty: cmd.allow_dirty,
    };
    let generated = crate::generate::apply(&project, &resolved.args, permit)?;
    let code = if generated.outcome.ok {
        EXIT_OK
    } else {
        EXIT_FAILED
    };
    Ok(output::outcome(
        format,
        &generated.outcome,
        generated.human,
        code,
    ))
}

/// `cargo rok new`: creates a new project from the reference template.
fn new(
    format: OutputFormat,
    cmd: &crate::cli::NewCmd,
    cwd: &Path,
    stdin: &mut dyn Read,
) -> Result<Execution, CliError> {
    let flags = cmd.flag_layer().map_err(CliError::Flag)?;
    let resolved: Resolved<NewArgs> = resolve(&cmd.input, flags, cwd, stdin)?;
    if cmd.input.print_payload {
        return Ok(output::json_value(&resolved.payload));
    }
    let args = resolved.args;

    if args.dry_run {
        let mut outcome = Outcome::new("new", true);
        outcome.data = serde_json::json!({ "dry_run": true, "project": args.name });
        let human = format!(
            "Would create project `{}` in {}/{}",
            args.name,
            cwd.display(),
            args.name
        );
        return Ok(output::outcome(format, &outcome, human, EXIT_OK));
    }

    // Create the project directory
    let project_dir = cwd.join(&args.name);
    if project_dir.exists() {
        return Err(CliError::Invalid(format!(
            "directory `{}` already exists",
            project_dir.display()
        )));
    }
    std::fs::create_dir_all(&project_dir).map_err(|source| CliError::Write {
        path: project_dir.clone(),
        source,
    })?;

    // Create the project structure
    create_new_project(&project_dir, &args)?;

    if args.verify {
        // Run cargo check to verify the project compiles
        let output = std::process::Command::new("cargo")
            .args(["check"])
            .current_dir(&project_dir)
            .output()
            .map_err(|err| CliError::Invalid(format!("could not run cargo check: {err}")))?;
        if !output.status.success() {
            // Clean up on failure
            let _ = std::fs::remove_dir_all(&project_dir);
            return Err(CliError::Invalid(format!(
                "cargo check failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
    }

    let mut outcome = Outcome::new("new", true);
    outcome.files_created.push(args.name.clone());
    outcome.data =
        serde_json::json!({ "dry_run": false, "verified": args.verify, "project": args.name });

    let human = format!(
        "Created project `{}` in {}\n",
        args.name,
        project_dir.display()
    );
    Ok(output::outcome(format, &outcome, human, EXIT_OK))
}

fn create_new_project(project_dir: &Path, args: &NewArgs) -> Result<(), CliError> {
    // Create workspace Cargo.toml
    let root_cargo_toml = format!(
        r#"[workspace]
members = ["crates/*"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.92"
license = "MIT"

[workspace.dependencies]
actix-web = "4"
async-trait = "0.1"
chrono = {{ version = "0.4", features = ["serde"] }}
db = {{ path = "crates/db" }}
envelope = {{ path = "crates/envelope" }}
error = {{ path = "crates/error" }}
macros = {{ path = "crates/macros" }}
pagination = {{ path = "crates/pagination" }}
session = {{ path = "crates/session" }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
sqlx = {{ version = "0.8", features = ["runtime-tokio-rustls", "postgres", "uuid", "chrono", "json", "migrate"] }}
thiserror = "2"
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
tracing = "0.1"
utoipa = {{ version = "5", features = ["actix_extras", "chrono", "uuid", "url"] }}
uuid = {{ version = "1", features = ["serde", "v4"] }}
validator = {{ version = "0.19", features = ["derive"] }}
validation = {{ path = "crates/validation" }}

[workspace.lints.rust]
trivial_casts = "warn"
trivial_numeric_casts = "warn"
unused_qualifications = "warn"
unsafe_code = "forbid"

[workspace.lints.clippy]
all = {{ level = "deny", priority = -1 }}
pedantic = {{ level = "warn", priority = -1 }}
nursery = {{ level = "warn", priority = -1 }}
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
todo = "deny"
unimplemented = "deny"
dbg_macro = "deny"
print_stdout = "deny"
print_stderr = "deny"
module_name_repetitions = "allow"

[package]
name = "{}"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
actix-web = {{ workspace = true }}
db = {{ workspace = true }}
envelope = {{ workspace = true }}
router = {{ path = "crates/router" }}
validation = {{ workspace = true }}
"#,
        args.name
    );
    std::fs::write(project_dir.join("Cargo.toml"), root_cargo_toml).map_err(|source| {
        CliError::Write {
            path: project_dir.join("Cargo.toml"),
            source,
        }
    })?;

    // Create rok.toml
    let rok_toml = format!(
        r#"[rok]
cli = "^{}"
templates = "0.1.0"

[project]
composition_root = "src"
composition_fn = "run"
router = "crates/router/src/lib.rs"
errors = "crates/error/src/lib.rs"
migrations = "crates/db/migrations"

[architecture]
allow = []
"#,
        env!("CARGO_PKG_VERSION")
    );
    std::fs::write(project_dir.join("rok.toml"), rok_toml).map_err(|source| CliError::Write {
        path: project_dir.join("rok.toml"),
        source,
    })?;

    // Create .gitignore
    let gitignore = r"/target
**/*.rs.bk
.env
*.db
";
    std::fs::write(project_dir.join(".gitignore"), gitignore).map_err(|source| {
        CliError::Write {
            path: project_dir.join(".gitignore"),
            source,
        }
    })?;

    // Create root lib.rs (composition root)
    let root_lib = r#"//! Composition root.

use std::sync::Arc;

use actix_web::{App, HttpServer, web};
use envelope::ApiResponse;

/// A minimal placeholder service to establish the service registration pattern.
struct SeedService;

impl SeedService {
    fn new() -> Self {
        SeedService
    }
}

pub async fn run() -> std::io::Result<()> {
    let pool = db::connect();

    let seed_service = SeedService::new();

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(seed_service.clone()))
            .configure(router::configure)
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
"#;
    std::fs::create_dir_all(project_dir.join("src")).map_err(|source| CliError::Write {
        path: project_dir.join("src"),
        source,
    })?;
    std::fs::write(project_dir.join("src/lib.rs"), root_lib).map_err(|source| CliError::Write {
        path: project_dir.join("src/lib.rs"),
        source,
    })?;

    // Create crates directory
    std::fs::create_dir_all(project_dir.join("crates")).map_err(|source| CliError::Write {
        path: project_dir.join("crates"),
        source,
    })?;

    // Create shared crates
    create_shared_crates(project_dir)?;

    // Create router crate
    create_router_crate(project_dir)?;

    // Create error crate
    create_error_crate(project_dir)?;

    // Create db crate with migrations
    create_db_crate(project_dir)?;

    // Initialize git if requested
    if args.git {
        init_git(project_dir)?;
    }

    // Create CI workflow if requested
    if args.ci {
        create_ci_workflow(project_dir)?;
    }

    // Create pre-commit hook if requested
    if args.hooks {
        create_pre_commit_hook(project_dir)?;
    }

    // Create supply-chain config if requested
    if args.supply_chain {
        create_supply_chain_config(project_dir)?;
    }

    // Create project skills if requested
    if args.skills {
        create_project_skills(project_dir)?;
    }

    Ok(())
}

// Mostly the shared crates' file contents.
#[allow(clippy::too_many_lines)]
fn create_shared_crates(project_dir: &Path) -> Result<(), CliError> {
    let crates_dir = project_dir.join("crates");

    // envelope crate
    create_crate(
        &crates_dir.join("envelope"),
        "envelope",
        r#"[package]
name = "envelope"
version.workspace = true
edition.workspace = true

[lints]
workspace = true

[dependencies]
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
chrono = { workspace = true }
utoipa = { workspace = true, features = ["macros", "chrono", "uuid", "url", "actix_extras"] }
"#,
    )?;

    std::fs::create_dir_all(crates_dir.join("envelope/src")).map_err(|source| CliError::Write {
        path: crates_dir.join("envelope/src"),
        source,
    })?;
    std::fs::write(
        crates_dir.join("envelope/src/lib.rs"),
        r#"//! Standardized API response envelopes.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Successful response envelope.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiResponse<T> {
    pub data: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ApiMeta>,
}

/// Error response envelope.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiErrorResponse {
    pub error: ApiError,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ApiMeta>,
}

/// Error details.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Response metadata.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiMeta {
    pub request_id: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination: Option<PaginationMeta>,
}

/// Pagination metadata.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PaginationMeta {
    pub page: u32,
    pub per_page: u32,
    pub total: u64,
    pub total_pages: u32,
}
"#,
    )
    .map_err(|source| CliError::Write {
        path: crates_dir.join("envelope/src/lib.rs"),
        source,
    })?;

    // db crate
    create_crate(
        &crates_dir.join("db"),
        "db",
        r#"[package]
name = "db"
version.workspace = true
edition.workspace = true

[lints]
workspace = true

[dependencies]
sqlx = { workspace = true }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
tracing = "0.1"
"#,
    )?;

    std::fs::create_dir_all(crates_dir.join("db/src")).map_err(|source| CliError::Write {
        path: crates_dir.join("db/src"),
        source,
    })?;
    std::fs::write(
        crates_dir.join("db/src/lib.rs"),
        r#"//! Database connection and utilities.

use sqlx::PgPool;

/// Creates a new Postgres connection pool.
pub fn connect() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");
    PgPool::connect_lazy(&database_url).expect("Failed to create pool")
}
"#,
    )
    .map_err(|source| CliError::Write {
        path: crates_dir.join("db/src/lib.rs"),
        source,
    })?;

    std::fs::create_dir_all(crates_dir.join("db/migrations")).map_err(|source| {
        CliError::Write {
            path: crates_dir.join("db/migrations"),
            source,
        }
    })?;

    // macros crate
    create_crate(
        &crates_dir.join("macros"),
        "macros",
        r#"[package]
name = "macros"
version.workspace = true
edition.workspace = true

[lints]
workspace = true

[dependencies]
quote = "1"
syn = { version = "2", features = ["full", "extra-traits"] }
utoipa = { workspace = true }
actix-web = { workspace = true }

[lib]
proc-macro = true
"#,
    )?;

    std::fs::create_dir_all(crates_dir.join("macros/src")).map_err(|source| CliError::Write {
        path: crates_dir.join("macros/src"),
        source,
    })?;
    std::fs::write(
        crates_dir.join("macros/src/lib.rs"),
        r#"//! Procedural macros for route registration.

use proc_macro::TokenStream;

/// Module registration attribute.
/// Usage: `#[module(prefix = "/posts", controllers = [list_posts, create_post])]`
#[proc_macro_attribute]
pub fn module(_args: TokenStream, input: TokenStream) -> TokenStream {
    input
}
"#,
    )
    .map_err(|source| CliError::Write {
        path: crates_dir.join("macros/src/lib.rs"),
        source,
    })?;

    // pagination crate
    create_crate(
        &crates_dir.join("pagination"),
        "pagination",
        r#"[package]
name = "pagination"
version.workspace = true
edition.workspace = true

[lints]
workspace = true

[dependencies]
serde = { workspace = true, features = ["derive"] }
"#,
    )?;

    std::fs::create_dir_all(crates_dir.join("pagination/src")).map_err(|source| {
        CliError::Write {
            path: crates_dir.join("pagination/src"),
            source,
        }
    })?;
    std::fs::write(
        crates_dir.join("pagination/src/lib.rs"),
        r"//! Pagination utilities.

use serde::{Deserialize, Serialize};

/// Pagination parameters.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct PaginationParams {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

impl Default for PaginationParams {
    fn default() -> Self {
        Self {
            page: Some(1),
            per_page: Some(20),
        }
    }
}

impl PaginationParams {
    pub fn offset(&self) -> u64 {
        let page = self.page.unwrap_or(1).max(1) as u64;
        let per_page = self.per_page.unwrap_or(20).max(1) as u64;
        (page - 1) * per_page
    }

    pub fn limit(&self) -> u64 {
        self.per_page.unwrap_or(20).max(1) as u64
    }
}

/// Pagination result.
#[derive(Debug, Clone, Serialize)]
pub struct Paginated<T> {
    pub items: Vec<T>,
    pub pagination: PaginationMeta,
}

#[derive(Debug, Clone, Serialize)]
pub struct PaginationMeta {
    pub page: u32,
    pub per_page: u32,
    pub total: u64,
    pub total_pages: u32,
}
",
    )
    .map_err(|source| CliError::Write {
        path: crates_dir.join("pagination/src/lib.rs"),
        source,
    })?;

    // session crate (for auth)
    create_crate(
        &crates_dir.join("session"),
        "session",
        r#"[package]
name = "session"
version.workspace = true
edition.workspace = true

[lints]
workspace = true

[dependencies]
actix-web = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }
"#,
    )?;

    std::fs::create_dir_all(crates_dir.join("session/src")).map_err(|source| CliError::Write {
        path: crates_dir.join("session/src"),
        source,
    })?;
    std::fs::write(
        crates_dir.join("session/src/lib.rs"),
        r#"//! Session-based authentication.

use actix_web::{dev::Payload, Error, FromRequest, HttpRequest, middleware::Condition};
use std::future::ready;
use uuid::Uuid;

/// Authenticated user extractor.
pub struct AuthenticatedUser {
    pub user_id: Uuid,
}

impl FromRequest for AuthenticatedUser {
    type Error = Error;
    type Future = std::future::Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        // In a real implementation, this would validate the session
        ready(Err(actix_web::error::ErrorUnauthorized("Not authenticated")))
    }
}

/// Admin user extractor.
pub struct AdminUser {
    pub user_id: Uuid,
}

impl FromRequest for AdminUser {
    type Error = Error;
    type Future = std::future::Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        // In a real implementation, this would check for admin role
        ready(Err(actix_web::error::ErrorUnauthorized("Not authorized")))
    }
}

/// Middleware to require authentication.
pub fn require_auth() -> Condition<fn(&HttpRequest) -> bool> {
    // Placeholder - returns a condition that always allows
    Condition::new(true, |_req| true)
}
"#,
    )
    .map_err(|source| CliError::Write {
        path: crates_dir.join("session/src/lib.rs"),
        source,
    })?;

    // validation crate
    create_crate(
        &crates_dir.join("validation"),
        "validation",
        r#"[package]
name = "validation"
version.workspace = true
edition.workspace = true

[lints]
workspace = true

[dependencies]
actix-web = { workspace = true }
validator = { workspace = true }
serde = { workspace = true }
"#,
    )?;

    std::fs::create_dir_all(crates_dir.join("validation/src")).map_err(|source| {
        CliError::Write {
            path: crates_dir.join("validation/src"),
            source,
        }
    })?;
    std::fs::write(
        crates_dir.join("validation/src/lib.rs"),
        r#"//! Request validation extractor.

use actix_web::{web, Error, FromRequest, HttpRequest};
use validator::Validate;

/// Validated JSON extractor.
pub struct ValidatedJson<T>(pub T)
where
    T: Validate;

impl<T> FromRequest for ValidatedJson<T>
where
    T: Validate + serde::de::DeserializeOwned,
{
    type Error = Error;
    type Future = std::future::Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, payload: &mut actix_web::dev::Payload) -> Self::Future {
        // Use actix_web::web::Json extractor and validate
        let json_result = web::Json::<T>::from_request(req, payload);
        // In actix-web 4, we need to await the future, but this is sync.
        // For now, we'll skip validation in the extractor and let handlers validate.
        // This is a simplified version - real implementation would need async FromRequest.
        std::future::ready(Err(actix_web::error::ErrorInternalServerError("ValidatedJson not implemented for sync FromRequest")))
    }
}
"#,
    ).map_err(|source| CliError::Write {
        path: crates_dir.join("validation/src/lib.rs"),
        source,
    })?;

    Ok(())
}

fn create_crate(crate_dir: &Path, _name: &str, cargo_toml: &str) -> Result<(), CliError> {
    std::fs::create_dir_all(crate_dir).map_err(|source| CliError::Write {
        path: crate_dir.to_path_buf(),
        source,
    })?;
    std::fs::write(crate_dir.join("Cargo.toml"), cargo_toml).map_err(|source| CliError::Write {
        path: crate_dir.join("Cargo.toml"),
        source,
    })?;
    Ok(())
}

fn create_router_crate(project_dir: &Path) -> Result<(), CliError> {
    let router_dir = project_dir.join("crates/router");
    create_crate(
        &router_dir,
        "router",
        r#"[package]
name = "router"
version.workspace = true
edition.workspace = true

[lints]
workspace = true

[dependencies]
actix-web = { workspace = true }
envelope = { workspace = true }
macros = { workspace = true }
utoipa = { workspace = true }
"#,
    )?;

    std::fs::create_dir_all(router_dir.join("src")).map_err(|source| CliError::Write {
        path: router_dir.join("src"),
        source,
    })?;

    std::fs::write(
        router_dir.join("src/lib.rs"),
        r"//! API router and OpenAPI documentation.

mod openapi;

pub use openapi::ApiDoc;

use macros::module;

#[module(imports = [])]
pub struct AppModule;

pub fn configure(cfg: &mut actix_web::web::ServiceConfig) {
    // Modules are registered here
}
",
    )
    .map_err(|source| CliError::Write {
        path: router_dir.join("src/lib.rs"),
        source,
    })?;

    std::fs::write(
        router_dir.join("src/openapi.rs"),
        r#"use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(),
    components(schemas()),
    tags(),
    modifiers(&SecurityAddon),
)]
pub struct ApiDoc;

mod security {
    use utoipa::Modify;
    use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};

    pub struct SecurityAddon;

    impl Modify for SecurityAddon {
        fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
            if let Some(components) = openapi.components.as_mut() {
                components.add_security_scheme(
                    "bearer_auth",
                    SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
                )
            }
        }
    }
}

use security::SecurityAddon;
"#,
    )
    .map_err(|source| CliError::Write {
        path: router_dir.join("src/openapi.rs"),
        source,
    })?;

    Ok(())
}

fn create_error_crate(project_dir: &Path) -> Result<(), CliError> {
    let error_dir = project_dir.join("crates/error");
    create_crate(
        &error_dir,
        "error",
        r#"[package]
name = "error"
version.workspace = true
edition.workspace = true

[lints]
workspace = true

[dependencies]
envelope = { workspace = true }
thiserror = "2"
actix-web = { workspace = true }
tracing = { workspace = true }
uuid = { workspace = true }
"#,
    )?;

    std::fs::create_dir_all(error_dir.join("src")).map_err(|source| CliError::Write {
        path: error_dir.join("src"),
        source,
    })?;

    std::fs::write(
        error_dir.join("src/lib.rs"),
        r#"//! Error types and conversions.

use envelope::{ApiErrorResponse, ApiError};
use thiserror::Error;

/// Application error.
#[derive(Debug, Clone, Error)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl From<AppError> for ApiErrorResponse {
    fn from(err: AppError) -> Self {
        let (code, status) = match &err {
            AppError::NotFound(_) => ("NOT_FOUND", 404),
            AppError::Unauthorized(_) => ("UNAUTHORIZED", 401),
            AppError::Forbidden(_) => ("FORBIDDEN", 403),
            AppError::BadRequest(_) => ("BAD_REQUEST", 400),
            AppError::Internal(_) => ("INTERNAL", 500),
        };
        Self {
            error: ApiError {
                code: code.to_string(),
                message: err.to_string(),
                details: None,
            },
            meta: None,
        }
    }
}

impl actix_web::ResponseError for AppError {
    fn status_code(&self) -> actix_web::http::StatusCode {
        match self {
            Self::NotFound(_) => actix_web::http::StatusCode::NOT_FOUND,
            Self::Unauthorized(_) => actix_web::http::StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => actix_web::http::StatusCode::FORBIDDEN,
            Self::BadRequest(_) => actix_web::http::StatusCode::BAD_REQUEST,
            Self::Internal(_) => actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> actix_web::HttpResponse {
        let response: ApiErrorResponse = <ApiErrorResponse as std::convert::From<AppError>>::from(self.clone());
        actix_web::HttpResponse::build(self.status_code()).json(response)
    }
}
"#,
    ).map_err(|source| CliError::Write {
        path: error_dir.join("src/lib.rs"),
        source,
    })?;

    Ok(())
}

fn create_db_crate(project_dir: &Path) -> Result<(), CliError> {
    // db crate already created in create_shared_crates
    // Just ensure migrations directory exists
    std::fs::create_dir_all(project_dir.join("crates/db/migrations")).map_err(|source| {
        CliError::Write {
            path: project_dir.join("crates/db/migrations"),
            source,
        }
    })?;
    Ok(())
}

fn init_git(project_dir: &Path) -> Result<(), CliError> {
    let output = std::process::Command::new("git")
        .args(["init"])
        .current_dir(project_dir)
        .output()
        .map_err(|err| CliError::Invalid(format!("could not run git: {err}")))?;
    if !output.status.success() {
        return Err(CliError::Invalid(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }
    Ok(())
}

fn create_ci_workflow(project_dir: &Path) -> Result<(), CliError> {
    let github_dir = project_dir.join(".github/workflows");
    std::fs::create_dir_all(&github_dir).map_err(|source| CliError::Write {
        path: github_dir.clone(),
        source,
    })?;

    std::fs::write(
        github_dir.join("ci.yml"),
        r"name: CI

on:
  push:
    branches: [main]
  pull_request:
        branches: [main]

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - name: Cache cargo
        uses: Swatinem/rust-cache@v2
      - name: Check formatting
        run: cargo fmt --check --all
      - name: Check clippy
        run: cargo clippy --workspace --all-targets -- -D warnings
      - name: Run tests
        run: cargo test --workspace
      - name: Run rok check
        run: cargo rok check --keep-going
",
    )
    .map_err(|source| CliError::Write {
        path: github_dir.join("ci.yml"),
        source,
    })?;

    Ok(())
}

fn create_pre_commit_hook(project_dir: &Path) -> Result<(), CliError> {
    let hooks_dir = project_dir.join(".git/hooks");
    std::fs::create_dir_all(&hooks_dir).map_err(|source| CliError::Write {
        path: hooks_dir.clone(),
        source,
    })?;

    let hook = r"#!/bin/sh
# Installed by `cargo rok hooks install`. Re-run it to change the gates.
exec cargo rok check fmt lint arch
";
    std::fs::write(hooks_dir.join("pre-commit"), hook).map_err(|source| CliError::Write {
        path: hooks_dir.join("pre-commit"),
        source,
    })?;

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            hooks_dir.join("pre-commit"),
            std::fs::Permissions::from_mode(0o755),
        )
        .map_err(|source| CliError::Write {
            path: hooks_dir.join("pre-commit"),
            source,
        })?;
    }

    Ok(())
}

fn create_supply_chain_config(project_dir: &Path) -> Result<(), CliError> {
    std::fs::write(
        project_dir.join("deny.toml"),
        r#"[licenses]
allow = ["MIT", "Apache-2.0", "BSD-3-Clause", "BSD-2-Clause", "ISC", "CC0-1.0", "Unlicense", "Apache-2.0 WITH LLVM-exception"]
unlicensed = "deny"

[advisories]
db = "https://raw.githubusercontent.com/rustsec/advisory-db/main/index.json"
fetch = true

[bans]
multiple-versions = "warn"
wildcards = "deny"
highlight = "all"

[sources]
unknown-registry = "deny"
unknown-git = "deny"
"#,
    ).map_err(|source| CliError::Write {
        path: project_dir.join("deny.toml"),
        source,
    })?;

    Ok(())
}

fn create_project_skills(project_dir: &Path) -> Result<(), CliError> {
    let skills_dir = project_dir.join(".opencode/skills");
    std::fs::create_dir_all(&skills_dir).map_err(|source| CliError::Write {
        path: skills_dir.clone(),
        source,
    })?;

    // Add-feature skill
    std::fs::write(
        skills_dir.join("add-feature.md"),
        r"# Skill: add-feature

Add a new feature crate to the project.

## Usage

```bash
cargo rok make:feature <name> --field <field1:type1> --field <field2:type2>
```

## Example

```bash
cargo rok make:feature posts --field title:string --field body:text --field views:int
```
",
    )
    .map_err(|source| CliError::Write {
        path: skills_dir.join("add-feature.md"),
        source,
    })?;

    // Add-endpoint skill
    std::fs::write(
        skills_dir.join("add-endpoint.md"),
        r"# Skill: add-endpoint

Add a new endpoint to an existing feature.

## Usage

```bash
cargo rok make:action <feature> <name> --method <METHOD> --path <PATH> --body <DTO>
```

## Example

```bash
cargo rok make:action posts publish_post --method post --path /{id}/publish --body PublishPostRequest
```
",
    ).map_err(|source| CliError::Write {
        path: skills_dir.join("add-endpoint.md"),
        source,
    })?;

    // Integrate-features skill
    std::fs::write(
        skills_dir.join("integrate-features.md"),
        r"# Skill: integrate-features

Integrate one feature into another.

## Usage

```bash
cargo rok integrate --preset has-many --var source=comments --var target=posts
```
",
    )
    .map_err(|source| CliError::Write {
        path: skills_dir.join("integrate-features.md"),
        source,
    })?;

    // Quality-gate skill
    std::fs::write(
        skills_dir.join("quality-gate.md"),
        r"# Skill: quality-gate

Run the project's quality gates.

## Usage

```bash
cargo rok check
```

## Pre-commit

The pre-commit hook runs `cargo rok check fmt lint arch` before each commit.
",
    )
    .map_err(|source| CliError::Write {
        path: skills_dir.join("quality-gate.md"),
        source,
    })?;

    Ok(())
}

/// `cargo rok adopt`: infers file roles and writes rok.toml for an existing project.
fn adopt(
    format: OutputFormat,
    cmd: &crate::cli::AdoptCmd,
    cwd: &Path,
    stdin: &mut dyn Read,
) -> Result<Execution, CliError> {
    let flags = cmd.flag_layer();
    let resolved: Resolved<rok_plan::make::AdoptArgs> = resolve(&cmd.input, flags, cwd, stdin)?;
    if cmd.input.print_payload {
        return Ok(output::json_value(&resolved.payload));
    }
    let args = resolved.args;

    // Check if we're in a rok-managed project (should fail if not)
    let project_result = Project::discover(cwd);
    if project_result.is_ok() {
        return Err(CliError::Invalid(
            "already a rok-managed project (rok.toml exists)".to_string(),
        ));
    }

    // Analyze the project structure
    let analysis = analyze_project(cwd)?;

    let mut outcome = Outcome::new("adopt", true);
    outcome.data = serde_json::json!({
        "discovered_features": analysis.features,
        "non_conforming_files": analysis.non_conforming,
        "suggested_rok_toml": analysis.rok_toml,
    });

    let human = format!(
        "Analyzed project at {}\n\nDiscovered features: {}\nNon-conforming files: {}\n\nSuggested rok.toml:\n{}",
        cwd.display(),
        if analysis.features.is_empty() {
            "none".to_string()
        } else {
            analysis.features.join(", ")
        },
        analysis.non_conforming.len(),
        analysis.rok_toml
    );

    if args.dry_run {
        outcome.data["dry_run"] = serde_json::json!(true);
        return Ok(output::outcome(format, &outcome, human, EXIT_OK));
    }

    // Write rok.toml
    std::fs::write(cwd.join("rok.toml"), &analysis.rok_toml).map_err(|source| CliError::Write {
        path: cwd.join("rok.toml"),
        source,
    })?;

    outcome.files_created.push("rok.toml".to_string());
    Ok(output::outcome(format, &outcome, human, EXIT_OK))
}

#[derive(Debug)]
struct ProjectAnalysis {
    features: Vec<String>,
    non_conforming: Vec<String>,
    rok_toml: String,
}

fn analyze_project(cwd: &Path) -> Result<ProjectAnalysis, CliError> {
    let mut features = Vec::new();
    let mut non_conforming = Vec::new();

    // Check for Cargo.toml
    let cargo_toml_path = cwd.join("Cargo.toml");
    let _cargo_toml =
        std::fs::read_to_string(&cargo_toml_path).map_err(|source| CliError::Write {
            path: cargo_toml_path.clone(),
            source,
        })?;

    // Look for crates/ directory
    let crates_dir = cwd.join("crates");
    if crates_dir.is_dir() {
        for entry in std::fs::read_dir(&crates_dir).map_err(|source| CliError::Write {
            path: crates_dir.clone(),
            source,
        })? {
            let entry = entry.map_err(|source| CliError::Write {
                path: crates_dir.clone(),
                source,
            })?;
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if is_feature_crate(&path) {
                    features.push(name.clone());
                } else {
                    non_conforming.push(format!("crates/{name} (not a feature crate)"));
                }
            }
        }
    } else {
        non_conforming.push("no crates/ directory found".to_string());
    }

    // Check for composition root (src/lib.rs or src/main.rs)
    let composition_root = "src";
    if !cwd.join("src/lib.rs").is_file() && !cwd.join("src/main.rs").is_file() {
        non_conforming.push("no composition root found (src/lib.rs or src/main.rs)".to_string());
    }

    // Check for router crate
    let router_path = cwd.join("crates/router/src/lib.rs");
    let router = "crates/router/src/lib.rs";
    if !router_path.is_file() {
        non_conforming.push("no router crate found at crates/router/src/lib.rs".to_string());
    }

    // Check for error crate
    let error_path = cwd.join("crates/error/src/lib.rs");
    let errors = "crates/error/src/lib.rs";
    if !error_path.is_file() {
        non_conforming.push("no error crate found at crates/error/src/lib.rs".to_string());
    }

    // Check for migrations directory
    let migrations_path = cwd.join("crates/db/migrations");
    let migrations = "crates/db/migrations";
    if !migrations_path.is_dir() {
        non_conforming.push("no migrations directory found at crates/db/migrations".to_string());
    }

    // Check feature crates for naming conventions
    for feature in &features {
        let feature_dir = crates_dir.join(feature);
        check_feature_structure(&feature_dir, feature, &mut non_conforming);
    }

    // Generate suggested rok.toml
    let cli_version = env!("CARGO_PKG_VERSION");
    let rok_toml = format!(
        r#"[rok]
cli = "^{cli_version}"
templates = "0.1.0"

[project]
composition_root = "{composition_root}"
composition_fn = "run"
router = "{router}"
errors = "{errors}"
migrations = "{migrations}"

[architecture]
allow = []
"#
    );

    Ok(ProjectAnalysis {
        features,
        non_conforming,
        rok_toml,
    })
}

fn is_feature_crate(path: &Path) -> bool {
    // A feature crate should have a Cargo.toml and src/*_module.rs
    let cargo_toml = path.join("Cargo.toml");
    if !cargo_toml.is_file() {
        return false;
    }
    let src_dir = path.join("src");
    if !src_dir.is_dir() {
        return false;
    }
    // Check for module file
    if let Ok(entries) = std::fs::read_dir(&src_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with("_module.rs") {
                return true;
            }
        }
    }
    false
}

fn check_feature_structure(
    feature_dir: &Path,
    feature_name: &str,
    non_conforming: &mut Vec<String>,
) {
    let src_dir = feature_dir.join("src");
    let expected_files = vec![
        format!("{}_action.rs", feature_name),
        format!("{}_entity.rs", feature_name),
        format!("{}_module.rs", feature_name),
        format!("{}_repository.rs", feature_name),
        format!("{}_service.rs", feature_name),
    ];

    for expected in &expected_files {
        if !src_dir.join(expected).is_file() {
            non_conforming.push(format!("crates/{feature_name}/src/{expected} missing"));
        }
    }

    // Check for dto/ directory
    let dto_dir = src_dir.join("dto");
    if dto_dir.is_dir() {
        // Check for create and update DTOs
        if !dto_dir.join(format!("create_{feature_name}.rs")).is_file() {
            non_conforming.push(format!(
                "crates/{feature_name}/src/dto/create_{feature_name}.rs missing"
            ));
        }
        if !dto_dir.join(format!("update_{feature_name}.rs")).is_file() {
            non_conforming.push(format!(
                "crates/{feature_name}/src/dto/update_{feature_name}.rs missing"
            ));
        }
    } else {
        non_conforming.push(format!("crates/{feature_name}/src/dto/ directory missing"));
    }

    // Check for migration
    let Some(crates_dir) = feature_dir.parent() else {
        return;
    };
    let migrations_dir = crates_dir.join("db/migrations");
    if migrations_dir.is_dir() {
        let has_migration = std::fs::read_dir(&migrations_dir).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|e| e.file_name().to_string_lossy().contains(feature_name))
        });
        if !has_migration {
            non_conforming.push(format!("no migration found for feature {feature_name}"));
        }
    }
}
