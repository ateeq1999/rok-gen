//! Formatting, line endings, diffs and rollback.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rok_edit::{EditError, Transaction};

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("rustfmt.toml"), "max_width = 100\n").unwrap();
    dir
}

#[test]
fn new_rust_files_are_always_formatted() {
    let dir = project();
    let mut tx = Transaction::new(dir.path());
    tx.create("src/a.rs", "pub fn a()->u32{1}").unwrap();
    tx.format().unwrap();
    tx.commit().unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/a.rs")).unwrap(),
        "pub fn a() -> u32 {\n    1\n}\n"
    );
    assert_eq!(tx.warnings().len(), 0, "{:?}", tx.warnings());
}

#[test]
fn invalid_rust_is_refused_before_writing() {
    let dir = project();
    let mut tx = Transaction::new(dir.path());
    tx.create("src/a.rs", "pub fn a( {").unwrap();
    assert!(matches!(tx.format(), Err(EditError::Format { .. })));
}

#[test]
fn crlf_files_stay_crlf() {
    let dir = project();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn a() {}\r\n").unwrap();
    let mut tx = Transaction::new(dir.path());
    let text = tx.read("src/lib.rs").unwrap();
    assert_eq!(text, "pub fn a() {}\n");
    tx.update("src/lib.rs", format!("{text}pub fn b() {{}}\n"))
        .unwrap();
    tx.format().unwrap();
    tx.commit().unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
        "pub fn a() {}\r\npub fn b() {}\r\n"
    );
}

#[test]
fn previously_unformatted_files_are_reported() {
    let dir = project();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn a()->u8{1}\n").unwrap();
    let mut tx = Transaction::new(dir.path());
    let text = tx.read("src/lib.rs").unwrap();
    tx.update("src/lib.rs", format!("{text}pub fn b() {{}}\n"))
        .unwrap();
    tx.format().unwrap();
    assert_eq!(tx.warnings().len(), 1);
    assert!(tx.warnings()[0].contains("wasn't rustfmt-clean"));
}

#[test]
fn diff_shows_new_and_changed_files_and_writes_nothing() {
    let dir = project();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn a() {}\n").unwrap();
    let mut tx = Transaction::new(dir.path());
    tx.update("src/lib.rs", "pub fn a() {}\npub fn b() {}\n")
        .unwrap();
    tx.create("src/new.rs", "pub fn n() {}\n").unwrap();
    let diff = tx.diff();
    assert!(
        diff.contains("--- a/src/lib.rs\n+++ b/src/lib.rs"),
        "{diff}"
    );
    assert!(diff.contains("+pub fn b() {}"), "{diff}");
    assert!(diff.contains("--- /dev/null\n+++ b/src/new.rs"), "{diff}");
    assert_eq!(tx.created(), ["src/new.rs"]);
    assert_eq!(tx.changed(), ["src/lib.rs"]);
    assert!(!dir.path().join("src/new.rs").exists());
}

#[test]
fn creating_an_existing_file_is_refused() {
    let dir = project();
    let mut tx = Transaction::new(dir.path());
    assert!(matches!(
        tx.create("Cargo.toml", ""),
        Err(EditError::Exists { .. })
    ));
}

#[test]
fn rollback_restores_files_and_removes_created_dirs() {
    let dir = project();
    let original = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
    let mut tx = Transaction::new(dir.path());
    tx.update("Cargo.toml", "garbage").unwrap();
    tx.create("crates/x/src/lib.rs", "pub fn x() {}\n").unwrap();
    let committed = tx.commit().unwrap();
    assert!(dir.path().join("crates/x/src/lib.rs").exists());

    rok_edit::rollback(&committed);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap(),
        original
    );
    assert!(!dir.path().join("crates").exists());
}

#[test]
fn a_failed_verify_rolls_everything_back() {
    let dir = project();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn a() {}\n").unwrap();
    let mut tx = Transaction::new(dir.path());
    tx.update("src/lib.rs", "pub fn a() -> u8 { \"not a u8\" }\n")
        .unwrap();
    let err = tx.commit_verified().unwrap_err();
    assert!(matches!(err, EditError::Verify { .. }), "{err}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
        "pub fn a() {}\n"
    );
}
