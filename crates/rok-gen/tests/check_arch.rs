//! `cargo rok check arch`: the clean fixture produces nothing, and each
//! fixture that violates exactly one rule produces exactly that rule.

// Test harness code: panicking on a broken fixture is the point.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::Fixture;

fn rules(fixture: &Fixture) -> Vec<String> {
    fixture.arch_rules()
}

#[test]
fn clean_workspace_has_no_findings() {
    let fixture = Fixture::clean();
    let (code, outcome) = fixture.run_json(&["check", "arch"]);
    assert_eq!(code, 0, "{outcome:#}");
    assert_eq!(outcome["ok"], true);
    assert_eq!(outcome["data"]["gates"][0]["status"], "passed");
    assert_eq!(rules(&fixture), Vec::<String>::new());
}

#[test]
fn unsanctioned_feature_edge_is_flagged_at_the_dependency_line() {
    let fixture = Fixture::clean();
    fixture.write(
        "crates/comments/Cargo.toml",
        "[package]\nname = \"comments\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nposts = { path = \"../posts\" }\n",
    );

    let findings = fixture.arch_findings();
    assert_eq!(
        findings,
        vec![(
            "feature-edge".to_string(),
            "crates/comments/Cargo.toml".to_string(),
            7
        )]
    );

    let (code, _) = fixture.run_json(&["check", "arch"]);
    assert_eq!(code, 1, "a finding fails the gate");
}

#[test]
fn allowlisted_feature_edge_passes() {
    let fixture = Fixture::clean();
    fixture.write(
        "crates/comments/Cargo.toml",
        "[package]\nname = \"comments\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nposts = { path = \"../posts\" }\n",
    );
    fixture.replace(
        "rok.toml",
        "[project]",
        "[architecture]\nallow = [\"comments -> posts\"]\n\n[project]",
    );
    assert_eq!(rules(&fixture), Vec::<String>::new());
}

#[test]
fn workspace_dependency_edges_are_resolved() {
    let fixture = Fixture::clean();
    fixture.replace(
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/*\"]\n",
        "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.dependencies]\nposts = { path = \"crates/posts\" }\n",
    );
    fixture.write(
        "crates/comments/Cargo.toml",
        "[package]\nname = \"comments\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nposts.workspace = true\n",
    );
    assert_eq!(rules(&fixture), vec!["feature-edge"]);
}

#[test]
fn dev_dependency_edges_are_not_architecture() {
    let fixture = Fixture::clean();
    fixture.write(
        "crates/comments/Cargo.toml",
        "[package]\nname = \"comments\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dev-dependencies]\nposts = { path = \"../posts\" }\n",
    );
    assert_eq!(rules(&fixture), Vec::<String>::new());
}

#[test]
fn dependency_cycle_is_flagged() {
    let fixture = Fixture::clean();
    fixture.replace(
        "rok.toml",
        "[project]",
        "[architecture]\nallow = [\"comments -> posts\", \"posts -> comments\"]\n\n[project]",
    );
    fixture.write(
        "crates/comments/Cargo.toml",
        "[package]\nname = \"comments\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nposts = { path = \"../posts\" }\n",
    );
    fixture.replace(
        "crates/posts/Cargo.toml",
        "edition = \"2024\"\n",
        "edition = \"2024\"\n\n[dependencies]\ncomments = { path = \"../comments\" }\n",
    );
    assert_eq!(rules(&fixture), vec!["cycle"]);
}

#[test]
fn sqlx_in_a_service_is_flagged() {
    let fixture = Fixture::clean();
    fixture.replace(
        "crates/posts/src/post_service.rs",
        "use std::sync::Arc;",
        "use std::sync::Arc;\n\nuse sqlx::PgPool;",
    );
    let findings = fixture.arch_findings();
    assert_eq!(
        findings,
        vec![(
            "service-purity".to_string(),
            "crates/posts/src/post_service.rs".to_string(),
            3
        )]
    );
}

#[test]
fn adapter_constructed_outside_the_composition_root_is_flagged() {
    let fixture = Fixture::clean();
    fixture.replace(
        "crates/posts/src/post_service.rs",
        "#[cfg(test)]",
        "pub fn build() -> crate::post_repository::PgPostRepository {\n    crate::post_repository::PgPostRepository::new()\n}\n\n#[cfg(test)]",
    );
    assert_eq!(rules(&fixture), vec!["adapter-construction"]);
}

#[test]
fn adapter_struct_literal_outside_the_composition_root_is_flagged() {
    let fixture = Fixture::clean();
    fixture.replace(
        "crates/router/src/lib.rs",
        "pub struct AppModule;",
        "pub struct AppModule;\n\npub fn sneaky() -> posts::PgPostRepository {\n    posts::PgPostRepository {}\n}",
    );
    assert_eq!(rules(&fixture), vec!["adapter-construction"]);
}

#[test]
fn handler_without_openapi_annotation_is_flagged() {
    let fixture = Fixture::clean();
    fixture.replace(
        "crates/posts/src/post_action.rs",
        "#[utoipa::path(get, path = \"/posts\")]\n",
        "",
    );
    assert_eq!(rules(&fixture), vec!["handler-openapi"]);
}

#[test]
fn handler_missing_from_every_module_is_flagged() {
    let fixture = Fixture::clean();
    fixture.replace(
        "crates/posts/src/post_module.rs",
        "controllers = [list_posts, create_post]",
        "controllers = [create_post]",
    );
    assert_eq!(rules(&fixture), vec!["handler-registered"]);
}

#[test]
fn protected_handlers_count_as_registered() {
    let fixture = Fixture::clean();
    fixture.replace(
        "crates/posts/src/post_module.rs",
        "controllers = [list_posts, create_post])]",
        "controllers = [list_posts], protected = [create_post], middleware = require_auth)]",
    );
    assert_eq!(rules(&fixture), Vec::<String>::new());
}

#[test]
fn handler_missing_from_apidoc_is_flagged() {
    let fixture = Fixture::clean();
    fixture.replace(
        "crates/router/src/openapi.rs",
        "        posts::post_action::list_posts,\n",
        "",
    );
    let findings = fixture.arch_findings();
    assert_eq!(
        findings,
        vec![(
            "handler-apidoc".to_string(),
            "crates/posts/src/post_action.rs".to_string(),
            8
        )]
    );
}

#[test]
fn configured_apidoc_file_is_the_only_one_read() {
    let fixture = Fixture::clean();
    fixture.replace(
        "rok.toml",
        "composition_root = \"src\"",
        "composition_root = \"src\"\napidoc = \"crates/router/src/elsewhere.rs\"",
    );
    assert_eq!(rules(&fixture), vec!["handler-apidoc"]);
}

#[test]
fn request_dto_without_validate_is_flagged() {
    let fixture = Fixture::clean();
    fixture.replace(
        "crates/posts/src/dto/create_post.rs",
        "Deserialize, Validate, ToSchema",
        "Deserialize, ToSchema",
    );
    assert_eq!(rules(&fixture), vec!["dto-derives"]);
}

#[test]
fn response_dto_only_needs_to_schema() {
    let fixture = Fixture::clean();
    fixture.write(
        "crates/posts/src/dto/post_response.rs",
        "#[derive(serde::Serialize, utoipa::ToSchema)]\npub struct PostResponse {\n    pub id: u64,\n}\n",
    );
    assert_eq!(rules(&fixture), Vec::<String>::new());

    fixture.write(
        "crates/posts/src/dto/post_response.rs",
        "#[derive(serde::Serialize)]\npub struct PostResponse {\n    pub id: u64,\n}\n",
    );
    assert_eq!(rules(&fixture), vec!["dto-derives"]);
}

#[test]
fn unvalidated_json_body_is_flagged() {
    let fixture = Fixture::clean();
    fixture.replace(
        "crates/posts/src/post_action.rs",
        "body: ValidatedJson<CreatePostRequest>",
        "body: actix_web::web::Json<CreatePostRequest>",
    );
    assert_eq!(rules(&fixture), vec!["validated-json"]);
}

#[test]
fn file_whose_main_type_does_not_match_its_name_is_flagged() {
    let fixture = Fixture::clean();
    fixture.replace(
        "crates/posts/src/post_service.rs",
        "pub struct PostService {",
        "pub struct PostsService {",
    );
    fixture.replace(
        "crates/posts/src/lib.rs",
        "pub use post_service::PostService;\n",
        "",
    );
    assert_eq!(rules(&fixture), vec!["naming"]);
}

#[test]
fn unparsable_file_is_reported_not_fatal() {
    let fixture = Fixture::clean();
    fixture.write("crates/posts/src/broken.rs", "pub fn oops( {\n");
    assert_eq!(rules(&fixture), vec!["parse"]);
}

#[test]
fn feature_list_in_config_overrides_inference() {
    let fixture = Fixture::clean();
    // `comments` is no longer a feature crate, so depending on it is fine.
    fixture.replace(
        "rok.toml",
        "[project]",
        "[architecture]\nfeatures = [\"posts\"]\n\n[project]",
    );
    fixture.replace(
        "crates/posts/Cargo.toml",
        "edition = \"2024\"\n",
        "edition = \"2024\"\n\n[dependencies]\ncomments = { path = \"../comments\" }\n",
    );
    assert_eq!(rules(&fixture), Vec::<String>::new());

    fixture.replace(
        "rok.toml",
        "features = [\"posts\"]",
        "features = [\"nope\"]",
    );
    let (code, outcome) = fixture.run_json(&["check", "arch"]);
    assert_eq!(code, 1);
    assert!(
        outcome["data"]["gates"][0]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("`nope`")),
        "{outcome:#}"
    );
}

#[test]
fn ci_mode_emits_github_annotations() {
    let fixture = Fixture::clean();
    fixture.replace(
        "crates/posts/src/post_action.rs",
        "#[utoipa::path(get, path = \"/posts\")]\n",
        "",
    );
    let execution = fixture.run(&["check", "arch", "--ci"]);
    assert_eq!(execution.code, 1);
    assert!(
        execution.stdout.contains(
            "::error file=crates/posts/src/post_action.rs,line=7,col=14,title=rok handler-openapi::"
        ),
        "{}",
        execution.stdout
    );
}

/// The reference project (actix-api-starter) must pass untouched. Opt-in,
/// since it lives in another repository: set `ROK_REFERENCE_PROJECT` to its
/// path.
#[test]
fn reference_project_has_no_findings() {
    let Some(reference) = std::env::var_os("ROK_REFERENCE_PROJECT") else {
        return;
    };
    let execution = rok_gen::run(
        ["check", "arch"],
        std::path::Path::new(&reference),
        &mut &b""[..],
    );
    assert_eq!(
        execution.code, 0,
        "{}{}",
        execution.stdout, execution.stderr
    );
}
