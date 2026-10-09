//! Project discovery, version pinning, `schema`, and the top-level CLI.

// Test harness code: panicking on a broken fixture is the point.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Fixture;
use serde_json::Value;

#[test]
fn outside_a_rok_project_the_error_says_so() {
    let fixture = Fixture::empty();
    let execution = fixture.run(&["check", "arch"]);
    assert_eq!(execution.code, 2);
    assert!(
        execution
            .stderr
            .contains("not a rok-managed project: no rok.toml"),
        "{}",
        execution.stderr
    );
}

#[test]
fn the_project_is_found_from_a_subdirectory() {
    let fixture = Fixture::clean();
    let execution = fixture.run_in("crates/posts/src", &["check", "arch"]);
    assert_eq!(
        execution.code, 0,
        "{}{}",
        execution.stdout, execution.stderr
    );
}

#[test]
fn an_incompatible_cli_pin_is_refused() {
    let fixture = Fixture::clean();
    fixture.replace("rok.toml", "cli = \"^0.1\"", "cli = \"^9\"");
    let execution = fixture.run(&["check", "arch"]);
    assert_eq!(execution.code, 2);
    assert!(
        execution.stderr.contains("requires rok-gen ^9")
            && execution
                .stderr
                .contains("cargo install rok-gen --version '^9'"),
        "{}",
        execution.stderr
    );
}

#[test]
fn an_invalid_config_is_reported_with_its_path() {
    let fixture = Fixture::clean();
    fixture.replace("rok.toml", "[project]", "[projekt]");
    let execution = fixture.run(&["check", "arch"]);
    assert_eq!(execution.code, 2);
    assert!(
        execution.stderr.contains("invalid") && execution.stderr.contains("rok.toml"),
        "{}",
        execution.stderr
    );

    let fixture = Fixture::clean();
    fixture.replace(
        "rok.toml",
        "[project]",
        "[architecture]\nallow = [\"posts\"]\n\n[project]",
    );
    let execution = fixture.run(&["check", "arch"]);
    assert_eq!(execution.code, 1);
    assert!(
        execution.stdout.contains("expected `<from> -> <to>`"),
        "{}",
        execution.stdout
    );
}

#[test]
fn schemas_are_published_for_every_payload_and_the_config() {
    let fixture = Fixture::empty();
    for name in ["check", "hooks-install", "project"] {
        let execution = fixture.run(&["schema", name]);
        assert_eq!(execution.code, 0, "{name}: {}", execution.stderr);
        let schema: Value = serde_json::from_str(&execution.stdout).expect("JSON");
        assert!(schema["properties"].is_object(), "{name}: {schema:#}");
    }

    let check: Value = serde_json::from_str(&fixture.run(&["schema", "check"]).stdout).unwrap();
    assert!(check["properties"]["gates"].is_object());

    let execution = fixture.run(&["schema", "nope"]);
    assert_eq!(execution.code, 2);
    for name in rok_plan::schema::SCHEMA_NAMES {
        assert!(
            execution.stderr.contains(name),
            "{name}: {}",
            execution.stderr
        );
        assert_eq!(fixture.run(&["schema", name]).code, 0, "{name}");
    }
}

#[test]
fn help_and_version_succeed_and_bad_arguments_are_usage_errors() {
    let fixture = Fixture::empty();
    let help = fixture.run(&["--help"]);
    assert_eq!(help.code, 0);
    assert!(help.stdout.contains("cargo rok"), "{}", help.stdout);

    let version = fixture.run(&["--version"]);
    assert_eq!(version.code, 0);
    assert!(version.stdout.contains(env!("CARGO_PKG_VERSION")));

    let bad = fixture.run(&["nonsense"]);
    assert_eq!(bad.code, 2);
    assert!(bad.stderr.contains("nonsense"), "{}", bad.stderr);
}
