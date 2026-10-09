//! `undo`, `unlink`, `journal`, and `--force` safety.

// Test harness code: panicking on a broken fixture is the point.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::BTreeMap;
use std::process::Command;

use common::Fixture;
use serde_json::json;

/// Every file under the fixture except the journal.
fn snapshot(fixture: &Fixture) -> BTreeMap<String, String> {
    walk(fixture.root(), fixture.root())
}

fn walk(root: &std::path::Path, dir: &std::path::Path) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel == ".rok" || rel == ".git" {
            continue;
        }
        if path.is_dir() {
            files.extend(walk(root, &path));
        } else {
            files.insert(rel, std::fs::read_to_string(&path).unwrap_or_default());
        }
    }
    files
}

fn ok(fixture: &Fixture, args: &[&str]) -> serde_json::Value {
    let (code, outcome) = fixture.run_json(args);
    assert_eq!(code, 0, "{args:?}: {outcome:#}");
    outcome
}

fn wire_plan() -> String {
    json!({
        "steps": [
            { "op": "inject_dependency", "into": "articles/service", "service": "ArticleService",
              "dep": "tags::TagRepository", "as": "tags" },
            { "op": "insert",
              "at": { "file": "articles/service", "fn": "ArticleService::delete", "within": [{ "if": 0 }], "start": true },
              "code": "drop(self.tags.find_by_id(id).await);" },
        ],
    })
    .to_string()
}

#[test]
fn undo_restores_the_project_exactly() {
    let fixture = Fixture::app();
    let original = snapshot(&fixture);

    let outcome = ok(
        &fixture,
        &["make:feature", "articles", "--field", "title:string"],
    );
    let id = outcome["data"]["transaction"].as_str().unwrap().to_string();
    assert!(fixture.exists("crates/articles/src/lib.rs"));

    let journal = ok(&fixture, &["journal"]);
    assert_eq!(journal["data"]["transactions"][0]["id"], id);
    assert_eq!(journal["data"]["transactions"][0]["undone"], false);

    let undone = ok(&fixture, &["undo"]);
    assert_eq!(undone["data"]["transaction"], id);
    assert_eq!(snapshot(&fixture), original);
    assert!(
        !fixture.exists("crates/articles"),
        "emptied directories are removed"
    );

    let journal = ok(&fixture, &["journal"]);
    assert_eq!(journal["data"]["transactions"][0]["undone"], true);
    let execution = fixture.run(&["undo"]);
    assert_eq!(execution.code, 2);
    assert!(
        execution.stderr.contains("nothing to undo"),
        "{}",
        execution.stderr
    );
}

#[test]
fn unlink_reverts_an_integration_and_leaves_the_features() {
    let fixture = Fixture::app();
    ok(
        &fixture,
        &["make:feature", "articles", "--field", "title:string"],
    );
    ok(
        &fixture,
        &["make:feature", "tags", "--field", "label:string"],
    );
    let before_integration = snapshot(&fixture);

    let outcome = ok(&fixture, &["integrate", "-j", &wire_plan()]);
    assert!(
        fixture
            .read("crates/articles/src/article_service.rs")
            .contains("tags: Arc<dyn TagRepository>")
    );
    let journal = ok(&fixture, &["journal"]);
    assert_eq!(
        journal["data"]["transactions"][2]["label"],
        "tags into articles"
    );
    assert!(outcome["data"]["transaction"].is_string());

    ok(&fixture, &["unlink", "tags", "articles"]);
    assert_eq!(snapshot(&fixture), before_integration);

    let execution = fixture.run(&["unlink", "tags", "articles"]);
    assert_eq!(execution.code, 2);
    assert!(
        execution
            .stderr
            .contains("no active integration `tags into articles`"),
        "{}",
        execution.stderr
    );
}

#[test]
fn transactions_undo_out_of_order() {
    let fixture = Fixture::app();
    let original = snapshot(&fixture);
    let first = ok(
        &fixture,
        &["make:feature", "articles", "--field", "title:string"],
    )["data"]["transaction"]
        .as_str()
        .unwrap()
        .to_string();
    ok(
        &fixture,
        &["make:feature", "tags", "--field", "label:string"],
    );
    ok(&fixture, &["make:migration", "add_index"]);

    ok(&fixture, &["undo", &first]);
    assert!(!fixture.exists("crates/articles"));
    assert!(fixture.exists("crates/tags"));
    ok(&fixture, &["undo"]);
    ok(&fixture, &["undo"]);
    assert_eq!(snapshot(&fixture), original);
}

#[test]
fn edited_code_is_drift_and_nothing_is_changed() {
    let fixture = Fixture::app();
    ok(
        &fixture,
        &["make:feature", "articles", "--field", "title:string"],
    );
    ok(
        &fixture,
        &["make:feature", "tags", "--field", "label:string"],
    );
    ok(&fixture, &["integrate", "-j", &wire_plan()]);
    fixture.replace(
        "crates/articles/src/article_service.rs",
        "drop(self.tags.find_by_id(id).await);",
        "drop(self.tags.find_by_id(id).await.ok());",
    );
    let edited = snapshot(&fixture);

    let execution = fixture.run(&["unlink", "tags", "articles"]);
    assert_eq!(execution.code, 2);
    assert!(
        execution
            .stderr
            .contains("were edited since (nothing was changed)"),
        "{}",
        execution.stderr
    );
    assert!(
        execution.stderr.contains("article_service.rs"),
        "{}",
        execution.stderr
    );
    assert_eq!(snapshot(&fixture), edited);
}

#[test]
fn undo_dry_run_shows_the_diff_and_keeps_the_journal() {
    let fixture = Fixture::app();
    ok(
        &fixture,
        &[
            "make:migration",
            "add_index",
            "--sql",
            "CREATE INDEX x ON posts (id);",
        ],
    );
    let before = snapshot(&fixture);
    let outcome = ok(&fixture, &["undo", "--dry-run"]);
    let diff = outcome["data"]["diff"].as_str().unwrap();
    assert!(
        diff.contains("--- a/crates/db/migrations/0002_add_index.sql\n+++ /dev/null"),
        "{diff}"
    );
    assert_eq!(snapshot(&fixture), before);
    assert_eq!(
        ok(&fixture, &["journal"])["data"]["transactions"][0]["undone"],
        false
    );
}

#[test]
fn dry_runs_are_not_recorded() {
    let fixture = Fixture::app();
    ok(
        &fixture,
        &[
            "make:feature",
            "articles",
            "--field",
            "title:string",
            "--dry-run",
        ],
    );
    let journal = ok(&fixture, &["journal"]);
    assert_eq!(journal["data"]["transactions"], json!([]));
    assert!(
        fixture
            .run(&["journal"])
            .stdout
            .contains("the journal is empty")
    );
}

fn git(fixture: &Fixture, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(fixture.root())
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&status.stderr)
    );
}

#[test]
fn force_refuses_files_with_uncommitted_changes() {
    let fixture = Fixture::app();
    let replace = json!({ "steps": [{
        "op": "replace",
        "at": { "file": "@root", "fn": "run", "stmt": { "let": "pool" } },
        "code": "let pool = db::connect_lazy();",
    }]})
    .to_string();

    // Outside git there's nothing to recover from.
    let execution = fixture.run(&["integrate", "-j", &replace, "--force"]);
    if execution.code == 2 && execution.stderr.contains("not a git repository") {
        assert!(execution.stderr.contains("--allow-dirty"));
    }

    git(&fixture, &["init", "-q"]);
    git(
        &fixture,
        &["-c", "user.name=t", "-c", "user.email=t@t", "add", "-A"],
    );
    git(
        &fixture,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "init",
        ],
    );
    fixture.replace(
        "src/lib.rs",
        "//! Composition root.",
        "//! Composition root (edited).",
    );

    let execution = fixture.run(&["integrate", "-j", &replace, "--force"]);
    assert_eq!(execution.code, 2);
    assert!(
        execution.stderr.contains("uncommitted changes"),
        "{}",
        execution.stderr
    );
    assert!(
        execution.stderr.contains("src/lib.rs"),
        "{}",
        execution.stderr
    );

    let (code, outcome) =
        fixture.run_json(&["integrate", "-j", &replace, "--force", "--allow-dirty"]);
    assert_eq!(code, 0, "{outcome:#}");
    assert!(fixture.read("src/lib.rs").contains("db::connect_lazy()"));

    // And the replace can be undone, restoring the original statement.
    ok(&fixture, &["undo"]);
    assert!(
        fixture
            .read("src/lib.rs")
            .contains("let pool = db::connect();")
    );
}

#[test]
fn a_plan_name_becomes_the_unlink_label() {
    let fixture = Fixture::app();
    ok(
        &fixture,
        &["make:feature", "articles", "--field", "title:string"],
    );
    let plan = json!({
        "name": "audit into articles",
        "steps": [{ "op": "insert",
                    "at": { "file": "articles/service", "fn": "ArticleService::delete", "start": true },
                    "code": "let _audit = id;" }],
    })
    .to_string();
    ok(&fixture, &["integrate", "-j", &plan]);
    ok(&fixture, &["unlink", "audit", "articles"]);
    assert!(
        !fixture
            .read("crates/articles/src/article_service.rs")
            .contains("_audit")
    );
}
