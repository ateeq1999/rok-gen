//! `cargo rok inspect` and `cargo rok integrate`.

// Test harness code: panicking on a broken fixture is the point.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Fixture;
use serde_json::{Value, json};

const SERVICE: &str = "crates/posts/src/post_service.rs";

/// The app fixture with a service whose `tidy` method exercises every
/// statement and block kind the selectors address.
fn fixture() -> Fixture {
    let fixture = Fixture::app();
    fixture.write(
        SERVICE,
        r"use std::sync::Arc;

use crate::post_repository::PostRepository;

pub struct PostService {
    posts: Arc<dyn PostRepository>,
}

impl PostService {
    pub const fn new(posts: Arc<dyn PostRepository>) -> Self {
        Self { posts }
    }

    pub fn tidy(&self, id: u32) -> u32 {
        let first = self.posts.count(id);
        if first > 0 {
            self.posts.touch(id);
        } else {
            self.posts.reset(id);
        }
        match first {
            0 => {
                self.posts.reset(id);
            }
            _ => {}
        }
        self.posts.touch(id);
        first
    }
}
",
    );
    fixture
}

fn plan(fixture: &Fixture, steps: &Value, extra: &[&str]) -> (i32, Value) {
    let payload = json!({ "steps": steps }).to_string();
    let mut args = vec!["integrate", "-j", payload.as_str()];
    args.extend_from_slice(extra);
    fixture.run_json(&args)
}

fn insert(at: &Value, code: &str) -> Value {
    json!([{ "op": "insert", "at": at, "code": code }])
}

#[test]
fn inspect_lists_functions_statements_and_within_steps() {
    let fixture = fixture();
    let (code, outcome) =
        fixture.run_json(&["inspect", "posts/service", "--fn", "PostService::tidy"]);
    assert_eq!(code, 0, "{outcome:#}");
    assert_eq!(outcome["data"]["file"], SERVICE);
    let statements = &outcome["data"]["inspection"]["functions"][0]["statements"];
    assert_eq!(statements[0]["text"], "let first = self.posts.count(id);");
    assert_eq!(statements[1]["within"], json!({ "if": 0 }));
    assert_eq!(statements[2]["within"], json!({ "match": 0 }));
    assert_eq!(statements[4]["tail"], true);

    let human = fixture.run(&["inspect", "posts/service"]).stdout;
    assert!(human.contains("struct PostService"), "{human}");
    assert!(human.contains("fn PostService::tidy"), "{human}");
    assert!(
        human.contains("[1] L16  if first > 0 {  within {\"if\":0}"),
        "{human}"
    );

    let execution = fixture.run(&["inspect", "posts/service", "--fn", "PostService::nope"]);
    assert_eq!(execution.code, 2);
    assert!(
        execution
            .stderr
            .contains("available: PostService::new, PostService::tidy"),
        "{}",
        execution.stderr
    );
    let execution = fixture.run(&["inspect", "nonsense"]);
    assert!(
        execution.stderr.contains("isn't a file alias"),
        "{}",
        execution.stderr
    );
}

#[test]
fn every_matcher_and_position_resolves() {
    let cases = [
        (
            json!({ "after": { "call": "self.posts.count" } }),
            "let first = self.posts.count(id);\n        log(1);",
        ),
        (
            json!({ "before": { "let": "first" } }),
            "log(1);\n        let first",
        ),
        (json!({ "start": true }), "-> u32 {\n        log(1);"),
        (json!({ "end": true }), "log(1);\n        first\n    }"),
        (
            json!({ "before": { "index": "tail" } }),
            "log(1);\n        first\n    }",
        ),
        (
            json!({ "after": { "index": 2 } }),
            "            _ => {}\n        }\n        log(1);",
        ),
        (
            json!({ "after": { "contains": "count(" } }),
            "count(id);\n        log(1);",
        ),
    ];
    for (selector, expected) in cases {
        let fixture = fixture();
        let mut at = json!({ "file": "posts/service", "fn": "PostService::tidy" });
        at.as_object_mut()
            .unwrap()
            .extend(selector.as_object().unwrap().clone());
        let (code, outcome) = plan(&fixture, &insert(&at, "log(1);"), &[]);
        assert_eq!(code, 0, "{selector}: {outcome:#}");
        let text = fixture.read(SERVICE);
        assert!(
            text.contains(expected),
            "{selector}: expected {expected:?} in\n{text}"
        );
    }
}

#[test]
fn within_enters_branches_arms_and_blocks() {
    let cases = [
        (
            json!([{ "if": 0 }]),
            "if first > 0 {\n            log(1);\n            self.posts.touch(id);",
        ),
        (
            json!([{ "if": 0, "branch": "else" }]),
            "} else {\n            log(1);\n            self.posts.reset(id);",
        ),
        (
            json!([{ "match": 0, "arm": "0" }]),
            "0 => {\n                log(1);\n                self.posts.reset(id);",
        ),
    ];
    for (within, expected) in cases {
        let fixture = fixture();
        let at = json!({ "file": "posts/service", "fn": "PostService::tidy", "within": within, "start": true });
        let (code, outcome) = plan(&fixture, &insert(&at, "log(1);"), &[]);
        assert_eq!(code, 0, "{within}: {outcome:#}");
        let text = fixture.read(SERVICE);
        assert!(
            text.contains(expected),
            "{within}: expected {expected:?} in\n{text}"
        );
    }

    let fixture = fixture();
    let at = json!({ "file": "posts/service", "fn": "PostService::tidy", "within": [{ "match": 0, "arm": "1" }], "start": true });
    let (code, outcome) = plan(&fixture, &insert(&at, "log(1);"), &[]);
    assert_eq!(code, 2);
    assert!(
        outcome["data"]["error"]
            .as_str()
            .unwrap()
            .contains("arms are: 0, _"),
        "{outcome:#}"
    );
}

#[test]
fn ambiguous_matches_list_the_candidates_unless_narrowed() {
    let fixture = fixture();
    let at = json!({ "file": "posts/service", "fn": "PostService::tidy", "after": { "contains": "self.posts" } });
    let (code, outcome) = plan(&fixture, &insert(&at, "log(1);"), &[]);
    assert_eq!(code, 2);
    let error = outcome["data"]["error"].as_str().unwrap();
    assert!(
        error.contains("statements in `PostService::tidy` match; add `occurrence`"),
        "{error}"
    );
    assert!(
        error.contains("L15 [0] let first = self.posts.count(id);"),
        "{error}"
    );

    let mut narrowed = at.clone();
    narrowed["occurrence"] = json!(2);
    let (code, _) = plan(&fixture, &insert(&narrowed, "log(2);"), &[]);
    assert_eq!(code, 0);
    assert!(
        fixture
            .read(SERVICE)
            .contains("        }\n        log(2);\n        match first")
    );

    let fixture = self::fixture();
    let mut all = at;
    all["all"] = json!(true);
    let (code, _) = plan(&fixture, &insert(&all, "log(3);"), &[]);
    assert_eq!(code, 0);
    assert!(fixture.read(SERVICE).matches("log(3);").count() >= 3);
}

#[test]
fn replace_needs_force_on_the_command_line() {
    let fixture = fixture();
    let before = fixture.read(SERVICE);
    let steps = json!([{
        "op": "replace",
        "at": { "file": "posts/service", "fn": "PostService::tidy", "stmt": { "call": "self.posts.count" } },
        "code": "let first = 7;",
    }]);
    let (code, outcome) = plan(&fixture, &steps, &[]);
    assert_eq!(code, 2);
    assert!(
        outcome["data"]["error"]
            .as_str()
            .unwrap()
            .contains("pass --force"),
        "{outcome:#}"
    );
    assert_eq!(fixture.read(SERVICE), before);

    // `force` inside the payload is not a thing.
    let payload = json!({ "steps": steps, "force": true }).to_string();
    let (code, _) = fixture.run_json(&["integrate", "-j", &payload]);
    assert_eq!(code, 2);

    let (code, outcome) = plan(&fixture, &steps, &["--force", "--allow-dirty"]);
    assert_eq!(code, 0, "{outcome:#}");
    let text = fixture.read(SERVICE);
    assert!(
        text.contains("let first = 7;") && !text.contains("count(id)"),
        "{text}"
    );
}

#[test]
fn invalid_snippets_and_failed_steps_write_nothing() {
    let fixture = fixture();
    let before = fixture.read(SERVICE);
    let at = json!({ "file": "posts/service", "fn": "PostService::tidy", "start": true });
    let (code, outcome) = plan(&fixture, &insert(&at, "let = ;"), &[]);
    assert_eq!(code, 2);
    assert!(
        outcome["data"]["error"]
            .as_str()
            .unwrap()
            .contains("isn't valid Rust statements"),
        "{outcome:#}"
    );

    let steps = json!([
        { "op": "insert", "at": at, "code": "log(1);" },
        { "op": "insert", "at": { "file": "posts/service", "fn": "PostService::nope", "start": true }, "code": "log(2);" },
    ]);
    let (code, outcome) = plan(&fixture, &steps, &[]);
    assert_eq!(code, 2);
    assert!(
        outcome["data"]["error"]
            .as_str()
            .unwrap()
            .starts_with("step 2 (insert)"),
        "{outcome:#}"
    );
    assert_eq!(fixture.read(SERVICE), before, "step 1 must not be written");
}

#[test]
fn add_field_with_a_default_updates_every_literal() {
    let fixture = fixture();
    fixture.write(
        "crates/posts/src/post_draft.rs",
        "pub struct Draft {\n    pub title: String,\n}\n\npub fn one() -> Draft {\n    Draft { title: String::new() }\n}\n\npub fn two(base: Draft) -> Draft {\n    Draft { ..base }\n}\n",
    );
    let steps = json!([{
        "op": "add_field", "file": "posts/post_draft", "struct": "Draft",
        "field": "pub tags: Vec<String>", "default": "Vec::new()",
    }]);
    let (code, outcome) = plan(&fixture, &steps, &[]);
    assert_eq!(code, 0, "{outcome:#}");
    let text = fixture.read("crates/posts/src/post_draft.rs");
    assert!(
        text.contains("    pub title: String,\n    pub tags: Vec<String>,\n}"),
        "{text}"
    );
    assert!(
        text.contains("Draft {\n        title: String::new(),\n        tags: Vec::new(),\n    }"),
        "{text}"
    );
    assert!(
        text.contains("Draft { ..base }"),
        "rest literals are left alone:\n{text}"
    );
}

#[test]
fn vars_render_into_every_string() {
    let fixture = fixture();
    let payload = json!({
        "vars": { "file": "posts/service", "what": "touch" },
        "steps": [{
            "op": "insert",
            "at": { "file": "{{ file }}", "fn": "PostService::tidy", "start": true },
            "code": "self.posts.{{ what }}(id);",
        }],
    })
    .to_string();
    let (code, outcome) = fixture.run_json(&["integrate", "-j", &payload, "--dry-run"]);
    assert_eq!(code, 0, "{outcome:#}");
    let diff = outcome["data"]["diff"].as_str().unwrap();
    assert!(diff.contains("+        self.posts.touch(id);"), "{diff}");
}

/// The flagship case from the spec: wire `comments` into `posts`.
#[test]
fn inject_dependency_wires_a_port_across_features() {
    let fixture = Fixture::app();
    fixture.remove("crates/error/src/posts.rs");
    fixture.write("crates/error/src/lib.rs", "//! Every error type.\n");
    std::fs::remove_dir_all(fixture.path("crates/posts")).unwrap();
    std::fs::remove_dir_all(fixture.path("crates/comments")).unwrap();
    fixture.write("src/lib.rs", "use std::sync::Arc;\n\nuse actix_web::{App, HttpServer, web};\n\npub async fn run() -> std::io::Result<()> {\n    let pool = db::connect();\n    let seed_service = SeedService::new();\n\n    HttpServer::new(move || App::new().app_data(web::Data::new(seed_service.clone())))\n        .bind(\"127.0.0.1:8080\")?\n        .run()\n        .await\n}\n");
    fixture.replace(
        "Cargo.toml",
        "comments = { path = \"crates/comments\" }\n",
        "",
    );
    fixture.replace("Cargo.toml", "posts = { path = \"crates/posts\" }\n", "");
    fixture.replace("Cargo.toml", "posts.workspace = true\n", "");
    fixture.write(
        "crates/router/Cargo.toml",
        "[package]\nname = \"router\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    );
    fixture.write(
        "crates/router/src/lib.rs",
        "mod openapi;\n\nuse macros::module;\n\n#[module(imports = [])]\npub struct AppModule;\n",
    );
    fixture.write("crates/router/src/openapi.rs", "#[derive(OpenApi)]\n#[openapi(paths(), components(schemas()), tags())]\npub struct ApiDoc;\n");
    for args in [
        &["make:feature", "posts", "--field", "title:string"][..],
        &[
            "make:feature",
            "comments",
            "--field",
            "body:text",
            "--field",
            "post_id:uuid",
        ][..],
    ] {
        let (code, outcome) = fixture.run_json(args);
        assert_eq!(code, 0, "{args:?}: {outcome:#}");
    }

    let steps = json!([
        { "op": "inject_dependency", "into": "posts/service", "service": "PostService",
          "dep": "comments::CommentRepository", "as": "comments" },
        { "op": "insert",
          "at": { "file": "posts/service", "fn": "PostService::delete", "within": [{ "if": 0 }], "start": true },
          "code": "drop(self.comments.find_by_id(id).await);" },
    ]);
    let (code, outcome) = plan(&fixture, &steps, &[]);
    assert_eq!(code, 0, "{outcome:#}");

    let service = fixture.read("crates/posts/src/post_service.rs");
    for expected in [
        "    posts: Arc<dyn PostRepository>,\n    comments: Arc<dyn CommentRepository>,\n}",
        "comments: Arc<dyn CommentRepository>) -> Self {\n        Self { posts, comments }",
        "use comments::CommentRepository;",
        "if self.posts.delete(id).await? {\n            drop(self.comments.find_by_id(id).await);",
        "Arc::new(comments::FakeCommentRepository::default()),",
    ] {
        assert!(service.contains(expected), "{expected} missing:\n{service}");
    }
    let root = fixture.read("src/lib.rs");
    assert!(
        root.contains("Arc::new(PgCommentRepository::new(pool.clone()))"),
        "{root}"
    );
    assert!(root.contains("use comments::"), "{root}");
    let manifest = fixture.read("crates/posts/Cargo.toml");
    assert!(manifest.contains("comments.workspace = true"), "{manifest}");
    assert!(
        manifest.contains("comments = { workspace = true, features = [\"fake\"] }"),
        "{manifest}"
    );
    assert!(
        fixture
            .read("rok.toml")
            .contains("allow = [\"posts -> comments\"]")
    );
    assert_eq!(fixture.arch_rules(), Vec::<String>::new());

    // The reverse edge would now be a cycle.
    let reverse = json!([{ "op": "inject_dependency", "into": "comments/service", "service": "CommentService",
                           "dep": "posts::PostRepository", "as": "posts" }]);
    let (code, outcome) = plan(&fixture, &reverse, &[]);
    assert_eq!(code, 2);
    assert!(
        outcome["data"]["error"]
            .as_str()
            .unwrap()
            .contains("dependency cycle"),
        "{outcome:#}"
    );
}
