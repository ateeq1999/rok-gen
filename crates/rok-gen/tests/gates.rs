//! Gate selection and execution (real cargo, on fixtures).

// Test harness code: panicking on a broken fixture is the point.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::process::Command;

use common::Fixture;
use serde_json::Value;

/// A tiny, dependency-free cargo package with a `rok.toml`.
fn cargo_fixture() -> Fixture {
    let fixture = Fixture::empty();
    fixture.write("rok.toml", "[rok]\ncli = \"^0.1\"\ntemplates = \"0.1.0\"\n");
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"tiny\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\n",
    );
    fixture.write("src/lib.rs", "pub fn answer() -> u32 {\n    42\n}\n");
    fixture
}

fn statuses(outcome: &Value) -> Vec<(String, String)> {
    outcome["data"]["gates"]
        .as_array()
        .expect("gates")
        .iter()
        .map(|gate| {
            (
                gate["gate"].as_str().unwrap().to_string(),
                gate["status"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn pair(gate: &str, status: &str) -> (String, String) {
    (gate.to_string(), status.to_string())
}

#[test]
fn fmt_passes_on_formatted_code_and_fails_on_unformatted() {
    let fixture = cargo_fixture();
    let (code, outcome) = fixture.run_json(&["check", "fmt"]);
    assert_eq!(code, 0, "{outcome:#}");
    assert_eq!(statuses(&outcome), vec![pair("fmt", "passed")]);

    fixture.write("src/lib.rs", "pub fn answer()->u32{42}\n");
    let (code, outcome) = fixture.run_json(&["check", "fmt"]);
    assert_eq!(code, 1);
    assert_eq!(statuses(&outcome), vec![pair("fmt", "failed")]);
    assert!(
        outcome["data"]["gates"][0]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("Diff in")),
        "{outcome:#}"
    );
}

#[test]
fn gates_run_in_canonical_order_and_stop_at_the_first_failure() {
    let fixture = cargo_fixture();
    fixture.write("src/lib.rs", "pub fn answer()->u32{42}\n");

    // Named out of order; fmt still runs first, fails, and arch never runs.
    let (code, outcome) = fixture.run_json(&["check", "arch", "fmt"]);
    assert_eq!(code, 1);
    assert_eq!(statuses(&outcome), vec![pair("fmt", "failed")]);

    let (_, outcome) = fixture.run_json(&["check", "arch", "fmt", "--keep-going"]);
    assert_eq!(
        statuses(&outcome),
        vec![pair("fmt", "failed"), pair("arch", "passed")]
    );
}

#[test]
fn unimplemented_and_unconfigured_gates_are_skipped_with_a_reason() {
    let fixture = cargo_fixture();
    let (code, outcome) = fixture.run_json(&["check", "openapi", "coverage", "sqlx"]);
    assert_eq!(code, 0, "skipped gates don't fail the run: {outcome:#}");
    assert_eq!(
        statuses(&outcome),
        vec![
            pair("sqlx", "skipped"),
            pair("openapi", "skipped"),
            pair("coverage", "skipped"),
        ]
    );
    let warnings = outcome["warnings"].as_array().expect("warnings");
    assert_eq!(warnings.len(), 3);
    assert!(warnings[0].as_str().unwrap().contains(".sqlx"));
}

#[test]
fn missing_tools_are_skipped_with_install_instructions() {
    let fixture = cargo_fixture();
    let installed = Command::new("cargo")
        .args(["deny", "--version"])
        .output()
        .is_ok_and(|output| output.status.success());
    if installed {
        return;
    }
    let (_, outcome) = fixture.run_json(&["check", "deny"]);
    assert_eq!(statuses(&outcome), vec![pair("deny", "skipped")]);
    assert!(
        outcome["data"]["gates"][0]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("cargo install cargo-deny")),
        "{outcome:#}"
    );
}

#[test]
fn human_output_summarizes_every_gate() {
    let fixture = cargo_fixture();
    let execution = fixture.run(&["check", "fmt", "openapi"]);
    assert_eq!(execution.code, 0, "{}", execution.stderr);
    assert!(
        execution
            .stdout
            .contains("passed   fmt  (cargo fmt --all -- --check)"),
        "{}",
        execution.stdout
    );
    assert!(
        execution.stdout.contains("skipped  openapi"),
        "{}",
        execution.stdout
    );
    assert!(
        execution
            .stdout
            .ends_with("check: 1 passed, 0 failed, 1 skipped\n"),
        "{}",
        execution.stdout
    );
}
