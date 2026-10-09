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

/// The app fixture's hand-written features replaced by generated ones, so
/// repositories, fakes and services have their real shape.
fn generated(features: &[&[&str]]) -> Fixture {
    let fixture = Fixture::app();
    fixture.remove("crates/error/src/posts.rs");
    fixture.write("crates/error/src/lib.rs", "//! Every error type.\n");
    std::fs::remove_dir_all(fixture.path("crates/posts")).unwrap();
    std::fs::remove_dir_all(fixture.path("crates/comments")).unwrap();
    fixture.write(
        "src/lib.rs",
        "use std::sync::Arc;\n\nuse actix_web::{App, HttpServer, web};\n\npub async fn run() -> std::io::Result<()> {\n    let pool = db::connect();\n    let seed_service = SeedService::new();\n\n    HttpServer::new(move || App::new().app_data(web::Data::new(seed_service.clone())))\n        .bind(\"127.0.0.1:8080\")?\n        .run()\n        .await\n}\n",
    );
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
    fixture.write(
        "crates/router/src/openapi.rs",
        "#[derive(OpenApi)]\n#[openapi(paths(), components(schemas()), tags())]\npub struct ApiDoc;\n",
    );
    for args in features {
        let (code, outcome) = fixture.run_json(args);
        assert_eq!(code, 0, "{args:?}: {outcome:#}");
    }
    fixture
}

fn error_of(outcome: &Value) -> String {
    outcome["data"]["error"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn add_method_needs_a_body_for_every_implementor() {
    let fixture = generated(&[&["make:feature", "articles", "--field", "title:string"]]);
    let step = |bodies: Value| {
        json!([{ "op": "add_method", "file": "articles/repository", "trait": "ArticleRepository",
                  "signature": "async fn count(&self) -> Result<u64, ArticlesError>",
                  "doc": ["How many there are."], "bodies": bodies }])
    };
    let (code, outcome) = plan(
        &fixture,
        &step(json!({ "PgArticleRepository": "Ok(0)" })),
        &[],
    );
    assert_eq!(code, 2);
    assert!(
        error_of(&outcome).contains("also implemented by FakeArticleRepository"),
        "{outcome:#}"
    );

    let three = json!({ "PgArticleRepository": "Ok(0)", "FakeArticleRepository": "Ok(1)", "Nope": "Ok(2)" });
    let (code, outcome) = plan(&fixture, &step(three), &[]);
    assert_eq!(code, 2);
    assert!(
        error_of(&outcome).contains("no `impl ArticleRepository for` `Nope`"),
        "{outcome:#}"
    );

    let both = json!({ "PgArticleRepository": "Ok(0)", "FakeArticleRepository": "Ok(1)" });
    let (code, outcome) = plan(&fixture, &step(both.clone()), &[]);
    assert_eq!(code, 0, "{outcome:#}");
    let repository = fixture.read("crates/articles/src/article_repository.rs");
    assert!(
        repository.contains("    /// How many there are.\n    async fn count(&self) -> Result<u64, ArticlesError>;\n}"),
        "{repository}"
    );
    assert_eq!(
        repository
            .matches("async fn count(&self) -> Result<u64, ArticlesError> {")
            .count(),
        2,
        "{repository}"
    );

    let (code, outcome) = plan(&fixture, &step(both), &[]);
    assert_eq!(code, 2);
    assert!(
        error_of(&outcome).contains("already has `count`"),
        "{outcome:#}"
    );
}

#[test]
fn migration_steps_are_numbered_in_the_same_transaction() {
    let fixture = fixture();
    let steps = json!([
        { "op": "migration", "name": "first", "sql": "SELECT 1;" },
        { "op": "migration", "name": "second", "sql": "SELECT 2;" },
    ]);
    let (code, outcome) = plan(&fixture, &steps, &[]);
    assert_eq!(code, 0, "{outcome:#}");
    assert_eq!(
        fixture.read("crates/db/migrations/0002_first.sql"),
        "SELECT 1;\n"
    );
    assert_eq!(
        fixture.read("crates/db/migrations/0003_second.sql"),
        "SELECT 2;\n"
    );
}

#[test]
fn has_many_preset_cascades_deletes_and_unlinks_cleanly() {
    let fixture = generated(&[
        &["make:feature", "articles", "--field", "title:string"],
        &[
            "make:feature",
            "comments",
            "--field",
            "body:text",
            "--field",
            "article_id:uuid",
        ],
    ]);
    let before = fixture.read("crates/articles/src/article_service.rs");
    let (code, outcome) = fixture.run_json(&[
        "integrate",
        "--preset",
        "has-many",
        "--var",
        "source=comments",
        "--var",
        "target=articles",
    ]);
    assert_eq!(code, 0, "{outcome:#}");

    let service = fixture.read("crates/articles/src/article_service.rs");
    assert!(
        service.contains(
            "pub async fn delete(&self, id: Uuid) -> Result<(), ArticlesError> {\n        self.comments\n            .delete_by_article(id)"
        ),
        "{service}"
    );
    let repository = fixture.read("crates/comments/src/comment_repository.rs");
    assert!(
        repository.contains("DELETE FROM comments WHERE article_id = $1"),
        "{repository}"
    );
    assert!(
        repository.contains("items.retain(|item| item.article_id != article_id);"),
        "{repository}"
    );
    assert!(fixture.exists("crates/db/migrations/0004_index_comments_article_id.sql"));
    assert_eq!(fixture.arch_rules(), Vec::<String>::new());

    let journal = fixture.run_json(&["journal"]).1;
    assert_eq!(
        journal["data"]["transactions"][2]["label"],
        "comments into articles"
    );
    let (code, _) = fixture.run_json(&["unlink", "comments", "articles"]);
    assert_eq!(code, 0);
    assert_eq!(
        fixture.read("crates/articles/src/article_service.rs"),
        before
    );
    assert!(
        !fixture
            .read("crates/comments/src/comment_repository.rs")
            .contains("delete_by_article")
    );
    assert!(!fixture.exists("crates/db/migrations/0004_index_comments_article_id.sql"));
}

#[test]
fn presets_validate_their_inputs() {
    let fixture = generated(&[
        &["make:feature", "articles", "--field", "title:string"],
        &["make:feature", "comments", "--field", "body:text"],
    ]);
    let cases: [(&[&str], &str); 4] = [
        (
            &["integrate", "--preset", "nope"],
            "no preset `nope`; available: has-many",
        ),
        (
            &[
                "integrate",
                "--preset",
                "has-many",
                "--var",
                "source=comments",
            ],
            "needs `vars.target`",
        ),
        (
            &[
                "integrate",
                "--preset",
                "has-many",
                "--var",
                "source=comments",
                "--var",
                "target=articles",
            ],
            "no required `article_id: Uuid` field",
        ),
        (
            &["integrate", "--preset", "has-many", "--var", "nonsense"],
            "expected `key=value`",
        ),
    ];
    for (args, expected) in cases {
        let execution = fixture.run(args);
        assert_eq!(execution.code, 2, "{args:?}");
        assert!(
            execution.stderr.contains(expected),
            "{args:?}: {}",
            execution.stderr
        );
    }
    let payload =
        json!({ "preset": "has-many", "steps": [{ "op": "migration", "name": "x", "sql": "" }] })
            .to_string();
    let execution = fixture.run(&["integrate", "-j", &payload]);
    assert!(
        execution.stderr.contains("either `preset` or `steps`"),
        "{}",
        execution.stderr
    );
}

#[test]
fn emit_plan_writes_the_expanded_plan_and_changes_nothing_else() {
    let fixture = generated(&[
        &["make:feature", "articles", "--field", "title:string"],
        &[
            "make:feature",
            "comments",
            "--field",
            "body:text",
            "--field",
            "article_id:uuid",
        ],
    ]);
    let service = fixture.read("crates/articles/src/article_service.rs");
    let (code, outcome) = fixture.run_json(&[
        "integrate",
        "--preset",
        "has-many",
        "--var",
        "source=comments",
        "--var",
        "target=articles",
        "--emit-plan",
        "plans/has-many.json",
    ]);
    assert_eq!(code, 0, "{outcome:#}");
    assert_eq!(
        fixture.read("crates/articles/src/article_service.rs"),
        service
    );

    let emitted: Value = serde_json::from_str(&fixture.read("plans/has-many.json")).unwrap();
    assert_eq!(emitted["name"], "comments into articles");
    assert_eq!(emitted["steps"].as_array().unwrap().len(), 4);

    // The emitted plan applies as-is.
    let (code, outcome) = fixture.run_json(&["integrate", "-f", "plans/has-many.json"]);
    assert_eq!(code, 0, "{outcome:#}");
    assert!(
        fixture
            .read("crates/articles/src/article_service.rs")
            .contains("delete_by_article")
    );
}

#[test]
fn snippets_reading_names_out_of_scope_are_refused() {
    let fixture = fixture();
    let at =
        json!({ "file": "posts/service", "fn": "PostService::tidy", "after": { "let": "first" } });
    let (code, outcome) = plan(&fixture, &insert(&at, "self.posts.touch(post_id);"), &[]);
    assert_eq!(code, 2);
    let error = error_of(&outcome);
    assert!(
        error.contains("reads `post_id`, which isn't in scope"),
        "{error}"
    );
    assert!(error.contains("in scope: first, id"), "{error}");

    // A later `let` isn't in scope yet; own bindings and callees are fine.
    let early = json!({ "file": "posts/service", "fn": "PostService::tidy", "start": true });
    let (code, _) = plan(&fixture, &insert(&early, "let n = first;"), &[]);
    assert_eq!(code, 2);
    let (code, outcome) = plan(&fixture, &insert(&early, "let n = id + 1;\nlog(n);"), &[]);
    assert_eq!(code, 0, "{outcome:#}");
}

#[test]
fn make_steps_run_generators_in_the_plan_transaction() {
    let fixture = generated(&[&["make:feature", "articles", "--field", "title:string"]]);
    let steps = json!([
        { "op": "make", "command": "make:dto",
          "args": { "feature": "articles", "name": "ArticleCountResponse", "kind": "response",
                    "fields": [{ "name": "count", "type": "bigint" }] } },
        { "op": "add_field", "file": "articles/dto/article_count_response",
          "struct": "ArticleCountResponse", "field": "pub drafts: i64" },
    ]);
    let (code, outcome) = plan(&fixture, &steps, &[]);
    assert_eq!(code, 0, "{outcome:#}");
    let dto = fixture.read("crates/articles/src/dto/article_count_response.rs");
    assert!(
        dto.contains("pub count: i64,\n    pub drafts: i64,"),
        "{dto}"
    );

    let bad = json!([{ "op": "make", "command": "make:nope", "args": {} }]);
    let (code, outcome) = plan(&fixture, &bad, &[]);
    assert_eq!(code, 2);
    assert!(
        error_of(&outcome).contains("`make:nope` isn't a generator"),
        "{outcome:#}"
    );
    let bad = json!([{ "op": "make", "command": "make:dto", "args": { "feature": "articles", "nope": 1 } }]);
    let (code, outcome) = plan(&fixture, &bad, &[]);
    assert_eq!(code, 2);
    assert!(
        error_of(&outcome).contains("`make:dto` arguments"),
        "{outcome:#}"
    );
}

#[test]
fn nest_routes_moves_the_prefix_and_documented_paths() {
    let fixture = generated(&[
        &["make:feature", "articles", "--field", "title:string"],
        &["make:feature", "comments", "--field", "body:text"],
    ]);
    let steps = json!([{ "op": "nest_routes", "module": "comments/module", "under": "articles/module", "param": "article_id" }]);
    let (code, outcome) = plan(&fixture, &steps, &[]);
    assert_eq!(code, 0, "{outcome:#}");
    assert!(
        fixture
            .read("crates/comments/src/comment_module.rs")
            .contains("prefix = \"/articles/{article_id}/comments\"")
    );
    let actions = fixture.read("crates/comments/src/comment_action.rs");
    assert_eq!(
        actions
            .matches("path = \"/articles/{article_id}/comments")
            .count(),
        5,
        "{actions}"
    );

    let routes = fixture.run(&["routes"]).stdout;
    assert!(
        routes.contains("GET     /articles/{article_id}/comments/{id}"),
        "{routes}"
    );

    let (code, outcome) = plan(&fixture, &steps, &[]);
    assert_eq!(code, 2);
    assert!(error_of(&outcome).contains("already nested"), "{outcome:#}");

    let (code, _) = fixture.run_json(&["undo"]);
    assert_eq!(code, 0);
    assert!(
        fixture
            .read("crates/comments/src/comment_module.rs")
            .contains("prefix = \"/comments\"")
    );
}

#[test]
fn hook_preset_calls_the_source_before_the_target_writes() {
    let fixture = generated(&[
        &["make:feature", "articles", "--field", "title:string"],
        &["make:feature", "audits", "--field", "note:string"],
    ]);
    for on in ["create", "delete"] {
        let var = format!("on={on}");
        let (code, outcome) = fixture.run_json(&[
            "integrate",
            "--preset",
            "hook",
            "--var",
            "source=audits",
            "--var",
            "target=articles",
            "--var",
            &var,
        ]);
        assert_eq!(code, 0, "{on}: {outcome:#}");
    }
    let service = fixture.read("crates/articles/src/article_service.rs");
    assert!(
        service.contains(".on_article_create(new.id)")
            && service.contains(".on_article_delete(id)"),
        "{service}"
    );
    // Injecting the same port twice is a no-op: the field and the `new`
    // parameter each appear once.
    assert_eq!(
        service.matches("audits: Arc<dyn AuditRepository>").count(),
        2,
        "{service}"
    );
    let repository = fixture.read("crates/audits/src/audit_repository.rs");
    assert_eq!(
        repository.matches("async fn on_article_create").count(),
        3,
        "{repository}"
    );
    assert_eq!(fixture.arch_rules(), Vec::<String>::new());

    let (code, outcome) = fixture.run_json(&[
        "integrate",
        "--preset",
        "hook",
        "--var",
        "source=audits",
        "--var",
        "target=articles",
        "--var",
        "on=patch",
    ]);
    assert_eq!(code, 2);
    assert!(
        error_of(&outcome).contains("use create, update or delete"),
        "{outcome:#}"
    );
}

fn port_plan(implementor: &str) -> Value {
    json!([{
        "op": "inject_port",
        "into": "articles/service",
        "service": "ArticleService",
        "port": "ArticleCleanup",
        "as": "cleanup",
        "implementor": implementor,
        "uses": ["uuid::Uuid"],
        "methods": [{
            "signature": "async fn delete_for_article(&self, article_id: Uuid) -> Result<u64, String>",
            "doc": ["Deletes whatever belongs to an article."],
            "body": "let _ = (&self.pool, article_id);\nOk(0)",
            "fake": "let _ = article_id;\nOk(0)",
        }],
    }])
}

/// `comments` already depends on `articles`, so `articles` can only reach
/// `comments` through a port it owns.
#[test]
fn inject_port_inverts_a_dependency_that_would_cycle() {
    let fixture = generated(&[
        &["make:feature", "articles", "--field", "title:string"],
        &[
            "make:feature",
            "comments",
            "--field",
            "body:text",
            "--field",
            "article_id:uuid",
        ],
    ]);
    fixture.replace(
        "crates/comments/Cargo.toml",
        "db.workspace = true\n",
        "articles.workspace = true\ndb.workspace = true\n",
    );
    fixture.replace(
        "rok.toml",
        "[project]",
        "[architecture]\nallow = [\"comments -> articles\"]\n\n[project]",
    );

    let direct = json!([{ "op": "inject_dependency", "into": "articles/service", "service": "ArticleService",
                          "dep": "comments::CommentRepository", "as": "comments" }]);
    let (code, outcome) = plan(&fixture, &direct, &[]);
    assert_eq!(code, 2);
    assert!(
        error_of(&outcome).contains("use an `inject_port` step"),
        "{outcome:#}"
    );

    let before = fixture.read("crates/articles/src/article_service.rs");
    let (code, outcome) = plan(&fixture, &port_plan("comments::PgCommentRepository"), &[]);
    assert_eq!(code, 0, "{outcome:#}");

    let port = fixture.read("crates/articles/src/article_cleanup.rs");
    assert!(
        port.contains("pub trait ArticleCleanup: Send + Sync {"),
        "{port}"
    );
    assert!(port.contains("pub struct FakeArticleCleanup {}"), "{port}");
    let service = fixture.read("crates/articles/src/article_service.rs");
    assert!(
        service.contains("cleanup: Arc<dyn ArticleCleanup>"),
        "{service}"
    );
    assert!(
        service.contains("Arc::new(crate::FakeArticleCleanup::default())"),
        "{service}"
    );
    let repository = fixture.read("crates/comments/src/comment_repository.rs");
    assert!(
        repository.contains("impl articles::ArticleCleanup for PgCommentRepository {"),
        "{repository}"
    );
    assert!(
        fixture
            .read("src/lib.rs")
            .contains("Arc::new(PgCommentRepository::new(pool.clone()))")
    );
    assert!(
        !fixture
            .read("crates/articles/Cargo.toml")
            .contains("comments"),
        "the target never depends on the source"
    );
    assert_eq!(fixture.arch_rules(), Vec::<String>::new());

    let (code, _) = fixture.run_json(&["unlink", "comments", "articles"]);
    assert_eq!(code, 0);
    assert_eq!(
        fixture.read("crates/articles/src/article_service.rs"),
        before
    );
    assert!(!fixture.exists("crates/articles/src/article_cleanup.rs"));
}

#[test]
fn inject_port_refuses_what_it_cannot_do() {
    let fixture = generated(&[
        &["make:feature", "articles", "--field", "title:string"],
        &["make:feature", "comments", "--field", "body:text"],
    ]);
    for (implementor, expected) in [
        ("PgCommentRepository", "write it as `<crate>::<Type>`"),
        (
            "articles::PgArticleRepository",
            "is the target crate itself",
        ),
        ("comments::NoSuchAdapter", "no `pub struct NoSuchAdapter`"),
    ] {
        let (code, outcome) = plan(&fixture, &port_plan(implementor), &[]);
        assert_eq!(code, 2, "{implementor}");
        assert!(
            error_of(&outcome).contains(expected),
            "{implementor}: {outcome:#}"
        );
    }
}
