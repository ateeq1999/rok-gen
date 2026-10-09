//! Phase 0 spike: proves each atom-engine gap the spec names is worked
//! around in rok-render, against the pinned atom-engine version.
//!
//! These test `rok-render`'s public API directly because no CLI command
//! renders templates yet; once `make:*` lands, the same guarantees are
//! asserted through the CLI seam and these become redundant.

use rok_render::{RenderError, Renderer, SyntaxKind, Trust};
use serde_json::json;

// By value so call sites can pass `json!(...)` literals directly.
#[allow(clippy::needless_pass_by_value)]
fn render(source: &str, context: serde_json::Value) -> Result<String, RenderError> {
    let mut renderer = Renderer::new(Trust::BuiltIn);
    renderer.add_template("t.rs", source)?;
    renderer.render("t.rs", &context)
}

#[test]
fn nothing_is_html_escaped_even_for_html_named_templates() {
    let payload = json!({ "s": "<T> & \"quoted\" 'single' a/b" });
    for name in ["service.rs", "email.html", "feed.xml", "page.htm"] {
        let mut renderer = Renderer::new(Trust::BuiltIn);
        renderer.add_template(name, "{{ s }}").unwrap();
        assert_eq!(
            renderer.render(name, &payload).unwrap(),
            "<T> & \"quoted\" 'single' a/b",
            "{name}"
        );
    }
}

#[test]
fn naming_filters() {
    let out = render(
        "{{ n | snake }} {{ n | pascal }} {{ n | camel }} {{ n | kebab }} {{ f | singular }} {{ e | plural }}",
        json!({ "n": "PostComment", "f": "posts", "e": "category" }),
    )
    .unwrap();
    assert_eq!(
        out,
        "post_comment PostComment postComment post-comment post categories"
    );
}

#[test]
fn type_filters() {
    let out = render(
        "{{ a | rust_type }}|{{ b | rust_type }}|{{ c | rust_type }}|{{ a | sql_type }}|{{ c | sql_type }}|{{ d | sql_type }}",
        json!({ "a": "uuid", "b": "Vec<String>", "c": "datetime", "d": "Option<i64>" }),
    )
    .unwrap();
    assert_eq!(
        out,
        "Uuid|Vec<String>|DateTime<Utc>|UUID|TIMESTAMPTZ|BIGINT"
    );

    let err = render("{{ x | sql_type }}", json!({ "x": "HashMap<String, u8>" })).unwrap_err();
    assert!(err.to_string().contains("no Postgres type"), "{err}");
}

#[test]
fn rust_ident_escapes_keywords_and_rejects_the_unusable() {
    let out = render(
        "{{ a | rust_ident }} {{ b | rust_ident }}",
        json!({ "a": "type", "b": "title" }),
    )
    .unwrap();
    assert_eq!(out, "r#type title");

    for bad in ["self", "1st", "has-dash"] {
        assert!(
            render("{{ a | rust_ident }}", json!({ "a": bad })).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn braces_avoid_delimiter_collisions() {
    let out = render(
        "use crate::dto::{{ names | braces }};\nlet body = {{ expr | braces }};",
        json!({ "names": ["CreatePostRequest", "PostResponse"], "expr": "x + 1" }),
    )
    .unwrap();
    assert_eq!(
        out,
        "use crate::dto::{CreatePostRequest, PostResponse};\nlet body = {x + 1};"
    );
}

#[test]
fn imports_are_collected_deduplicated_and_grouped() {
    let source = "\
{{ imports() }}

pub struct {{ name }}Service {
{{ import(path=\"std::sync::Arc\") }}
    repo: Arc<dyn {{ name }}Repository>,
}
{{ import(path=\"crate::post_repository::PostRepository\") }}
{{ import(path=\"uuid::Uuid\") }}{{ import(path=\"std::sync::Arc\") }}
";
    let out = render(source, json!({ "name": "Post" })).unwrap();
    assert_eq!(
        out,
        "\
use std::sync::Arc;

use uuid::Uuid;

use crate::post_repository::PostRepository;

pub struct PostService {
    repo: Arc<dyn PostRepository>,
}
"
    );
}

#[test]
fn imports_without_a_placeholder_go_on_top() {
    let out = render(
        "{{ import(path=\"std::sync::Arc\") }}fn f() {}\n",
        json!({}),
    )
    .unwrap();
    assert_eq!(out, "use std::sync::Arc;\n\nfn f() {}\n");
}

#[test]
fn whitespace_control_works() {
    let out = render(
        "fields:\n{%- for f in fields %}\n  {{ f }}\n{%- endfor %}\n",
        json!({ "fields": ["a", "b"] }),
    )
    .unwrap();
    assert_eq!(out, "fields:\n  a\n  b\n");
}

#[test]
fn template_packs_cannot_read_the_environment_or_filesystem() {
    let mut pack = Renderer::new(Trust::Pack);
    pack.add_template("leak.rs", "const K: &str = \"{{ env(name=\"PATH\") }}\";")
        .unwrap();
    let err = pack.render("leak.rs", &json!({})).unwrap_err();
    assert!(
        err.to_string()
            .contains("disabled for third-party template packs"),
        "{err}"
    );

    pack.add_template("probe.rs", "{{ file_exists(path=\"Cargo.toml\") }}")
        .unwrap();
    assert!(pack.render("probe.rs", &json!({})).is_err());

    // Built-in templates keep them.
    let mut builtin = Renderer::new(Trust::BuiltIn);
    builtin
        .add_template(
            "ok.rs",
            "{{ file_exists(path=\"definitely-missing-file\") }}",
        )
        .unwrap();
    assert_eq!(builtin.render("ok.rs", &json!({})).unwrap(), "false");
}

#[test]
fn rendered_rust_is_checked_against_its_syntax_kind() {
    let mut renderer = Renderer::new(Trust::BuiltIn);
    renderer
        .add_template("stmt.rs", "self.{{ field }}.delete_by_post(id).await?;")
        .unwrap();
    renderer
        .add_template("field.rs", "pub {{ field }}: Arc<dyn {{ trait_name }}>")
        .unwrap();
    let context = json!({ "field": "comments", "trait_name": "CommentRepository" });

    assert_eq!(
        renderer
            .render_rust("stmt.rs", &context, SyntaxKind::Stmt)
            .unwrap(),
        "self.comments.delete_by_post(id).await?;"
    );
    renderer
        .render_rust("field.rs", &context, SyntaxKind::Field)
        .unwrap();

    let err = renderer
        .render_rust("stmt.rs", &context, SyntaxKind::Item)
        .unwrap_err();
    assert!(
        matches!(
            err,
            RenderError::Syntax {
                kind: SyntaxKind::Item,
                ..
            }
        ),
        "{err}"
    );
}

#[test]
fn errors_name_the_template_and_the_cause() {
    let err = render("{{ missing }}", json!({})).unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("`t.rs`") && message.contains("missing"),
        "{message}"
    );

    let mut renderer = Renderer::new(Trust::BuiltIn);
    let err = renderer.add_template("bad.rs", "{% if %}").unwrap_err();
    assert!(matches!(err, RenderError::Template { .. }), "{err}");
}
