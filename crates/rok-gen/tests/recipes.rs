//! Recipes: `add`, `remove` and `list`.

// Test harness code: panicking on a broken fixture is the point.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::BTreeMap;

use common::Fixture;
use serde_json::Value;

const SECRETS: &str = "DATABASE_URL=postgres://real\nRESEND_API_KEY=re_live_secret\n";

/// An app fixture with a real `.env`, which recipes must never touch.
fn app() -> Fixture {
    let fixture = Fixture::app();
    fixture.write(".env", SECRETS);
    fixture
}

fn ok(fixture: &Fixture, args: &[&str]) -> Value {
    let (code, outcome) = fixture.run_json(args);
    assert_eq!(code, 0, "{args:?}: {outcome:#}");
    outcome
}

fn refused(fixture: &Fixture, args: &[&str]) -> String {
    let execution = fixture.run(args);
    assert_eq!(execution.code, 2, "{args:?}: {}", execution.stdout);
    execution.stderr
}

/// Every file except the journal and `Cargo.toml`.
fn snapshot(fixture: &Fixture) -> BTreeMap<String, String> {
    walkdir::WalkDir::new(fixture.root())
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            let rel = entry
                .path()
                .strip_prefix(fixture.root())
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            (
                rel,
                std::fs::read_to_string(entry.path()).unwrap_or_default(),
            )
        })
        .filter(|(rel, _)| !rel.starts_with(".rok/") && rel != "Cargo.toml")
        .collect()
}

#[test]
fn add_mail_with_resend_creates_the_crate_and_declares_it() {
    let fixture = app();
    let outcome = ok(&fixture, &["add", "mail", "--driver", "resend"]);

    assert_eq!(outcome["data"]["installed"][0]["driver"], "resend");
    assert!(
        fixture
            .read("crates/mail/src/lib.rs")
            .contains("pub struct ResendMailer")
    );
    assert!(
        !fixture
            .read("crates/mail/src/lib.rs")
            .contains("SmtpMailer")
    );
    assert!(
        fixture
            .read("crates/mail/Cargo.toml")
            .contains("reqwest.workspace = true")
    );
    let manifest = fixture.read("Cargo.toml");
    assert!(
        manifest.contains("mail = { path = \"crates/mail\" }"),
        "{manifest}"
    );
    assert!(manifest.contains("reqwest = { version = "), "{manifest}");
    let config = fixture.read("rok.toml");
    assert!(
        config.contains("[recipes.mail]\nversion = \"0.1.0\"\ndriver = \"resend\"\n"),
        "{config}"
    );
}

#[test]
fn env_keys_go_to_the_example_file_and_never_to_dot_env() {
    let fixture = app();
    ok(&fixture, &["add", "mail", "--driver", "smtp"]);

    let example = fixture.read(".env.example");
    for key in ["SMTP_HOST=localhost", "SMTP_PASSWORD=", "MAIL_FROM="] {
        assert!(example.contains(key), "{example}");
    }
    assert_eq!(fixture.read(".env"), SECRETS);

    ok(&fixture, &["remove", "mail"]);
    assert!(!fixture.exists(".env.example"));
    assert_eq!(fixture.read(".env"), SECRETS);
}

#[test]
fn add_installs_required_recipes_first() {
    let fixture = app();
    let outcome = ok(&fixture, &["add", "auth", "--driver", "session"]);

    let installed: Vec<(&str, &str)> = outcome["data"]["installed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["recipe"].as_str().unwrap(),
                entry["driver"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(installed, [("mail", "resend"), ("auth", "session")]);
    assert!(
        fixture
            .read("crates/auth/Cargo.toml")
            .contains("mail.workspace = true")
    );
    assert!(
        fixture
            .read("crates/auth/src/lib.rs")
            .contains("pub fn session_middleware")
    );
    assert!(fixture.exists("crates/mail/src/lib.rs"));
    let config = fixture.read("rok.toml");
    assert!(config.contains("[recipes.auth]") && config.contains("[recipes.mail]"));
}

#[test]
fn required_recipes_take_their_driver_from_recipe_equals_driver() {
    let fixture = app();
    ok(
        &fixture,
        &["add", "auth", "--driver", "jwt", "--driver", "mail=log"],
    );
    assert!(
        fixture
            .read("crates/mail/src/lib.rs")
            .contains("pub struct LogMailer")
    );
    assert!(
        fixture
            .read("crates/auth/src/lib.rs")
            .contains("pub struct JwtAuth")
    );
}

#[test]
fn an_installed_requirement_is_reused() {
    let fixture = app();
    ok(&fixture, &["add", "mail", "--driver", "smtp"]);
    let outcome = ok(&fixture, &["add", "auth"]);
    assert_eq!(outcome["data"]["installed"].as_array().unwrap().len(), 1);
    assert!(
        fixture
            .read("crates/mail/src/lib.rs")
            .contains("SmtpMailer")
    );
}

#[test]
fn storage_s3_brings_its_terraform_module_when_asked() {
    let fixture = app();
    ok(
        &fixture,
        &["add", "storage", "--driver", "s3", "--terraform"],
    );

    assert!(
        fixture
            .read("crates/storage/src/lib.rs")
            .contains("pub struct S3Storage")
    );
    for file in ["main.tf", "variables.tf", "outputs.tf"] {
        assert!(fixture.exists(&format!("infra/terraform/modules/storage/{file}")));
    }
    assert!(
        fixture
            .read("infra/terraform/modules/storage/main.tf")
            .contains("resource \"aws_s3_bucket\" \"this\"")
    );
    assert!(fixture.read("rok.toml").contains("terraform = true"));

    ok(&fixture, &["remove", "storage"]);
    assert!(!fixture.exists("infra"));
    assert!(!fixture.exists("crates/storage"));
}

#[test]
fn terraform_is_refused_for_a_driver_without_a_module() {
    let fixture = app();
    let stderr = refused(
        &fixture,
        &["add", "storage", "--driver", "local", "--terraform"],
    );
    assert!(stderr.contains("has no Terraform module"), "{stderr}");
    assert!(!fixture.exists("crates/storage"));
}

#[test]
fn unknown_recipes_drivers_and_reinstalls_are_refused() {
    let fixture = app();
    let stderr = refused(&fixture, &["add", "cache"]);
    assert!(
        stderr.contains("no recipe `cache`; available: auth, mail, storage"),
        "{stderr}"
    );
    let stderr = refused(&fixture, &["add", "mail", "--driver", "pigeon"]);
    assert!(
        stderr.contains("no driver `pigeon`; choose one of: resend, smtp, log"),
        "{stderr}"
    );

    ok(&fixture, &["add", "mail", "--driver", "log"]);
    let stderr = refused(&fixture, &["add", "mail", "--driver", "smtp"]);
    assert!(
        stderr.contains("already installed (driver `log`)"),
        "{stderr}"
    );
}

#[test]
fn remove_refuses_while_recipes_or_crates_depend_on_it_and_names_them() {
    let fixture = app();
    ok(&fixture, &["add", "auth"]);
    let manifest = fixture.read("crates/posts/Cargo.toml");
    fixture.write(
        "crates/posts/Cargo.toml",
        &format!("{manifest}\n[dependencies]\nmail.workspace = true\n"),
    );

    let stderr = refused(&fixture, &["remove", "mail"]);
    assert!(stderr.contains("required by recipe auth"), "{stderr}");
    assert!(
        stderr.contains("depended on by crates/posts/Cargo.toml"),
        "{stderr}"
    );
    assert!(fixture.exists("crates/mail/src/lib.rs"));
}

#[test]
fn remove_reverts_everything_add_did() {
    let fixture = app();
    let before = snapshot(&fixture);
    ok(&fixture, &["add", "auth", "--driver", "jwt"]);

    let outcome = ok(&fixture, &["remove", "auth"]);
    assert!(
        outcome["data"]["deleted"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file == "crates/auth/src/lib.rs")
    );
    ok(&fixture, &["remove", "mail"]);

    assert_eq!(snapshot(&fixture), before);
    let manifest = fixture.read("Cargo.toml");
    assert!(!manifest.contains("crates/auth") && !manifest.contains("crates/mail"));
}

#[test]
fn changed_recipe_files_need_force() {
    let fixture = app();
    ok(&fixture, &["add", "mail", "--driver", "log"]);
    // Reformatting isn't a change; editing is.
    fixture.replace(
        "crates/mail/Cargo.toml",
        "\n\n[dev-dependencies]",
        "\n[dev-dependencies]",
    );
    fixture.write(
        "crates/mail/src/templates.rs",
        "pub const HELLO: &str = \"hi\";\n",
    );

    let stderr = refused(&fixture, &["remove", "mail"]);
    assert!(
        stderr.contains(
            "changed or added since it was installed: crates/mail/src/templates.rs; pass --force"
        ),
        "{stderr}"
    );

    ok(&fixture, &["remove", "mail", "--force"]);
    assert!(!fixture.exists("crates/mail"));
}

#[test]
fn remove_dry_run_writes_nothing() {
    let fixture = app();
    ok(&fixture, &["add", "storage", "--driver", "local"]);
    let before = snapshot(&fixture);

    let outcome = ok(&fixture, &["remove", "storage", "--dry-run"]);
    assert_eq!(outcome["data"]["dry_run"], true);
    assert!(
        outcome["data"]["diff"]
            .as_str()
            .unwrap()
            .contains("crates/storage/src/lib.rs")
    );
    assert_eq!(snapshot(&fixture), before);
}

#[test]
fn add_is_one_undoable_transaction() {
    let fixture = app();
    let before = snapshot(&fixture);
    let manifest = fixture.read("Cargo.toml");
    ok(&fixture, &["add", "auth"]);

    ok(&fixture, &["undo"]);
    assert_eq!(snapshot(&fixture), before);
    assert_eq!(fixture.read("Cargo.toml"), manifest);
}

#[test]
fn remove_of_a_recipe_that_is_not_installed_is_refused() {
    let fixture = app();
    let stderr = refused(&fixture, &["remove", "mail"]);
    assert!(
        stderr.contains("recipe `mail` isn't installed; installed: none"),
        "{stderr}"
    );
}

#[test]
fn list_shows_available_and_installed_recipes_with_versions() {
    let fixture = app();
    ok(&fixture, &["add", "mail", "--driver", "smtp"]);

    let execution = fixture.run(&["list"]);
    assert_eq!(execution.code, 0, "{}", execution.stderr);
    let lines: Vec<&str> = execution.stdout.lines().collect();
    assert!(lines[0].starts_with("recipe"), "{}", execution.stdout);
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("mail") && line.contains("0.1.0 smtp")),
        "{}",
        execution.stdout
    );
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("storage") && line.contains("s3* (terraform), local")),
        "{}",
        execution.stdout
    );

    let outcome = ok(&fixture, &["list"]);
    let recipes = outcome["data"]["recipes"].as_array().unwrap();
    let mail = recipes
        .iter()
        .find(|recipe| recipe["name"] == "mail")
        .unwrap();
    assert_eq!(mail["version"], "0.1.0");
    assert_eq!(mail["installed"]["driver"], "smtp");
    let auth = recipes
        .iter()
        .find(|recipe| recipe["name"] == "auth")
        .unwrap();
    assert_eq!(auth["installed"], Value::Null);
    assert_eq!(auth["requires"][0], "mail");
}

#[test]
fn add_and_remove_take_json_payloads_and_publish_schemas() {
    let fixture = app();
    ok(
        &fixture,
        &[
            "add",
            "-j",
            r#"{"recipe": "auth", "driver": "jwt", "drivers": {"mail": "log"}}"#,
        ],
    );
    assert!(fixture.read("crates/mail/src/lib.rs").contains("LogMailer"));

    for name in ["add", "remove"] {
        let execution = fixture.run(&["schema", name]);
        assert_eq!(execution.code, 0, "{}", execution.stderr);
        assert!(execution.stdout.contains("\"recipe\""));
    }
}

#[test]
fn new_installs_the_chosen_recipes() {
    let fixture = Fixture::empty();
    let execution = fixture.run(&[
        "new",
        "shop",
        "--recipe",
        "auth",
        "--driver",
        "auth=jwt",
        "--recipe",
        "storage",
        "--driver",
        "storage=local",
        "--no-git",
        "--no-hooks",
    ]);
    assert_eq!(
        execution.code, 0,
        "{}{}",
        execution.stdout, execution.stderr
    );

    assert!(
        fixture
            .read("shop/crates/auth/src/lib.rs")
            .contains("JwtAuth")
    );
    assert!(
        fixture
            .read("shop/crates/mail/src/lib.rs")
            .contains("ResendMailer")
    );
    assert!(
        fixture
            .read("shop/crates/storage/src/lib.rs")
            .contains("LocalStorage")
    );
    let config = fixture.read("shop/rok.toml");
    for section in ["[recipes.auth]", "[recipes.mail]", "[recipes.storage]"] {
        assert!(config.contains(section), "{config}");
    }
    assert!(fixture.read("shop/.env.example").contains("JWT_SECRET="));
}

#[test]
fn new_refuses_unknown_recipes_before_writing_anything() {
    let fixture = Fixture::empty();
    let execution = fixture.run(&["new", "shop", "--recipe", "cache", "--no-git"]);
    assert_eq!(execution.code, 2);
    assert!(
        execution.stderr.contains("no recipe `cache`"),
        "{}",
        execution.stderr
    );
    assert!(!fixture.exists("shop"));
}
