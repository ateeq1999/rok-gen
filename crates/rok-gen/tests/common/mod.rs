//! A throwaway rok-managed workspace on disk, driven through
//! [`rok_gen::run`], the CLI's single test seam.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use serde_json::Value;
use tempfile::TempDir;

pub struct Fixture {
    dir: TempDir,
}

impl Fixture {
    /// An empty directory (not a rok project).
    pub fn empty() -> Self {
        Self {
            dir: tempfile::tempdir().expect("temp dir"),
        }
    }

    /// A small workspace that satisfies every architecture rule: two
    /// feature crates (`posts`, `comments`), a `router` holding `ApiDoc`,
    /// and a root crate acting as the composition root.
    pub fn clean() -> Self {
        let fixture = Self::empty();
        for (path, content) in CLEAN_FILES {
            fixture.write(path, content);
        }
        fixture
    }

    /// [`Fixture::clean`] shaped like a full app, for the generators: a
    /// workspace-dependency manifest, an `error` crate, migrations, and a
    /// composition root that builds `PostService` and registers it.
    pub fn app() -> Self {
        let fixture = Self::clean();
        for (path, content) in APP_FILES {
            fixture.write(path, content);
        }
        fixture
    }

    /// A copy of `source` (without `target/`, `.git/` or `.env`).
    pub fn copy_of(source: &Path) -> Self {
        fn copy(from: &Path, to: &Path) {
            std::fs::create_dir_all(to).unwrap();
            for entry in std::fs::read_dir(from).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name();
                if matches!(name.to_str(), Some("target" | ".git" | ".env")) {
                    continue;
                }
                let path = entry.path();
                if path.is_dir() {
                    copy(&path, &to.join(&name));
                } else {
                    std::fs::copy(&path, to.join(&name)).unwrap();
                }
            }
        }
        let fixture = Self::empty();
        copy(source, fixture.root());
        fixture
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.path(rel).exists()
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    pub fn write(&self, rel: &str, content: &str) {
        let path = self.path(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, content).expect("write");
    }

    pub fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path(rel)).expect("read")
    }

    pub fn remove(&self, rel: &str) {
        std::fs::remove_file(self.path(rel)).expect("remove");
    }

    /// Rewrites one file, failing loudly if `from` isn't in it (so a fixture
    /// edit can never silently do nothing).
    pub fn replace(&self, rel: &str, from: &str, to: &str) {
        let text = self.read(rel);
        assert!(text.contains(from), "`{from}` not found in {rel}");
        self.write(rel, &text.replacen(from, to, 1));
    }

    pub fn run(&self, args: &[&str]) -> rok_gen::Execution {
        run_at(self.root(), args, "")
    }

    pub fn run_with_stdin(&self, args: &[&str], stdin: &str) -> rok_gen::Execution {
        run_at(self.root(), args, stdin)
    }

    /// Runs from `subdir` (relative to the fixture root).
    pub fn run_in(&self, subdir: &str, args: &[&str]) -> rok_gen::Execution {
        run_at(&self.path(subdir), args, "")
    }

    /// Runs with `--output json` and parses stdout.
    pub fn run_json(&self, args: &[&str]) -> (i32, Value) {
        let mut full = vec!["--output", "json"];
        full.extend_from_slice(args);
        let execution = self.run(&full);
        let value = serde_json::from_str(&execution.stdout).unwrap_or_else(|err| {
            panic!(
                "stdout isn't JSON ({err}):\n{}\nstderr:\n{}",
                execution.stdout, execution.stderr
            )
        });
        (execution.code, value)
    }

    /// `(rule, file, line)` for every architecture finding.
    pub fn arch_findings(&self) -> Vec<(String, String, u64)> {
        let (_, outcome) = self.run_json(&["check", "arch"]);
        outcome["data"]["gates"]
            .as_array()
            .expect("gates")
            .iter()
            .flat_map(|gate| gate["findings"].as_array().cloned().unwrap_or_default())
            .map(|finding| {
                (
                    finding["rule"].as_str().expect("rule").to_string(),
                    finding["file"].as_str().expect("file").to_string(),
                    finding["line"].as_u64().expect("line"),
                )
            })
            .collect()
    }

    /// The distinct rule ids that fired.
    pub fn arch_rules(&self) -> Vec<String> {
        let mut rules: Vec<String> = self
            .arch_findings()
            .into_iter()
            .map(|(rule, ..)| rule)
            .collect();
        rules.sort();
        rules.dedup();
        rules
    }
}

const CLEAN_FILES: &[(&str, &str)] = &[
    (
        "rok.toml",
        r#"[rok]
cli = "^0.1"
templates = "0.1.0"

[project]
composition_root = "src"
"#,
    ),
    (
        "Cargo.toml",
        r#"[workspace]
members = ["crates/*"]

[package]
name = "app"
version = "0.1.0"
edition = "2024"

[dependencies]
posts = { path = "crates/posts" }
router = { path = "crates/router" }
"#,
    ),
    (
        "src/lib.rs",
        "use posts::PgPostRepository;

pub fn run() {
    let _repository = PgPostRepository::new();
}
",
    ),
    (
        "crates/posts/Cargo.toml",
        r#"[package]
name = "posts"
version = "0.1.0"
edition = "2024"
"#,
    ),
    (
        "crates/posts/src/lib.rs",
        "pub mod post_action;
mod dto;
mod post_module;
mod post_repository;
mod post_service;

pub use dto::CreatePostRequest;
pub use post_module::PostModule;
pub use post_repository::{PgPostRepository, PostRepository};
pub use post_service::PostService;
",
    ),
    (
        "crates/posts/src/post_module.rs",
        r#"use macros::module;

use crate::post_action::{create_post, list_posts};

#[module(prefix = "/posts", controllers = [list_posts, create_post])]
pub struct PostModule;
"#,
    ),
    (
        "crates/posts/src/post_action.rs",
        r#"use actix_web::{get, post};
use validation::ValidatedJson;

use crate::dto::CreatePostRequest;

#[utoipa::path(get, path = "/posts")]
#[get("")]
pub async fn list_posts() -> &'static str {
    "[]"
}

#[utoipa::path(post, path = "/posts")]
#[post("")]
pub async fn create_post(body: ValidatedJson<CreatePostRequest>) -> &'static str {
    let _ = body;
    "ok"
}
"#,
    ),
    (
        "crates/posts/src/post_service.rs",
        "use std::sync::Arc;

use crate::post_repository::PostRepository;

pub struct PostService {
    posts: Arc<dyn PostRepository>,
}

#[cfg(test)]
mod tests {
    use crate::post_repository::PgPostRepository;

    #[test]
    fn tests_may_construct_adapters() {
        let _repository = PgPostRepository::new();
    }
}
",
    ),
    (
        "crates/posts/src/post_repository.rs",
        "pub trait PostRepository {}

pub struct PgPostRepository;

impl PgPostRepository {
    pub fn new() -> Self {
        Self
    }
}

impl PostRepository for PgPostRepository {}
",
    ),
    (
        "crates/posts/src/dto/mod.rs",
        "mod create_post;

pub use create_post::CreatePostRequest;
",
    ),
    (
        "crates/posts/src/dto/create_post.rs",
        "use serde::Deserialize;
use utoipa::ToSchema;
use validator::Validate;

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreatePostRequest {
    pub title: String,
}
",
    ),
    (
        "crates/comments/Cargo.toml",
        r#"[package]
name = "comments"
version = "0.1.0"
edition = "2024"
"#,
    ),
    (
        "crates/comments/src/lib.rs",
        "mod comment_module;

pub use comment_module::CommentModule;
",
    ),
    (
        "crates/comments/src/comment_module.rs",
        r#"use macros::module;

#[module(prefix = "/comments")]
pub struct CommentModule;
"#,
    ),
    (
        "crates/router/Cargo.toml",
        r#"[package]
name = "router"
version = "0.1.0"
edition = "2024"

[dependencies]
comments = { path = "../comments" }
posts = { path = "../posts" }
"#,
    ),
    (
        "crates/router/src/lib.rs",
        "mod openapi;

pub use openapi::ApiDoc;

use macros::module;

#[module(imports = [posts::PostModule, comments::CommentModule])]
pub struct AppModule;
",
    ),
    (
        "crates/router/src/openapi.rs",
        r#"use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        posts::post_action::list_posts,
        posts::post_action::create_post,
    ),
    components(schemas(posts::CreatePostRequest)),
    tags((name = "Posts", description = "Blog posts")),
    modifiers(&SecurityAddon),
)]
pub struct ApiDoc;
"#,
    ),
];

fn run_at(cwd: &Path, args: &[&str], stdin: &str) -> rok_gen::Execution {
    rok_gen::run(args.iter().copied(), cwd, &mut stdin.as_bytes())
}

const APP_FILES: &[(&str, &str)] = &[
    (
        "Cargo.toml",
        r#"[workspace]
members = ["crates/*"]

[workspace.dependencies]
comments = { path = "crates/comments" }
error = { path = "crates/error" }
posts = { path = "crates/posts" }
router = { path = "crates/router" }

actix-web = "4"

[package]
name = "app"
version = "0.1.0"
edition = "2024"

[dependencies]
posts.workspace = true
router.workspace = true

actix-web.workspace = true
"#,
    ),
    (
        "src/lib.rs",
        r#"//! Composition root.

use std::sync::Arc;

use actix_web::{App, HttpServer, web};
use posts::{PgPostRepository, PostRepository, PostService};

pub async fn run() -> std::io::Result<()> {
    let pool = db::connect();

    let post_repository: Arc<dyn PostRepository> = Arc::new(PgPostRepository::new(pool.clone()));
    let post_service = PostService::new(post_repository);

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(post_service.clone()))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
"#,
    ),
    (
        "crates/error/Cargo.toml",
        "[package]\nname = \"error\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    ),
    (
        "crates/error/src/lib.rs",
        "//! Every error type.\n\nmod posts;\n\npub use posts::PostsError;\n",
    ),
    ("crates/error/src/posts.rs", "pub enum PostsError {}\n"),
    (
        "crates/db/migrations/0001_create_posts.sql",
        "CREATE TABLE posts (id UUID PRIMARY KEY);\n",
    ),
];
