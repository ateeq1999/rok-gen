//! Running the quality gates in order.
//!
//! External gates shell out to cargo subcommands. A gate whose tool isn't
//! installed is reported as skipped (with how to install it) rather than
//! failed, so a fresh machine still gets a useful run; CI should install
//! every tool so nothing is skipped there.

use std::path::Path;
use std::process::Command;

use rok_plan::check::{CheckArgs, Gate};
use rok_plan::project::Project;
use serde::{Deserialize, Serialize};

use crate::arch;
use crate::finding::Finding;

/// Lines of tool output kept in a failed gate's detail.
const OUTPUT_TAIL_LINES: usize = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GateStatus {
    Passed,
    Failed,
    Skipped,
}

/// The result of one gate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateResult {
    pub gate: Gate,
    pub status: GateStatus,
    /// The command that ran, for external gates.
    pub command: Option<String>,
    /// Why it was skipped, or the tail of a failing tool's output.
    pub detail: Option<String>,
    /// Architecture findings (the `arch` gate only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<Finding>,
}

/// Every gate that ran, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub gates: Vec<GateResult>,
}

impl Report {
    /// No gate failed (skipped gates don't count as failures).
    #[must_use]
    pub fn ok(&self) -> bool {
        self.gates
            .iter()
            .all(|result| result.status != GateStatus::Failed)
    }
}

/// Runs the selected gates against `project`, stopping at the first failure
/// unless `args.keep_going`.
#[must_use]
pub fn run(project: &Project, args: &CheckArgs) -> Report {
    let mut report = Report::default();
    for gate in args.selected() {
        let result = run_gate(project, gate);
        let failed = result.status == GateStatus::Failed;
        report.gates.push(result);
        if failed && !args.keep_going {
            break;
        }
    }
    report
}

fn run_gate(project: &Project, gate: Gate) -> GateResult {
    let root = project.root.as_path();
    match gate {
        Gate::Fmt => external(root, gate, &["fmt", "--all", "--", "--check"]),
        Gate::Lint => external(
            root,
            gate,
            // No `-D warnings`: the project's own `[workspace.lints]` levels
            // decide what fails. A lint a project deliberately left at
            // `warn` (e.g. clippy::pedantic) stays a warning.
            &["clippy", "--workspace", "--all-targets", "--quiet"],
        ),
        Gate::Test => {
            if has_subcommand(root, "nextest") {
                external(root, gate, &["nextest", "run", "--workspace"])
            } else {
                external(root, gate, &["test", "--workspace", "--quiet"])
            }
        }
        Gate::Deny => optional(
            root,
            gate,
            "deny",
            &["deny", "check"],
            "cargo install cargo-deny",
        ),
        Gate::Machete => optional(
            root,
            gate,
            "machete",
            &["machete"],
            "cargo install cargo-machete",
        ),
        Gate::Sqlx => {
            if !root.join(".sqlx").is_dir() {
                return skipped(
                    gate,
                    "no `.sqlx` offline query data in the project; run `cargo sqlx prepare \
                     --workspace` and commit it to enable this gate",
                );
            }
            optional(
                root,
                gate,
                "sqlx",
                &["sqlx", "prepare", "--workspace", "--check"],
                "cargo install sqlx-cli --no-default-features --features rustls,postgres",
            )
        }
        Gate::Openapi | Gate::Coverage => skipped(
            gate,
            "this gate isn't implemented yet in this version of rok-gen",
        ),
        Gate::Arch => match arch::check(project) {
            Ok(findings) => GateResult {
                gate,
                status: if findings.is_empty() {
                    GateStatus::Passed
                } else {
                    GateStatus::Failed
                },
                command: None,
                detail: None,
                findings,
            },
            Err(err) => GateResult {
                gate,
                status: GateStatus::Failed,
                command: None,
                detail: Some(err.to_string()),
                findings: Vec::new(),
            },
        },
    }
}

fn optional(root: &Path, gate: Gate, subcommand: &str, args: &[&str], install: &str) -> GateResult {
    if has_subcommand(root, subcommand) {
        external(root, gate, args)
    } else {
        skipped(
            gate,
            &format!("`cargo {subcommand}` isn't installed; install it with `{install}`"),
        )
    }
}

fn external(root: &Path, gate: Gate, args: &[&str]) -> GateResult {
    let command = format!("cargo {}", args.join(" "));
    let output = Command::new("cargo").args(args).current_dir(root).output();
    let (status, detail) = match output {
        Ok(output) if output.status.success() => (GateStatus::Passed, None),
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            (GateStatus::Failed, Some(tail(&text)))
        }
        Err(err) => (
            GateStatus::Failed,
            Some(format!("could not run `{command}`: {err}")),
        ),
    };
    GateResult {
        gate,
        status,
        command: Some(command),
        detail,
        findings: Vec::new(),
    }
}

fn skipped(gate: Gate, reason: &str) -> GateResult {
    GateResult {
        gate,
        status: GateStatus::Skipped,
        command: None,
        detail: Some(reason.to_string()),
        findings: Vec::new(),
    }
}

fn has_subcommand(root: &Path, subcommand: &str) -> bool {
    Command::new("cargo")
        .args([subcommand, "--version"])
        .current_dir(root)
        .output()
        .is_ok_and(|output| output.status.success())
}

fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.trim_end().lines().collect();
    let start = lines.len().saturating_sub(OUTPUT_TAIL_LINES);
    lines[start..].join("\n")
}
