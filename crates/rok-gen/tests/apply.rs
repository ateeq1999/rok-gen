//! `cargo rok apply`: a list of commands as one batch that lands whole or
//! not at all.

// Test harness code: panicking on a broken fixture is the point.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{Fixture, generated};
use serde_json::{Value, json};

fn apply(fixture: &Fixture, batch: &Value, extra: &[&str]) -> (i32, Value) {
    fixture.write("setup.json", &batch.to_string());
    let mut args = vec!["apply", "-f", "setup.json"];
    args.extend_from_slice(extra);
    fixture.run_json(&args)
}

fn error_of(outcome: &Value) -> String {
    outcome["data"]["error"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn blog() -> Value {
    json!({
        "name": "blog",
        "steps": [
            { "command": "make:feature",
              "args": { "name": "articles", "fields": [{ "name": "title", "type": "string" }] } },
            { "command": "make:feature",
              "args": { "name": "comments", "fields": [
                  { "name": "body", "type": "text" },
                  { "name": "article_id", "type": "uuid" },
              ] } },
            { "command": "integrate",
              "args": { "preset": "has-many", "vars": { "source": "comments", "target": "articles" } } },
            { "command": "check", "args": { "gates": ["arch"] } },
        ],
    })
}

#[test]
fn one_batch_builds_features_and_integrates_them_as_one_transaction() {
    let fixture = generated(&[]);
    let manifest = fixture.read("Cargo.toml");
    let root = fixture.read("src/lib.rs");

    let (code, outcome) = apply(&fixture, &blog(), &[]);
    assert_eq!(code, 0, "{outcome:#}");
    assert_eq!(outcome["command"], "apply");
    assert_eq!(outcome["data"]["checks"][0]["gates"][0]["gate"], "arch");

    // The preset saw the entity the same batch generated.
    let service = fixture.read("crates/articles/src/article_service.rs");
    assert!(service.contains("delete_by_article(id)"), "{service}");
    assert!(fixture.exists("crates/comments/src/lib.rs"));

    let journal = fixture.run_json(&["journal"]).1;
    let transactions = journal["data"]["transactions"].as_array().unwrap();
    assert_eq!(transactions.len(), 1, "{journal:#}");
    assert_eq!(transactions[0]["label"], "blog");
    assert_eq!(transactions[0]["command"], "apply");

    let (code, outcome) = fixture.run_json(&["undo"]);
    assert_eq!(code, 0, "{outcome:#}");
    assert_eq!(fixture.read("Cargo.toml"), manifest);
    assert_eq!(fixture.read("src/lib.rs"), root);
    assert!(!fixture.exists("crates/articles/src/lib.rs"));
    assert!(!fixture.exists("crates/comments/src/lib.rs"));
}

#[test]
fn a_failing_check_rolls_the_whole_batch_back() {
    let fixture = generated(&[&["make:feature", "articles", "--field", "title:string"]]);
    // A violation the batch's `check` step will find.
    fixture.replace(
        "crates/articles/src/article_service.rs",
        "use std::sync::Arc;",
        "use std::sync::Arc;\n\nuse sqlx::PgPool;",
    );
    let manifest = fixture.read("Cargo.toml");
    let journal = fixture.read(".rok/journal.json");

    let batch = json!({ "steps": [
        { "command": "make:feature",
          "args": { "name": "tags", "fields": [{ "name": "label", "type": "string" }] } },
        { "command": "check", "args": { "gates": ["arch"] } },
    ] });
    let (code, outcome) = apply(&fixture, &batch, &[]);
    assert_eq!(code, 1, "{outcome:#}");
    assert_eq!(outcome["ok"], false);
    assert_eq!(outcome["data"]["rolled_back"], true);
    assert_eq!(
        outcome["data"]["checks"][0]["gates"][0]["findings"][0]["rule"], "service-purity",
        "{outcome:#}"
    );
    assert!(!fixture.exists("crates/tags/src/lib.rs"));
    assert!(!fixture.path("crates/tags").exists());
    assert_eq!(fixture.read("Cargo.toml"), manifest);
    assert_eq!(fixture.read(".rok/journal.json"), journal);

    let execution = fixture.run(&["apply", "-f", "setup.json"]);
    assert!(
        execution
            .stdout
            .contains("a check failed, so the batch was rolled back"),
        "{}",
        execution.stdout
    );
}

#[test]
fn a_step_that_fails_writes_nothing_and_is_named() {
    let fixture = generated(&[]);
    let manifest = fixture.read("Cargo.toml");
    let batch = json!({ "steps": [
        { "command": "make:feature",
          "args": { "name": "tags", "fields": [{ "name": "label", "type": "string" }] } },
        { "command": "integrate", "args": { "steps": [
            { "op": "insert", "at": { "file": "tags/service", "fn": "TagService::nope", "start": true },
              "code": "let _ = 1;" },
        ] } },
    ] });
    let (code, outcome) = apply(&fixture, &batch, &[]);
    assert_ne!(code, 0, "{outcome:#}");
    assert!(
        error_of(&outcome).starts_with("step 2 (integrate): step 1 (insert)"),
        "{outcome:#}"
    );
    assert!(!fixture.path("crates/tags").exists());
    assert_eq!(fixture.read("Cargo.toml"), manifest);
}

#[test]
fn a_dry_run_shows_the_whole_diff_and_skips_checks() {
    let fixture = generated(&[]);
    let (code, outcome) = apply(&fixture, &blog(), &["--dry-run"]);
    assert_eq!(code, 0, "{outcome:#}");
    let diff = outcome["data"]["diff"].as_str().unwrap();
    assert!(diff.contains("crates/comments/src/lib.rs"), "{diff}");
    assert!(diff.contains("delete_by_article"), "{diff}");
    assert!(
        outcome["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning == "`check` steps don't run in a dry run"),
        "{outcome:#}"
    );
    assert!(!fixture.path("crates/articles").exists());
    assert!(!fixture.exists(".rok/journal.json"));
}

#[test]
fn batches_refuse_what_they_cannot_run() {
    let fixture = generated(&[]);
    let cases = [
        (
            json!({ "steps": [] }),
            "the batch has no `steps`".to_string(),
        ),
        (
            json!({ "steps": [{ "command": "undo" }] }),
            "step 1 (undo): `undo` can't run in a batch; use one of make:feature".to_string(),
        ),
        (
            json!({ "steps": [{ "command": "make:migration", "args": { "name": "x", "dry_run": true } }] }),
            "step 1 (make:migration): set `dry_run` and `verify` on the batch".to_string(),
        ),
        (
            json!({ "steps": [{ "command": "integrate", "args": { "preset": "hook", "emit_plan": "p.json" } }] }),
            "step 1 (integrate): `emit_plan` isn't available in a batch".to_string(),
        ),
        (
            json!({ "steps": [{ "command": "integrate", "args": { "steps": [
                { "op": "replace", "at": { "file": "@root", "fn": "run", "start": true }, "code": "x();" },
            ] } }] }),
            "step 1 (integrate): the plan replaces code; pass --force".to_string(),
        ),
        (
            json!({ "steps": [{ "command": "check", "args": { "gates": ["nope"] } }] }),
            "step 1 (check): arguments:".to_string(),
        ),
    ];
    for (batch, expected) in cases {
        let (code, outcome) = apply(&fixture, &batch, &[]);
        assert_ne!(code, 0, "{batch}: {outcome:#}");
        assert!(
            error_of(&outcome).starts_with(&expected),
            "{batch}: expected `{expected}`, got {outcome:#}"
        );
    }
    assert!(!fixture.exists(".rok/journal.json"));
}

#[test]
fn print_payload_and_schema_cover_apply() {
    let fixture = generated(&[]);
    fixture.write("setup.json", &blog().to_string());
    let execution = fixture.run(&["apply", "-f", "setup.json", "--dry-run", "--print-payload"]);
    assert_eq!(execution.code, 0, "{}", execution.stderr);
    let payload: Value = serde_json::from_str(&execution.stdout).unwrap();
    assert_eq!(payload["dry_run"], true);
    assert_eq!(payload["steps"].as_array().unwrap().len(), 4);

    let execution = fixture.run(&["schema", "apply"]);
    let schema: Value = serde_json::from_str(&execution.stdout).unwrap();
    assert!(schema["properties"]["steps"].is_object(), "{schema:#}");
}

#[test]
fn later_generators_build_on_what_earlier_ones_staged() {
    let fixture = generated(&[]);
    let batch = json!({ "steps": [
        { "command": "make:feature",
          "args": { "name": "articles", "fields": [{ "name": "title", "type": "string" }] } },
        { "command": "make:feature",
          "args": { "name": "tags", "fields": [{ "name": "label", "type": "string" }] } },
        { "command": "make:action",
          "args": { "feature": "articles", "name": "publish_article", "method": "post", "path": "/{id}/publish" } },
        { "command": "make:dto",
          "args": { "feature": "tags", "name": "RenameTag", "fields": [{ "name": "label", "type": "string" }] } },
    ] });
    let (code, outcome) = apply(&fixture, &batch, &[]);
    assert_eq!(code, 0, "{outcome:#}");

    let manifest = fixture.read("Cargo.toml");
    for line in [
        "articles = { path = \"crates/articles\" }",
        "tags = { path = \"crates/tags\" }",
    ] {
        assert!(manifest.contains(line), "{line}\n{manifest}");
    }
    assert!(
        fixture
            .read("crates/articles/src/article_action.rs")
            .contains("pub async fn publish_article(")
    );
    assert!(fixture.exists("crates/tags/src/dto/rename_tag.rs"));

    // A feature the batch already made can't be made again.
    let again = json!({ "steps": [
        { "command": "make:feature",
          "args": { "name": "notes", "fields": [{ "name": "body", "type": "text" }] } },
        { "command": "make:feature",
          "args": { "name": "notes", "fields": [{ "name": "body", "type": "text" }] } },
    ] });
    let (code, outcome) = apply(&fixture, &again, &[]);
    assert_ne!(code, 0);
    assert!(
        error_of(&outcome).contains("`crates/notes` already exists"),
        "{outcome:#}"
    );
}
