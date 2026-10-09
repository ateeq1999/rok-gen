//! The `make:*` generators: what they create, how they wire it in, that
//! the result passes the architecture checker, and that a failure leaves
//! the project untouched.

// Test harness code: panicking on a broken fixture is the point.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Fixture;
use serde_json::Value;

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap().to_string())
        .collect()
}

fn make_articles(fixture: &Fixture, extra: &[&str]) -> (i32, Value) {
    let mut args = vec![
        "make:feature",
        "articles",
        "--field",
        "title:string",
        "--field",
        "views:int",
        "--field",
        "summary:string?",
    ];
    args.extend_from_slice(extra);
    fixture.run_json(&args)
}

#[test]
fn make_feature_creates_the_crate_and_wires_every_shared_file() {
    let fixture = Fixture::app();
    let (code, outcome) = make_articles(&fixture, &[]);
    assert_eq!(code, 0, "{outcome:#}");

    let created = strings(&outcome["files_created"]);
    for file in [
        "crates/articles/Cargo.toml",
        "crates/articles/src/lib.rs",
        "crates/articles/src/article_action.rs",
        "crates/articles/src/article_entity.rs",
        "crates/articles/src/article_module.rs",
        "crates/articles/src/article_repository.rs",
        "crates/articles/src/article_service.rs",
        "crates/articles/src/dto/mod.rs",
        "crates/articles/src/dto/create_article.rs",
        "crates/articles/src/dto/update_article.rs",
        "crates/error/src/articles.rs",
        "crates/db/migrations/0002_create_articles.sql",
    ] {
        assert!(
            created.contains(&file.to_string()),
            "{file} not in {created:?}"
        );
        assert!(fixture.exists(file), "{file} wasn't written");
    }
    assert_eq!(
        strings(&outcome["files_changed"]),
        [
            "Cargo.toml",
            "crates/error/src/lib.rs",
            "crates/router/Cargo.toml",
            "crates/router/src/lib.rs",
            "crates/router/src/openapi.rs",
            "src/lib.rs",
        ]
    );

    let root_manifest = fixture.read("Cargo.toml");
    assert!(root_manifest.contains(
        "[workspace.dependencies]\narticles = { path = \"crates/articles\" }\ncomments = "
    ));
    assert!(root_manifest.contains("[dependencies]\narticles.workspace = true\nposts.workspace"));
    assert!(
        fixture
            .read("crates/router/Cargo.toml")
            .contains("articles.workspace = true")
    );
    assert!(fixture.read("crates/router/src/lib.rs").contains(
        "imports = [posts::PostModule, comments::CommentModule, articles::ArticleModule]"
    ));
    let apidoc = fixture.read("crates/router/src/openapi.rs");
    for entry in [
        "articles::article_action::list_articles,",
        "articles::article_action::delete_article,",
        "envelope::ApiResponse<Vec<articles::Article>>",
        "(name = \"Articles\", description = \"Articles CRUD\")",
    ] {
        assert!(apidoc.contains(entry), "{entry} missing:\n{apidoc}");
    }
    let errors = fixture.read("crates/error/src/lib.rs");
    assert!(errors.contains("mod articles;\nmod posts;"), "{errors}");
    assert!(
        errors.contains("pub use articles::ArticlesError;"),
        "{errors}"
    );

    let root = fixture.read("src/lib.rs");
    assert!(
        root.contains("use articles::{ArticleRepository, ArticleService, PgArticleRepository};"),
        "{root}"
    );
    assert!(
        root.contains(
            "    let post_service = PostService::new(post_repository);\n\n    let article_repository: Arc<dyn ArticleRepository> =\n        Arc::new(PgArticleRepository::new(pool.clone()));\n    let article_service = ArticleService::new(article_repository);\n"
        ),
        "{root}"
    );
    assert!(
        root.contains(".app_data(web::Data::new(post_service.clone()))\n            .app_data(web::Data::new(article_service.clone()))"),
        "{root}"
    );

    let migration = fixture.read("crates/db/migrations/0002_create_articles.sql");
    assert!(
        migration
            .contains("    title TEXT NOT NULL,\n    views INTEGER NOT NULL,\n    summary TEXT,\n"),
        "{migration}"
    );
}

#[test]
fn generated_code_passes_the_architecture_checker() {
    let fixture = Fixture::app();
    assert_eq!(make_articles(&fixture, &[]).0, 0);
    assert_eq!(
        fixture
            .run_json(&[
                "make:feature",
                "tags",
                "--field",
                "label:string",
                "--guard",
                "none"
            ])
            .0,
        0
    );
    assert_eq!(
        fixture
            .run_json(&[
                "make:dto",
                "articles",
                "ArticleSummaryResponse",
                "--kind",
                "response",
                "--field",
                "title:string"
            ])
            .0,
        0
    );
    assert_eq!(
        fixture
            .run_json(&[
                "make:action",
                "articles",
                "publish_article",
                "--method",
                "post",
                "--path",
                "/{id}/publish",
                "--body",
                "PublishArticleRequest",
                "--guard",
                "admin",
            ])
            .0,
        0
    );
    assert_eq!(fixture.arch_rules(), Vec::<String>::new());
}

#[test]
fn guards_shape_the_module_and_handlers() {
    let fixture = Fixture::app();
    fixture.run_json(&[
        "make:feature",
        "tags",
        "--field",
        "label:string",
        "--guard",
        "none",
    ]);
    let module = fixture.read("crates/tags/src/tag_module.rs");
    assert!(!module.contains("middleware"), "{module}");
    assert!(!fixture.read("crates/tags/Cargo.toml").contains("session"));

    fixture.run_json(&[
        "make:feature",
        "reports",
        "--field",
        "score:float",
        "--guard",
        "admin",
    ]);
    assert!(
        fixture
            .read("crates/reports/src/report_module.rs")
            .contains("middleware = require_auth")
    );
    let actions = fixture.read("crates/reports/src/report_action.rs");
    assert_eq!(actions.matches("_admin: AdminUser").count(), 5, "{actions}");
}

#[test]
fn dry_run_shows_the_diff_and_writes_nothing() {
    let fixture = Fixture::app();
    let before = fixture.read("Cargo.toml");
    let (code, outcome) = make_articles(&fixture, &["--dry-run"]);
    assert_eq!(code, 0, "{outcome:#}");
    let diff = outcome["data"]["diff"].as_str().unwrap();
    assert!(diff.contains("+++ b/crates/articles/src/lib.rs"), "{diff}");
    assert!(
        diff.contains("+articles = { path = \"crates/articles\" }"),
        "{diff}"
    );
    assert!(!fixture.exists("crates/articles"));
    assert_eq!(fixture.read("Cargo.toml"), before);

    let human = fixture.run(&[
        "make:feature",
        "articles",
        "--field",
        "title:string",
        "--dry-run",
    ]);
    assert!(
        human.stdout.ends_with("dry run: nothing was written\n"),
        "{}",
        human.stdout
    );
}

#[test]
fn the_payload_can_come_from_json() {
    let fixture = Fixture::app();
    fixture.write(
        "articles.yaml",
        "name: articles\nguard: none\nfields:\n  - { name: title, type: string }\n  - { name: rating, type: float, optional: true }\n",
    );
    let (code, outcome) = fixture.run_json(&["make:feature", "-f", "articles.yaml"]);
    assert_eq!(code, 0, "{outcome:#}");
    let entity = fixture.read("crates/articles/src/article_entity.rs");
    assert!(entity.contains("pub rating: Option<f64>,"), "{entity}");
}

#[test]
fn invalid_requests_are_refused_with_a_reason() {
    let fixture = Fixture::app();
    for (args, expected) in [
        (
            vec!["make:feature", "Articles", "--field", "a:string"],
            "must be snake_case",
        ),
        (vec!["make:feature", "articles"], "at least one field"),
        (
            vec!["make:feature", "articles", "--field", "id:uuid"],
            "generated automatically",
        ),
        (
            vec!["make:feature", "articles", "--field", "type:string"],
            "Rust keyword",
        ),
        (
            vec![
                "make:feature",
                "articles",
                "--field",
                "a:HashMap<String, u8>",
            ],
            "no Postgres type",
        ),
        (
            vec!["make:feature", "articles", "--field", "nocolon"],
            "expected `name:type`",
        ),
        (
            vec!["make:feature", "posts", "--field", "a:string"],
            "already exists",
        ),
        (vec!["make:dto", "posts", "lower"], "must be PascalCase"),
        (
            vec!["make:action", "posts", "x", "--path", "no-slash"],
            "must be empty or start with `/`",
        ),
        (vec!["make:action", "nope", "x"], "doesn't exist"),
    ] {
        let execution = fixture.run(&args);
        assert_eq!(execution.code, 2, "{args:?}: {}", execution.stdout);
        assert!(
            execution.stderr.contains(expected),
            "{args:?}: {}",
            execution.stderr
        );
    }
    assert!(!fixture.exists("crates/articles"));
}

#[test]
fn a_missing_anchor_aborts_before_anything_is_written() {
    let fixture = Fixture::app();
    fixture.write("crates/router/src/lib.rs", "pub struct AppModule;\n");
    let before = fixture.read("Cargo.toml");

    let execution = fixture.run(&["make:feature", "articles", "--field", "title:string"]);
    assert_eq!(execution.code, 2);
    assert!(
        execution.stderr.contains("no #[module(...)]"),
        "{}",
        execution.stderr
    );
    assert!(!fixture.exists("crates/articles"));
    assert!(!fixture.exists("crates/error/src/articles.rs"));
    assert_eq!(fixture.read("Cargo.toml"), before);
}

#[test]
fn make_dto_exports_and_documents_the_type() {
    let fixture = Fixture::app();
    let (code, outcome) = fixture.run_json(&[
        "make:dto",
        "posts",
        "PostSummaryResponse",
        "--kind",
        "response",
        "--field",
        "title:string",
        "--field",
        "words:int",
    ]);
    assert_eq!(code, 0, "{outcome:#}");
    let dto = fixture.read("crates/posts/src/dto/post_summary_response.rs");
    assert!(
        dto.contains("#[derive(Clone, Debug, Serialize, ToSchema)]"),
        "{dto}"
    );
    assert!(dto.contains("pub words: i32,"), "{dto}");
    let dto_mod = fixture.read("crates/posts/src/dto/mod.rs");
    assert!(dto_mod.contains("mod post_summary_response;"), "{dto_mod}");
    assert!(
        dto_mod.contains("pub use post_summary_response::PostSummaryResponse;"),
        "{dto_mod}"
    );
    assert!(
        fixture
            .read("crates/posts/src/lib.rs")
            .contains("pub use dto::PostSummaryResponse;")
    );
    assert!(
        fixture
            .read("crates/router/src/openapi.rs")
            .contains("posts::PostSummaryResponse")
    );

    let (code, _) = fixture.run_json(&["make:dto", "posts", "PostSummaryResponse"]);
    assert_eq!(code, 2, "a second identical DTO is refused");
}

#[test]
fn make_dto_request_types_validate_strings() {
    let fixture = Fixture::app();
    fixture.run_json(&[
        "make:dto",
        "posts",
        "RenamePostRequest",
        "--field",
        "title:string",
    ]);
    let dto = fixture.read("crates/posts/src/dto/rename_post.rs");
    assert!(dto.contains("Deserialize, Validate, ToSchema"), "{dto}");
    assert!(dto.contains("#[validate(length(min = 1"), "{dto}");
}

#[test]
fn make_action_adds_a_registered_documented_handler() {
    let fixture = Fixture::app();
    let (code, outcome) = fixture.run_json(&[
        "make:action",
        "posts",
        "publish_post",
        "--method",
        "post",
        "--path",
        "/{id}/publish",
        "--body",
        "PublishPostRequest",
    ]);
    assert_eq!(code, 0, "{outcome:#}");

    let actions = fixture.read("crates/posts/src/post_action.rs");
    assert!(
        actions.contains("#[post(\"/{id}/publish\")]\npub async fn publish_post("),
        "{actions}"
    );
    assert!(
        actions.contains("path = \"/posts/{id}/publish\""),
        "{actions}"
    );
    assert!(actions.contains("_path: web::Path<Uuid>"), "{actions}");
    assert!(
        actions.contains("_body: ValidatedJson<PublishPostRequest>"),
        "{actions}"
    );
    assert!(
        actions.contains("use crate::dto::PublishPostRequest;"),
        "{actions}"
    );
    assert!(actions.contains("use uuid::Uuid;"), "{actions}");

    let module = fixture.read("crates/posts/src/post_module.rs");
    assert!(
        module.contains("controllers = [list_posts, create_post, publish_post]"),
        "{module}"
    );
    assert!(
        module.contains("use crate::post_action::publish_post;"),
        "{module}"
    );
    assert!(
        fixture.exists("crates/posts/src/dto/publish_post.rs"),
        "the body DTO is generated"
    );
    assert!(
        fixture
            .read("crates/router/src/openapi.rs")
            .contains("posts::post_action::publish_post,")
    );

    let (code, _) = fixture.run_json(&["make:action", "posts", "publish_post"]);
    assert_eq!(code, 2, "an existing handler name is refused");
}

#[test]
fn make_migration_numbers_sequentially() {
    let fixture = Fixture::app();
    let (code, outcome) = fixture.run_json(&[
        "make:migration",
        "add_post_slug",
        "--sql",
        "ALTER TABLE posts ADD COLUMN slug TEXT;",
    ]);
    assert_eq!(code, 0, "{outcome:#}");
    assert_eq!(
        strings(&outcome["files_created"]),
        ["crates/db/migrations/0002_add_post_slug.sql"]
    );
    assert_eq!(
        fixture.read("crates/db/migrations/0002_add_post_slug.sql"),
        "ALTER TABLE posts ADD COLUMN slug TEXT;\n"
    );
    let (_, outcome) = fixture.run_json(&["make:migration", "add_post_index"]);
    assert_eq!(
        strings(&outcome["files_created"]),
        ["crates/db/migrations/0003_add_post_index.sql"]
    );
}

#[test]
fn generator_schemas_are_published() {
    let fixture = Fixture::empty();
    for name in ["make:feature", "make:action", "make:dto", "make:migration"] {
        let execution = fixture.run(&["schema", name]);
        assert_eq!(execution.code, 0, "{name}: {}", execution.stderr);
    }
}

/// End to end on the real reference project: generate, `--verify` with
/// `cargo check`, then the fmt/lint/arch gates and the generated tests.
/// Opt-in (it compiles the whole workspace): set `ROK_REFERENCE_PROJECT`
/// and `ROK_E2E=1`; point `CARGO_TARGET_DIR` at a warm target dir to make
/// it fast.
#[test]
fn reference_project_end_to_end() {
    let (Some(reference), Some(_)) = (
        std::env::var_os("ROK_REFERENCE_PROJECT"),
        std::env::var_os("ROK_E2E"),
    ) else {
        return;
    };
    let fixture = Fixture::copy_of(std::path::Path::new(&reference));
    let (code, outcome) = fixture.run_json(&[
        "make:feature",
        "articles",
        "--field",
        "title:string",
        "--field",
        "body:text",
        "--field",
        "rating:float?",
        "--field",
        "author_id:uuid",
        "--verify",
    ]);
    assert_eq!(code, 0, "{outcome:#}");
    let (code, outcome) = fixture.run_json(&[
        "make:action",
        "articles",
        "publish_article",
        "--method",
        "post",
        "--path",
        "/{id}/publish",
        "--body",
        "PublishArticleRequest",
        "--guard",
        "admin",
        "--verify",
    ]);
    assert_eq!(code, 0, "{outcome:#}");
    let more: [&[&str]; 3] = [
        &[
            "make:repository",
            "articles",
            "revision",
            "--field",
            "body:text",
            "--field",
            "meta:json?",
            "--verify",
        ],
        &[
            "make:service",
            "articles",
            "publishing",
            "--repository",
            "article",
            "--repository",
            "revision",
            "--verify",
        ],
        &["make:error", "audits", "--verify"],
    ];
    for args in more {
        let (code, outcome) = fixture.run_json(args);
        assert_eq!(code, 0, "{args:?}: {outcome:#}");
    }

    let (code, outcome) = fixture.run_json(&["check", "fmt", "lint", "arch", "--keep-going"]);
    assert_eq!(code, 0, "{outcome:#}");
    let tests = std::process::Command::new("cargo")
        .args(["test", "-p", "articles", "--quiet"])
        .current_dir(fixture.root())
        .output()
        .unwrap();
    assert!(
        tests.status.success(),
        "{}",
        String::from_utf8_lossy(&tests.stderr)
    );
}
