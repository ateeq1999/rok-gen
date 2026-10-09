//! Template rendering for rok-gen, on top of atom-engine.
//!
//! atom-engine is used as-is (pinned exactly); every gap it has for
//! generating Rust is worked around here rather than patched upstream:
//!
//! - **HTML auto-escaping.** atom-engine builds on `Tera::default()`, which
//!   auto-escapes templates whose names end in `.html`/`.htm`/`.xml`, with
//!   no way to turn it off. Every template is registered under an internal
//!   name ending in `.atom`, so escaping never applies, whatever the
//!   template is called.
//! - **Import collection.** atom-engine's `push`/`stack` don't round-trip
//!   (`stack` reads a key off a plain string and always yields ""), so
//!   imports use their own `import(path=...)`/`imports()` pair, resolved in
//!   a post-processing pass.
//! - **Brace collisions.** `use a::{{{ x }}};` can't be written in Tera
//!   syntax; the `braces` filter wraps a value (or joins a list) in `{}`.
//! - **Untrusted packs.** `env()` and `file_exists()` are always registered
//!   by atom-engine; for [`Trust::Pack`] they're replaced by functions that
//!   fail, so a third-party template can't copy secrets into generated code.
//! - **Lost render errors.** atom-engine stringifies Tera's error without its
//!   cause chain, leaving only "Failed to render 'x'". On failure the
//!   template is rendered again on a plain Tera with the same templates,
//!   rok filters and context, and that error's full chain is reported.
//!
//! Rendered Rust can be checked against the syntax kind it's meant to be
//! ([`SyntaxKind`]) before anyone writes it to disk.

mod filters;
/// Naming and type helpers, shared with the template filters of the same names.
pub mod naming {
    pub use crate::filters::{plural, rust_ident, rust_type, singular, sql_type};
}
mod imports;

use std::collections::HashMap;

use atom_engine::Atom;
use serde_json::{Map, Value};

/// Suffix of every internal template name. Must never end in a suffix Tera
/// auto-escapes.
const INTERNAL_SUFFIX: &str = ".atom";

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("template `{name}` doesn't parse: {message}")]
    Template { name: String, message: String },
    #[error("template `{name}` failed to render: {message}")]
    Render { name: String, message: String },
    #[error(
        "template `{name}` rendered invalid Rust for a {kind}: {message}\n--- output ---\n{output}"
    )]
    Syntax {
        name: String,
        kind: SyntaxKind,
        message: String,
        output: String,
    },
}

/// Where templates come from, which decides what they may call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    /// rok-gen's own embedded templates and the project's ejected overrides.
    BuiltIn,
    /// Third-party template packs: no environment or filesystem access.
    Pack,
}

/// The Rust syntax a rendered snippet must parse as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxKind {
    File,
    Item,
    Stmt,
    Expr,
    /// A named struct field, e.g. `pub comments: Arc<dyn CommentRepository>`.
    Field,
}

impl std::fmt::Display for SyntaxKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::File => "file",
            Self::Item => "item",
            Self::Stmt => "statement",
            Self::Expr => "expression",
            Self::Field => "field",
        })
    }
}

/// Somewhere rok filters and functions can be registered: the atom-engine
/// renderer, and the plain Tera used to diagnose its failures.
pub(crate) trait Registry {
    fn filter<F: tera::Filter + 'static>(&mut self, name: &str, filter: F);
    fn function<F: tera::Function + 'static>(&mut self, name: &str, function: F);
}

impl Registry for Atom {
    fn filter<F: tera::Filter + 'static>(&mut self, name: &str, filter: F) {
        self.register_filter(name, filter);
    }

    fn function<F: tera::Function + 'static>(&mut self, name: &str, function: F) {
        self.register_function(name, function);
    }
}

impl Registry for tera::Tera {
    fn filter<F: tera::Filter + 'static>(&mut self, name: &str, filter: F) {
        self.register_filter(name, filter);
    }

    fn function<F: tera::Function + 'static>(&mut self, name: &str, function: F) {
        self.register_function(name, function);
    }
}

/// A template set plus the rok filters and functions.
pub struct Renderer {
    atom: Atom,
    trust: Trust,
    /// Every template's (internal name, source), kept for diagnostics.
    sources: Vec<(String, String)>,
    /// Every provided global, kept for diagnostics.
    provided: Map<String, Value>,
}

impl Renderer {
    #[must_use]
    pub fn new(trust: Trust) -> Self {
        let mut atom = Atom::new();
        register_rok(&mut atom, trust);
        Self {
            atom,
            trust,
            sources: Vec::new(),
            provided: Map::new(),
        }
    }

    /// Registers a template under `name` (any name; see the crate docs on
    /// escaping).
    ///
    /// # Errors
    ///
    /// [`RenderError::Template`] if the source doesn't parse.
    pub fn add_template(&mut self, name: &str, source: &str) -> Result<(), RenderError> {
        let internal = internal_name(name);
        self.atom
            .add_template(&internal, source)
            .map_err(|err| RenderError::Template {
                name: name.to_string(),
                message: err.to_string(),
            })?;
        self.sources.retain(|(existing, _)| *existing != internal);
        self.sources.push((internal, source.to_string()));
        Ok(())
    }

    /// Makes `value` available to every template as `key`.
    pub fn provide(&mut self, key: &str, value: Value) {
        self.provided.insert(key.to_string(), value.clone());
        self.atom.provide(key, value);
    }

    /// Renders `name` with `context` (a JSON object) and resolves imports.
    ///
    /// # Errors
    ///
    /// [`RenderError::Render`] for a missing template, a missing variable,
    /// or a failing filter or function, with the underlying cause.
    pub fn render(&self, name: &str, context: &Value) -> Result<String, RenderError> {
        let raw = self
            .atom
            .render(&internal_name(name), context)
            .map_err(|err| RenderError::Render {
                name: name.to_string(),
                message: self
                    .diagnose(&internal_name(name), context)
                    .unwrap_or_else(|| err.to_string()),
            })?;
        Ok(imports::resolve(&raw))
    }

    /// Re-renders on a plain Tera to recover the error chain atom-engine
    /// drops. `None` if the plain render fails to reproduce the failure
    /// (e.g. the template relies on an atom-only built-in).
    fn diagnose(&self, internal: &str, context: &Value) -> Option<String> {
        let mut tera = tera::Tera::default();
        register_rok(&mut tera, self.trust);
        tera.add_raw_templates(
            self.sources
                .iter()
                .map(|(name, source)| (name.as_str(), source.as_str())),
        )
        .ok()?;
        let mut tera_context = tera::Context::from_serialize(context).ok()?;
        for (key, value) in &self.provided {
            tera_context.insert(key, value);
        }
        let err = tera.render(internal, &tera_context).err()?;
        let mut chain = vec![err.to_string()];
        let mut source = std::error::Error::source(&err);
        while let Some(cause) = source {
            chain.push(cause.to_string());
            source = cause.source();
        }
        Some(chain.join(": "))
    }

    /// [`Renderer::render`], then requires the output to parse as `kind`.
    ///
    /// # Errors
    ///
    /// Anything `render` returns, or [`RenderError::Syntax`] with the
    /// offending output.
    pub fn render_rust(
        &self,
        name: &str,
        context: &Value,
        kind: SyntaxKind,
    ) -> Result<String, RenderError> {
        let output = self.render(name, context)?;
        let parsed = match kind {
            SyntaxKind::File => syn::parse_file(&output).map(drop),
            SyntaxKind::Item => syn::parse_str::<syn::Item>(&output).map(drop),
            SyntaxKind::Stmt => syn::parse_str::<syn::Stmt>(&output).map(drop),
            SyntaxKind::Expr => syn::parse_str::<syn::Expr>(&output).map(drop),
            SyntaxKind::Field => {
                syn::parse::Parser::parse_str(syn::Field::parse_named, &output).map(drop)
            }
        };
        parsed
            .map(|()| output.clone())
            .map_err(|err| RenderError::Syntax {
                name: name.to_string(),
                kind,
                message: err.to_string(),
                output,
            })
    }
}

fn internal_name(name: &str) -> String {
    format!("{name}{INTERNAL_SUFFIX}")
}

fn register_rok(engine: &mut impl Registry, trust: Trust) {
    filters::register(engine);
    imports::register(engine);
    if trust == Trust::Pack {
        for name in ["env", "file_exists"] {
            engine.function(name, move |_: &HashMap<String, Value>| {
                Err(tera::Error::msg(format!(
                    "`{name}()` is disabled for third-party template packs"
                )))
            });
        }
    }
}
