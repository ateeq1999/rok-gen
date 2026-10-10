//! Recording committed transactions, and `undo` / `unlink` / `journal`.

use std::fmt::Write as _;
use std::path::Path;

use rok_edit::Transaction;
use rok_journal::{Change, Journal, Revert, TxRecord, plan_revert};
use rok_plan::outcome::Outcome;
use rok_plan::project::Project;
use serde_json::json;

use super::{GenError, Generated};

/// Records `tx` (already committed). Returns the transaction id, or a
/// warning when the journal couldn't be written (the change itself stands).
pub fn record(tx: &Transaction, command: &str, label: Option<String>) -> Result<String, String> {
    let changes: Vec<Change> = tx
        .changes()
        .into_iter()
        .map(|change| Change {
            file: change.rel,
            before: change.before,
            after: change.after,
        })
        .collect();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let result = Journal::load(tx.root()).and_then(|mut journal| {
        let id = journal.record(command, label, &changes, now)?;
        journal.save(tx.root())?;
        Ok(id)
    });
    result.map_err(|err| {
        format!("the change was written, but not recorded in the journal ({err}); it can't be undone with `cargo rok undo`")
    })
}

/// `cargo rok undo [id]`: reverts one transaction (default: the latest
/// that's still active).
///
/// # Errors
///
/// [`GenError::Invalid`] when there's nothing to undo, or when any region
/// drifted (nothing is changed then).
pub fn undo(project: &Project, id: Option<&str>, dry_run: bool) -> Result<Generated, GenError> {
    let mut journal = load(&project.root)?;
    let record = match id {
        Some(id) => journal
            .find(id)
            .cloned()
            .ok_or_else(|| GenError::Invalid(format!("no transaction `{id}` in the journal")))?,
        None => journal
            .last_active()
            .cloned()
            .ok_or_else(|| GenError::Invalid("nothing to undo".to_string()))?,
    };
    if record.undone {
        return Err(GenError::Invalid(format!(
            "`{}` was already undone",
            record.id
        )));
    }
    let generated = revert(project, &record, dry_run, "undo")?;
    if !dry_run {
        journal.mark_undone(&record.id);
        save(&journal, &project.root)?;
    }
    Ok(generated)
}

/// `cargo rok unlink <source> <target>`: undoes every active integration
/// labeled `<source> into <target>`, newest first.
///
/// # Errors
///
/// [`GenError::Invalid`] when there's no such integration or a region
/// drifted.
pub fn unlink(
    project: &Project,
    source: &str,
    target: &str,
    dry_run: bool,
) -> Result<Generated, GenError> {
    let label = format!("{source} into {target}");
    let mut journal = load(&project.root)?;
    let records: Vec<TxRecord> = journal
        .transactions
        .iter()
        .rev()
        .filter(|record| !record.undone && record.label.as_deref() == Some(label.as_str()))
        .cloned()
        .collect();
    if records.is_empty() {
        return Err(GenError::Invalid(format!(
            "no active integration `{label}` in the journal (see `cargo rok journal`)"
        )));
    }
    if dry_run && records.len() > 1 {
        return Err(GenError::Invalid(format!(
            "`{label}` spans {} transactions; preview them one at a time with `cargo rok undo <id> --dry-run`",
            records.len()
        )));
    }
    let mut combined = Outcome::new("unlink", true);
    let mut human = String::new();
    for record in &records {
        let generated = revert(project, record, dry_run, "unlink")?;
        combined
            .files_changed
            .extend(generated.outcome.files_changed);
        combined
            .files_created
            .extend(generated.outcome.files_created);
        combined.warnings.extend(generated.outcome.warnings);
        human.push_str(&generated.human);
        if !dry_run {
            journal.mark_undone(&record.id);
            save(&journal, &project.root)?;
        }
    }
    combined.data = json!({
        "dry_run": dry_run,
        "transactions": records.iter().map(|record| record.id.clone()).collect::<Vec<_>>(),
    });
    Ok(Generated {
        outcome: combined,
        human,
    })
}

/// `cargo rok journal`: every recorded transaction.
///
/// # Errors
///
/// [`GenError::Invalid`] when the journal can't be read.
pub fn list(project: &Project) -> Result<Generated, GenError> {
    let journal = load(&project.root)?;
    let mut human = String::new();
    for record in &journal.transactions {
        let state = if record.undone { "undone" } else { "active" };
        let label = record
            .label
            .as_deref()
            .map(|label| format!("  ({label})"))
            .unwrap_or_default();
        let _ = writeln!(
            human,
            "{}  {state:6}  {}{label}  {} files",
            record.id,
            record.command,
            record.files.len()
        );
    }
    if journal.transactions.is_empty() {
        human.push_str("the journal is empty\n");
    }
    let mut outcome = Outcome::new("journal", true);
    outcome.data = serde_json::to_value(&journal).unwrap_or_default();
    Ok(Generated { outcome, human })
}

/// Reverts one recorded transaction as a new (formatted, unrecorded)
/// transaction.
fn revert(
    project: &Project,
    record: &TxRecord,
    dry_run: bool,
    command: &str,
) -> Result<Generated, GenError> {
    let read = |file: &str| {
        std::fs::read_to_string(project.root.join(file))
            .ok()
            .map(|text| text.replace("\r\n", "\n"))
    };
    let actions = plan_revert(record, read).map_err(|drift| {
        GenError::Invalid(format!(
            "can't undo `{}`: some of its changes were edited since (nothing was changed):\n  {}",
            record.id,
            drift.join("\n  ")
        ))
    })?;

    let mut tx = Transaction::new(&project.root);
    for (file, action) in &actions {
        match action {
            Revert::Write(text) => tx.update(file, text.clone())?,
            Revert::Delete => tx.delete(file)?,
        }
    }
    tx.format()?;
    let mut outcome = Outcome::new(command, true);
    outcome.files_changed = tx.changed();
    outcome.warnings = tx.warnings().to_vec();
    let deleted = tx.deleted();
    let mut human = String::new();
    if dry_run {
        let diff = tx.diff();
        human.push_str(&diff);
        human.push_str("dry run: nothing was written\n");
        outcome.data = json!({ "dry_run": true, "diff": diff, "transaction": record.id });
    } else {
        tx.commit()?;
        remove_empty_dirs(&project.root, &deleted);
        outcome.data = json!({ "dry_run": false, "transaction": record.id, "deleted": deleted });
        for file in &outcome.files_changed {
            let _ = writeln!(human, "reverted {file}");
        }
        for file in &deleted {
            let _ = writeln!(human, "deleted  {file}");
        }
        let _ = writeln!(human, "undid {}", record.id);
    }
    Ok(Generated { outcome, human })
}

/// Removes directories left empty by deleted files, up to the root.
pub(super) fn remove_empty_dirs(root: &Path, deleted: &[String]) {
    for file in deleted {
        let mut dir = root.join(file).parent().map(Path::to_path_buf);
        while let Some(current) = dir {
            if current == root || std::fs::remove_dir(&current).is_err() {
                break;
            }
            dir = current.parent().map(Path::to_path_buf);
        }
    }
}

fn load(root: &Path) -> Result<Journal, GenError> {
    Journal::load(root).map_err(|err| GenError::Invalid(err.to_string()))
}

fn save(journal: &Journal, root: &Path) -> Result<(), GenError> {
    journal
        .save(root)
        .map_err(|err| GenError::Invalid(err.to_string()))
}
