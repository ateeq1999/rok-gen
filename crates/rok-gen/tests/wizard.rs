//! `cargo rok integrate --wizard`, driven by scripted answers on stdin.

// Test harness code: panicking on a broken fixture is the point.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Fixture;
use serde_json::Value;

const SERVICE: &str = "crates/posts/src/post_service.rs";

fn tidy_fixture() -> Fixture {
    let fixture = Fixture::app();
    fixture.write(
        SERVICE,
        "use std::sync::Arc;\n\nuse crate::post_repository::PostRepository;\n\npub struct PostService {\n    posts: Arc<dyn PostRepository>,\n}\n\nimpl PostService {\n    pub const fn new(posts: Arc<dyn PostRepository>) -> Self {\n        Self { posts }\n    }\n\n    pub fn tidy(&self, id: u32) -> u32 {\n        let first = self.posts.count(id);\n        first\n    }\n}\n",
    );
    fixture
}

fn wizard(fixture: &Fixture, answers: &[&str]) -> rok_gen::Execution {
    let mut stdin = answers.join("\n");
    stdin.push('\n');
    fixture.run_with_stdin(&["integrate", "--wizard"], &stdin)
}

#[test]
fn the_wizard_inserts_code_where_the_answers_say() {
    let fixture = tidy_fixture();
    let execution = wizard(
        &fixture,
        &[
            "2",             // insert code into a function
            "posts/service", // which file
            "2",             // PostService::tidy
            "4",             // after [0] let first = ...
            "log(first);",
            "",  // end of code
            "n", // no more steps
            "1", // apply
        ],
    );
    assert_eq!(
        execution.code, 0,
        "{}{}",
        execution.stdout, execution.stderr
    );
    assert!(
        execution.stderr.contains("What should this step do?")
            && execution
                .stderr
                .contains("after  [0] L15  let first = self.posts.count(id);"),
        "{}",
        execution.stderr
    );
    assert!(
        fixture
            .read(SERVICE)
            .contains("let first = self.posts.count(id);\n        log(first);"),
        "{}",
        fixture.read(SERVICE)
    );
}

#[test]
fn the_wizard_can_save_an_injection_plan_instead_of_applying_it() {
    let fixture = Fixture::app();
    std::fs::remove_dir_all(fixture.path("crates/posts")).unwrap();
    std::fs::remove_dir_all(fixture.path("crates/comments")).unwrap();
    fixture.write("crates/error/src/lib.rs", "//! Every error type.\n");
    fixture.remove("crates/error/src/posts.rs");
    fixture.replace(
        "Cargo.toml",
        "comments = { path = \"crates/comments\" }\n",
        "",
    );
    fixture.replace("Cargo.toml", "posts = { path = \"crates/posts\" }\n", "");
    fixture.replace("Cargo.toml", "posts.workspace = true\n", "");
    fixture.write("src/lib.rs", "pub async fn run() {\n    let pool = db::connect();\n    let seed_service = SeedService::new();\n    App::new().app_data(web::Data::new(seed_service.clone()));\n}\n");
    fixture.write(
        "crates/router/Cargo.toml",
        "[package]\nname = \"router\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    );
    fixture.write(
        "crates/router/src/lib.rs",
        "mod openapi;\n\nuse macros::module;\n\n#[module(imports = [])]\npub struct AppModule;\n",
    );
    fixture.write("crates/router/src/openapi.rs", "#[derive(OpenApi)]\n#[openapi(paths(), components(schemas()), tags())]\npub struct ApiDoc;\n");
    for feature in ["articles", "comments"] {
        let (code, outcome) =
            fixture.run_json(&["make:feature", feature, "--field", "title:string"]);
        assert_eq!(code, 0, "{outcome:#}");
    }
    let before = fixture.read("crates/articles/src/article_service.rs");

    let execution = wizard(
        &fixture,
        &[
            "1", // inject a repository into a service
            "1", // ArticleService
            "2", // comments::CommentRepository
            "",  // field name: the default, `comments`
            "n",
            "3", // save
            "plans/wired.json",
        ],
    );
    assert_eq!(
        execution.code, 0,
        "{}{}",
        execution.stdout, execution.stderr
    );
    assert!(
        execution.stderr.contains("Field name [comments]"),
        "{}",
        execution.stderr
    );
    assert_eq!(
        fixture.read("crates/articles/src/article_service.rs"),
        before
    );

    let plan: Value = serde_json::from_str(&fixture.read("plans/wired.json")).unwrap();
    assert_eq!(plan["steps"][0]["op"], "inject_dependency");
    assert_eq!(plan["steps"][0]["service"], "ArticleService");
    assert_eq!(plan["steps"][0]["dep"], "comments::CommentRepository");
    assert_eq!(plan["steps"][0]["as"], "comments");
}

#[test]
fn bad_or_missing_answers_stop_the_wizard() {
    let fixture = tidy_fixture();
    let execution = wizard(&fixture, &["9", "x", "0"]);
    assert_eq!(execution.code, 2);
    assert!(
        execution.stderr.contains("pick a number from 1 to 4"),
        "{}",
        execution.stderr
    );
    assert!(
        execution.stderr.contains("no valid answer"),
        "{}",
        execution.stderr
    );

    let execution = fixture.run_with_stdin(&["integrate", "--wizard"], "2\n");
    assert_eq!(execution.code, 2);
    assert!(
        execution.stderr.contains("ran out of answers"),
        "{}",
        execution.stderr
    );
    assert!(!fixture.read(SERVICE).contains("log("));
}

#[test]
fn the_wizard_conflicts_with_a_plan_from_flags() {
    let fixture = tidy_fixture();
    let execution = fixture.run(&["integrate", "--wizard", "--preset", "hook"]);
    assert_eq!(execution.code, 2);
    assert!(
        execution.stderr.contains("cannot be used with"),
        "{}",
        execution.stderr
    );
}
