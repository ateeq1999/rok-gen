//! The second seam the spec allows: statement fingerprints stay findable
//! through reformatting, line shifts and moves, and edits made afterwards
//! are reported as drift instead of being touched.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rok_journal::{Change, FileRecord, Journal, Revert, plan_revert};

const BEFORE: &str = "\
pub struct Service {
    posts: Posts,
}

impl Service {
    pub fn delete(&self, id: u32) {
        self.posts.delete(id);
    }
}
";

const AFTER: &str = "\
pub struct Service {
    posts: Posts,
    comments: Comments,
}

impl Service {
    pub fn delete(&self, id: u32) {
        self.posts.delete(id);
        self.comments.delete_by_post(id);
    }
}
";

fn record(before: &str, after: &str) -> Journal {
    let mut journal = Journal::default();
    journal
        .record(
            "integrate",
            Some("comments into posts".to_string()),
            &[Change {
                file: "src/service.rs".to_string(),
                before: Some(before.to_string()),
                after: Some(after.to_string()),
            }],
            1,
        )
        .unwrap();
    journal
}

fn revert(journal: &Journal, current: &str) -> Result<String, Vec<String>> {
    let tx = journal.last_active().unwrap();
    let current = current.to_string();
    plan_revert(tx, |_| Some(current.clone())).map(|mut actions| match actions.remove(0).1 {
        Revert::Write(text) => text,
        Revert::Delete => panic!("unexpected delete"),
    })
}

#[test]
fn reverting_right_away_restores_the_original_exactly() {
    let journal = record(BEFORE, AFTER);
    assert_eq!(revert(&journal, AFTER).unwrap(), BEFORE);
}

#[test]
fn reformatting_and_lines_added_above_are_tolerated() {
    let journal = record(BEFORE, AFTER);
    let reformatted = format!(
        "// a new comment\nuse std::fmt;\n\n{}",
        AFTER.replace(
            "        self.comments.delete_by_post(id);",
            "        self.comments\n            .delete_by_post( id );"
        )
    );
    let reverted = revert(&journal, &reformatted).unwrap();
    assert!(!reverted.contains("comments"), "{reverted}");
    assert!(
        reverted.starts_with("// a new comment\nuse std::fmt;\n"),
        "{reverted}"
    );
}

#[test]
fn a_moved_function_is_still_found() {
    let journal = record(BEFORE, AFTER);
    let moved = format!(
        "{}\npub fn helper() {{}}\n",
        AFTER.replacen("impl Service {", "pub fn other() {}\n\nimpl Service {", 1)
    );
    let reverted = revert(&journal, &moved).unwrap();
    assert!(!reverted.contains("comments"), "{reverted}");
    assert!(reverted.contains("pub fn other() {}") && reverted.contains("pub fn helper() {}"));
}

#[test]
fn edited_inserted_code_is_drift_and_nothing_changes() {
    let journal = record(BEFORE, AFTER);
    let edited = AFTER.replace("delete_by_post(id)", "delete_by_post(id + 1)");
    let drift = revert(&journal, &edited).unwrap_err();
    assert_eq!(drift.len(), 1, "{drift:?}");
    assert!(drift[0].contains("no longer there"), "{drift:?}");
}

#[test]
fn duplicated_inserted_code_with_identical_context_is_ambiguous() {
    let journal = record(BEFORE, AFTER);
    let doubled = format!("{AFTER}\n{}", AFTER.replace("Service", "Service2"));
    // The statement's context (`delete(id);` ... `}}`) is the same in both
    // copies: refuse to guess.
    let drift = revert(&journal, &doubled).unwrap_err();
    assert!(
        drift.iter().any(|reason| reason.contains("more than once")),
        "{drift:?}"
    );
}

#[test]
fn replacements_restore_the_original_text() {
    let after = BEFORE.replace(
        "self.posts.delete(id);",
        "self.posts.soft_delete(id, true);",
    );
    let journal = record(BEFORE, &after);
    assert_eq!(revert(&journal, &after).unwrap(), BEFORE);
}

#[test]
fn created_files_are_deleted_only_while_unchanged() {
    let mut journal = Journal::default();
    journal
        .record(
            "make:dto",
            None,
            &[Change {
                file: "src/dto.rs".to_string(),
                before: None,
                after: Some("pub struct A;\n".to_string()),
            }],
            1,
        )
        .unwrap();
    let tx = journal.last_active().unwrap();
    assert!(matches!(tx.files[0], FileRecord::Created { .. }));
    let actions = plan_revert(tx, |_| Some("pub struct A;\n".to_string())).unwrap();
    assert_eq!(actions[0].1, Revert::Delete);
    let drift = plan_revert(tx, |_| Some("pub struct B;\n".to_string())).unwrap_err();
    assert!(
        drift[0].contains("edited after it was created"),
        "{drift:?}"
    );
}

#[test]
fn non_rust_files_revert_by_line() {
    let before = "[dependencies]\nauth.workspace = true\nusers.workspace = true\n";
    let after = "[dependencies]\nauth.workspace = true\ncomments.workspace = true\nusers.workspace = true\n";
    let mut journal = Journal::default();
    journal
        .record(
            "integrate",
            None,
            &[Change {
                file: "Cargo.toml".to_string(),
                before: Some(before.to_string()),
                after: Some(after.to_string()),
            }],
            1,
        )
        .unwrap();
    let tx = journal.last_active().unwrap();
    let shifted = format!("# a comment above\n{after}");
    let actions = plan_revert(tx, |_| Some(shifted.clone())).unwrap();
    assert_eq!(
        actions[0].1,
        Revert::Write(format!("# a comment above\n{before}"))
    );
}

#[test]
fn ids_are_unique_and_undone_transactions_are_skipped() {
    let mut journal = record(BEFORE, AFTER);
    let second = journal.record("integrate", None, &[], 1).unwrap();
    assert_eq!(second, "1-integrate-2");
    journal.mark_undone(&second);
    assert_eq!(journal.last_active().unwrap().id, "1-integrate");

    let dir = tempfile::tempdir().unwrap();
    journal.save(dir.path()).unwrap();
    assert_eq!(Journal::load(dir.path()).unwrap(), journal);
}

#[test]
fn comments_next_to_an_insertion_survive_its_revert() {
    let before = "\
fn run() {
    let a = A::new();

    // Explains b; must survive.
    let b = B::new();
}
";
    let after = "\
fn run() {
    let a = A::new();

    let c = C::new();
    let d = D::new(c);

    // Explains b; must survive.
    let b = B::new();
}
";
    let journal = record(before, after);
    assert_eq!(revert(&journal, after).unwrap(), before);
}
