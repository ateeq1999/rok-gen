//! Layered input (defaults < -f < -j < flags), file formats, errors with
//! JSON pointers, format versions, and `--print-payload`.

// Test harness code: panicking on a broken fixture is the point.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Fixture;
use serde_json::{Value, json};

fn payload(fixture: &Fixture, args: &[&str]) -> Value {
    let mut full = args.to_vec();
    full.push("--print-payload");
    let execution = fixture.run(&full);
    assert_eq!(
        execution.code, 0,
        "{}{}",
        execution.stdout, execution.stderr
    );
    serde_json::from_str(&execution.stdout).expect("payload is JSON")
}

#[test]
fn defaults_apply_when_nothing_is_passed() {
    let fixture = Fixture::clean();
    assert_eq!(
        payload(&fixture, &["check"]),
        json!({ "gates": [], "keep_going": false, "ci": false })
    );
}

#[test]
fn precedence_is_defaults_then_file_then_inline_then_flags() {
    let fixture = Fixture::clean();
    fixture.write(
        "check.json",
        r#"{ "gates": ["fmt"], "keep_going": true, "ci": true }"#,
    );

    // File alone.
    assert_eq!(
        payload(&fixture, &["check", "-f", "check.json"]),
        json!({ "gates": ["fmt"], "keep_going": true, "ci": true })
    );
    // Inline beats file.
    assert_eq!(
        payload(
            &fixture,
            &[
                "check",
                "-f",
                "check.json",
                "-j",
                r#"{"gates":["lint"],"ci":false}"#
            ]
        ),
        json!({ "gates": ["lint"], "keep_going": true, "ci": false })
    );
    // Flags beat both.
    assert_eq!(
        payload(
            &fixture,
            &[
                "check",
                "arch",
                "-f",
                "check.json",
                "-j",
                r#"{"gates":["lint"],"ci":false}"#,
                "--ci"
            ]
        ),
        json!({ "gates": ["arch"], "keep_going": true, "ci": true })
    );
}

#[test]
fn jsonc_yaml_and_toml_files_are_equivalent() {
    let fixture = Fixture::clean();
    fixture.write(
        "a.jsonc",
        "{\n  // only the fast gates\n  \"gates\": [\"fmt\", \"arch\"],\n  \"keep_going\": true\n}\n",
    );
    fixture.write("a.yaml", "gates: [fmt, arch]\nkeep_going: true\n");
    fixture.write("a.toml", "gates = [\"fmt\", \"arch\"]\nkeep_going = true\n");

    let expected = json!({ "gates": ["fmt", "arch"], "keep_going": true, "ci": false });
    for file in ["a.jsonc", "a.yaml", "a.toml"] {
        assert_eq!(
            payload(&fixture, &["check", "-f", file]),
            expected,
            "{file}"
        );
    }
}

#[test]
fn stdin_is_read_with_dash() {
    let fixture = Fixture::clean();
    let execution = fixture.run_with_stdin(
        &["check", "-f", "-", "--print-payload"],
        r#"{"gates":["arch"]}"#,
    );
    assert_eq!(execution.code, 0, "{}", execution.stderr);
    let value: Value = serde_json::from_str(&execution.stdout).expect("JSON");
    assert_eq!(value["gates"], json!(["arch"]));
}

#[test]
fn a_misspelled_key_names_its_pointer_and_suggests_the_right_one() {
    let fixture = Fixture::clean();
    let execution = fixture.run(&["check", "-j", r#"{"gaets":["arch"]}"#]);
    assert_eq!(execution.code, 2);
    assert!(
        execution.stderr.contains("invalid payload at /gaets")
            && execution.stderr.contains("did you mean `gates`?"),
        "{}",
        execution.stderr
    );
}

#[test]
fn an_unknown_gate_names_its_array_index() {
    let fixture = Fixture::clean();
    let (code, outcome) = fixture.run_json(&["check", "arch", "nope"]);
    assert_eq!(code, 2);
    let error = outcome["data"]["error"].as_str().expect("error");
    assert!(
        error.contains("/gates/1") && error.contains("nope"),
        "{error}"
    );
}

#[test]
fn malformed_and_non_object_payloads_are_rejected() {
    let fixture = Fixture::clean();
    let execution = fixture.run(&["check", "-j", "{not json"]);
    assert_eq!(execution.code, 2);
    assert!(
        execution.stderr.contains("could not parse --json"),
        "{}",
        execution.stderr
    );

    let execution = fixture.run(&["check", "-j", "[1, 2]"]);
    assert_eq!(execution.code, 2);
    assert!(
        execution.stderr.contains("must be an object"),
        "{}",
        execution.stderr
    );

    let execution = fixture.run(&["check", "-f", "missing.json"]);
    assert_eq!(execution.code, 2);
    assert!(
        execution.stderr.contains("could not read missing.json"),
        "{}",
        execution.stderr
    );
}

#[test]
fn a_newer_payload_version_is_refused_and_the_current_one_accepted() {
    let fixture = Fixture::clean();
    let execution = fixture.run(&["check", "-j", r#"{"version": 99}"#]);
    assert_eq!(execution.code, 2);
    assert!(
        execution
            .stderr
            .contains("newer than this rok-gen supports"),
        "{}",
        execution.stderr
    );

    assert_eq!(
        payload(
            &fixture,
            &["check", "-j", r#"{"version": 1, "gates": ["arch"]}"#]
        )["gates"],
        json!(["arch"])
    );
}

#[test]
fn json_output_reports_usage_errors_as_an_outcome() {
    let fixture = Fixture::empty();
    let (code, outcome) = fixture.run_json(&["check", "arch"]);
    assert_eq!(code, 2);
    assert_eq!(outcome["ok"], false);
    assert_eq!(outcome["command"], "check");
    assert_eq!(outcome["version"], 1);
}
