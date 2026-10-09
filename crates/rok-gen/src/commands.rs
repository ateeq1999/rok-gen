//! Executing a parsed command: resolve the payload, find the project, run.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use rok_plan::check::{CheckArgs, Gate};
use rok_plan::hooks::HooksInstallArgs;
use rok_plan::input::{self, Layers, Resolved};
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

pub fn execute(cli: Cli, cwd: &Path, stdin: &mut dyn Read) -> Execution {
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
    };
    result.unwrap_or_else(|err| output::error(format, name, &err))
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
