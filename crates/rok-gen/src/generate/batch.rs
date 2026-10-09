//! `cargo rok apply`: a list of commands run as one batch.
//!
//! Every generator and integration step edits one transaction, so a later
//! step sees the files earlier ones created and nothing is written unless
//! every step applies. `check` steps run once the batch is written; a
//! failing gate restores every file, so the batch lands whole or not at all.

use std::fmt::Write as _;

use rok_check::gates::Report;
use rok_edit::Transaction;
use rok_plan::apply::{ApplyArgs, BATCH_COMMANDS, BatchStep};
use rok_plan::check::CheckArgs;
use rok_plan::integrate::IntegrateArgs;
use rok_plan::outcome::Outcome;
use rok_plan::project::Project;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::integrate::{Permit, integrate_into};
use super::{GenError, Generated, journal, make_into};

/// `cargo rok apply`. `permit` comes from the command line only, as for
/// `integrate`. The outcome is not `ok` when a `check` step failed (the
/// batch was rolled back then).
///
/// # Errors
///
/// [`GenError`] for a step that can't be applied; nothing is written.
pub fn apply(project: &Project, args: &ApplyArgs, permit: Permit) -> Result<Generated, GenError> {
    if args.steps.is_empty() {
        return Err(GenError::Invalid("the batch has no `steps`".to_string()));
    }
    let permit = Permit {
        allow_dirty: permit.allow_dirty || args.dry_run,
        ..permit
    };
    let mut tx = Transaction::new(&project.root);
    let mut checks = Vec::new();
    let mut warnings = Vec::new();
    for (index, step) in args.steps.iter().enumerate() {
        run_step(project, &mut tx, step, permit, &mut checks, &mut warnings).map_err(|err| {
            GenError::Invalid(format!("step {} ({}): {err}", index + 1, step.command))
        })?;
    }
    tx.format()?;
    warnings.extend(tx.warnings().iter().cloned());

    let mut outcome = Outcome::new("apply", true);
    outcome.files_created = tx.created();
    outcome.files_changed = tx.changed();
    let mut human = String::new();
    if args.dry_run {
        let diff = tx.diff();
        if !checks.is_empty() {
            warnings.push("`check` steps don't run in a dry run".to_string());
        }
        outcome.data = json!({ "dry_run": true, "diff": diff });
        human.push_str(&diff);
        human.push_str("dry run: nothing was written\n");
        return Ok(finish(outcome, human, warnings));
    }

    let committed = if args.verify {
        tx.commit_verified()?
    } else {
        tx.commit()?
    };
    let mut reports = Vec::new();
    // The batch may have changed rok.toml (e.g. an allowlisted edge).
    let reloaded = if checks.is_empty() {
        None
    } else {
        match Project::discover(&project.root) {
            Ok(reloaded) => Some(reloaded),
            Err(err) => {
                rok_edit::rollback(&committed);
                return Err(GenError::Invalid(format!(
                    "the batch left rok.toml unreadable, so it was rolled back: {err}"
                )));
            }
        }
    };
    let checked = reloaded.as_ref().unwrap_or(project);
    for check in &checks {
        let report = rok_check::gates::run(checked, check);
        let ok = report.ok();
        reports.push(report);
        if !ok {
            rok_edit::rollback(&committed);
            return Ok(rolled_back(outcome, &reports, warnings));
        }
    }

    let label = args
        .name
        .clone()
        .or_else(|| Some(format!("batch of {} steps", args.steps.len())));
    let has_changes = !tx.changes().is_empty();
    let transaction = if has_changes {
        journal::record(&tx, "apply", label).map(Some)
    } else {
        Ok(None)
    };
    if let Err(warning) = &transaction {
        warnings.push(warning.clone());
    }
    let id = transaction.ok().flatten();
    outcome.data = json!({
        "dry_run": false,
        "verified": args.verify,
        "transaction": id,
        "checks": reports,
    });
    for file in &outcome.files_created {
        let _ = writeln!(human, "created  {file}");
    }
    for file in &outcome.files_changed {
        let _ = writeln!(human, "changed  {file}");
    }
    for report in &reports {
        human.push_str(&crate::output::check_report(report));
    }
    if let Some(id) = id {
        let _ = writeln!(human, "transaction {id} (undo with `cargo rok undo {id}`)");
    }
    Ok(finish(outcome, human, warnings))
}

fn run_step(
    project: &Project,
    tx: &mut Transaction,
    step: &BatchStep,
    permit: Permit,
    checks: &mut Vec<CheckArgs>,
    warnings: &mut Vec<String>,
) -> Result<(), GenError> {
    if ["dry_run", "verify"].iter().any(|key| {
        step.args
            .get(key)
            .is_some_and(|value| value != &Value::Bool(false))
    }) {
        return Err(GenError::Invalid(
            "set `dry_run` and `verify` on the batch, not on its steps".to_string(),
        ));
    }
    match step.command.as_str() {
        "check" => checks.push(payload(&step.args)?),
        "integrate" => {
            let plan: IntegrateArgs = payload(&step.args)?;
            if plan.emit_plan.is_some() {
                return Err(GenError::Invalid(
                    "`emit_plan` isn't available in a batch".to_string(),
                ));
            }
            warnings.extend(integrate_into(tx, project, &plan, permit)?.warnings);
        }
        command if command.starts_with("make:") && BATCH_COMMANDS.contains(&command) => {
            let args = if step.args.is_null() {
                json!({})
            } else {
                step.args.clone()
            };
            make_into(tx, project, command, &args)?;
        }
        other => {
            return Err(GenError::Invalid(format!(
                "`{other}` can't run in a batch; use one of {}",
                BATCH_COMMANDS.join(", ")
            )));
        }
    }
    Ok(())
}

fn payload<T: DeserializeOwned + Default>(args: &Value) -> Result<T, GenError> {
    if args.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(args.clone())
        .map_err(|err| GenError::Invalid(format!("arguments: {err}")))
}

/// A gate failed after the batch was written; every file was restored.
fn rolled_back(mut outcome: Outcome, reports: &[Report], warnings: Vec<String>) -> Generated {
    outcome.ok = false;
    outcome.data = json!({ "dry_run": false, "rolled_back": true, "checks": reports });
    let mut human = String::new();
    for report in reports {
        human.push_str(&crate::output::check_report(report));
    }
    human.push_str("a check failed, so the batch was rolled back: nothing was changed\n");
    finish(outcome, human, warnings)
}

fn finish(mut outcome: Outcome, mut human: String, warnings: Vec<String>) -> Generated {
    for warning in &warnings {
        let _ = writeln!(human, "warning: {warning}");
    }
    outcome.warnings = warnings;
    Generated { outcome, human }
}
