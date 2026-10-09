//! `cargo rok hooks install`.

// Test harness code: panicking on a broken fixture is the point.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::process::Command;

use common::Fixture;

fn git_fixture() -> Fixture {
    let fixture = Fixture::clean();
    let status = Command::new("git")
        .args(["init", "-q"])
        .current_dir(fixture.root())
        .status()
        .expect("git is installed");
    assert!(status.success());
    fixture
}

#[test]
fn installs_a_pre_commit_hook_running_the_fast_gates() {
    let fixture = git_fixture();
    let (code, outcome) = fixture.run_json(&["hooks", "install"]);
    assert_eq!(code, 0, "{outcome:#}");
    assert_eq!(outcome["files_created"][0], ".git/hooks/pre-commit");

    let hook = fixture.read(".git/hooks/pre-commit");
    assert!(hook.starts_with("#!/bin/sh\n"), "{hook}");
    assert!(
        hook.ends_with("exec cargo rok check fmt lint arch\n"),
        "{hook}"
    );
}

#[test]
fn reinstalling_its_own_hook_updates_it() {
    let fixture = git_fixture();
    assert_eq!(fixture.run(&["hooks", "install"]).code, 0);
    let (code, outcome) = fixture.run_json(&["hooks", "install", "arch"]);
    assert_eq!(code, 0, "{outcome:#}");
    assert_eq!(outcome["files_changed"][0], ".git/hooks/pre-commit");
    assert!(
        fixture
            .read(".git/hooks/pre-commit")
            .ends_with("exec cargo rok check arch\n")
    );
}

#[test]
fn a_foreign_hook_is_kept_unless_overwrite_is_passed() {
    let fixture = git_fixture();
    fixture.write(".git/hooks/pre-commit", "#!/bin/sh\necho mine\n");

    let execution = fixture.run(&["hooks", "install"]);
    assert_eq!(execution.code, 2);
    assert!(
        execution.stderr.contains("--overwrite"),
        "{}",
        execution.stderr
    );
    assert_eq!(
        fixture.read(".git/hooks/pre-commit"),
        "#!/bin/sh\necho mine\n"
    );

    assert_eq!(fixture.run(&["hooks", "install", "--overwrite"]).code, 0);
    assert!(
        fixture
            .read(".git/hooks/pre-commit")
            .contains("cargo rok check")
    );
}

#[test]
fn outside_a_git_repository_the_error_says_so() {
    let fixture = Fixture::clean();
    let execution = fixture.run(&["hooks", "install"]);
    // A temp dir could sit inside some other repository; only assert when
    // it doesn't.
    if execution.code == 2 {
        assert!(
            execution.stderr.contains("not a git repository"),
            "{}",
            execution.stderr
        );
    }
}
