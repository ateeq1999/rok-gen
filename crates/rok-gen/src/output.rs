//! Rendering results for people (`--output human`) and machines
//! (`--output json`, GitHub annotations).

use std::fmt::Write;

use rok_check::{GateStatus, Report};
use rok_plan::outcome::Outcome;

use crate::cli::OutputFormat;
use crate::{CliError, EXIT_OK, EXIT_USAGE, Execution};

pub fn outcome(format: OutputFormat, outcome: &Outcome, human: String, code: i32) -> Execution {
    let stdout = match format {
        OutputFormat::Human => human,
        OutputFormat::Json => to_json(outcome),
    };
    Execution {
        code,
        stdout,
        stderr: String::new(),
    }
}

/// A value that is itself the result (a schema, a merged payload): printed
/// as JSON in both formats.
pub fn json_value(value: &serde_json::Value) -> Execution {
    Execution {
        code: EXIT_OK,
        stdout: to_json(value),
        stderr: String::new(),
    }
}

pub fn error(format: OutputFormat, command: &str, err: &CliError) -> Execution {
    match format {
        OutputFormat::Human => Execution {
            code: EXIT_USAGE,
            stdout: String::new(),
            stderr: format!("error: {err}\n"),
        },
        OutputFormat::Json => {
            let mut outcome = Outcome::new(command, false);
            outcome.data = serde_json::json!({ "error": err.to_string() });
            Execution {
                code: EXIT_USAGE,
                stdout: to_json(&outcome),
                stderr: String::new(),
            }
        }
    }
}

pub fn check_report(report: &Report) -> String {
    let mut text = String::new();
    let (mut passed, mut failed, mut skipped) = (0, 0, 0);
    for result in &report.gates {
        let label = match result.status {
            GateStatus::Passed => {
                passed += 1;
                "passed "
            }
            GateStatus::Failed => {
                failed += 1;
                "FAILED "
            }
            GateStatus::Skipped => {
                skipped += 1;
                "skipped"
            }
        };
        let _ = write!(text, "{label}  {}", result.gate);
        if let Some(command) = &result.command {
            let _ = write!(text, "  ({command})");
        }
        text.push('\n');
        for finding in &result.findings {
            let _ = writeln!(text, "    {finding}");
        }
        if let Some(detail) = &result.detail {
            for line in detail.lines() {
                let _ = writeln!(text, "    {line}");
            }
        }
    }
    let _ = writeln!(
        text,
        "check: {passed} passed, {failed} failed, {skipped} skipped"
    );
    text
}

/// GitHub Actions workflow commands for every finding, failed gate and
/// skipped gate.
pub fn annotations(report: &Report) -> String {
    let mut text = String::new();
    for result in &report.gates {
        for finding in &result.findings {
            let _ = writeln!(
                text,
                "::error file={},line={},col={},title=rok {}::{}",
                escape_property(&finding.file),
                finding.line,
                finding.column,
                finding.rule,
                escape_data(&finding.message)
            );
        }
        let detail = result.detail.as_deref().unwrap_or_default();
        match result.status {
            GateStatus::Failed if result.findings.is_empty() => {
                let _ = writeln!(
                    text,
                    "::error title=rok {} failed::{}",
                    result.gate,
                    escape_data(detail)
                );
            }
            GateStatus::Skipped => {
                let _ = writeln!(
                    text,
                    "::warning title=rok {} skipped::{}",
                    result.gate,
                    escape_data(detail)
                );
            }
            _ => {}
        }
    }
    text
}

fn escape_data(text: &str) -> String {
    text.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn escape_property(text: &str) -> String {
    escape_data(text).replace(':', "%3A").replace(',', "%2C")
}

fn to_json<T: serde::Serialize>(value: &T) -> String {
    let mut text = serde_json::to_string_pretty(value)
        .unwrap_or_else(|err| format!("{{\"error\": \"could not serialize output: {err}\"}}"));
    text.push('\n');
    text
}
