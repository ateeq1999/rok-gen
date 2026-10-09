//! Structural locations in Rust source, as byte offsets.
//!
//! Files are parsed with `syn` (with `proc-macro2`'s `span-locations`), and
//! every anchor this crate finds is turned into an offset in the original
//! text, so edits insert text there and never re-print the syntax tree:
//! comments and formatting outside an edit survive untouched.
//!
//! Offsets refer to `\n`-normalized text; `rok-edit` normalizes line endings
//! before locating and restores them on write.

mod list;

use proc_macro2::{LineColumn, Span};
use syn::parse::{ParseStream, Parser};
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Attribute, Expr, Item, ItemFn, Meta, MetaNameValue, Stmt, Token};

pub use list::ListSite;

#[derive(Debug, thiserror::Error)]
pub enum LocateError {
    #[error("{what} doesn't parse at {line}:{column}: {message}")]
    Parse {
        what: String,
        line: usize,
        column: usize,
        message: String,
    },
    #[error("{0}")]
    NotFound(String),
}

/// One text insertion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub offset: usize,
    pub insert: String,
}

impl Edit {
    #[must_use]
    pub fn new(offset: usize, insert: impl Into<String>) -> Self {
        Self {
            offset,
            insert: insert.into(),
        }
    }
}

/// Applies `edits` to `text`. Edits at the same offset keep their given
/// order.
#[must_use]
pub fn apply(text: &str, mut edits: Vec<Edit>) -> String {
    edits.sort_by_key(|edit| edit.offset);
    let extra: usize = edits.iter().map(|edit| edit.insert.len()).sum();
    let mut output = String::with_capacity(text.len() + extra);
    let mut cursor = 0;
    for edit in edits {
        let offset = edit.offset.clamp(cursor, text.len());
        output.push_str(&text[cursor..offset]);
        output.push_str(&edit.insert);
        cursor = offset;
    }
    output.push_str(&text[cursor..]);
    output
}

/// A parsed file plus the line index needed to turn spans into offsets.
pub struct Source {
    text: String,
    line_starts: Vec<usize>,
    pub file: syn::File,
}

impl Source {
    /// Parses `text` (`what` names it in errors, e.g. its path).
    ///
    /// # Errors
    ///
    /// [`LocateError::Parse`] when `text` isn't valid Rust.
    pub fn parse(what: &str, text: &str) -> Result<Self, LocateError> {
        let file = syn::parse_file(text).map_err(|err| {
            let start = err.span().start();
            LocateError::Parse {
                what: what.to_string(),
                line: start.line,
                column: start.column + 1,
                message: err.to_string(),
            }
        })?;
        let line_starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(index, _)| index + 1))
            .collect();
        Ok(Self {
            text: text.to_string(),
            line_starts,
            file,
        })
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Byte offset of a line/column (`column` counts chars).
    #[must_use]
    pub fn offset(&self, position: LineColumn) -> usize {
        let Some(&line_start) = self.line_starts.get(position.line.saturating_sub(1)) else {
            return self.text.len();
        };
        let line = &self.text[line_start..];
        line_start
            + line
                .char_indices()
                .nth(position.column)
                .map_or(line.len(), |(index, _)| index)
    }

    #[must_use]
    pub fn start(&self, span: Span) -> usize {
        self.offset(span.start())
    }

    #[must_use]
    pub fn end(&self, span: Span) -> usize {
        self.offset(span.end())
    }

    /// The source text of any syntax node.
    #[must_use]
    pub fn snippet(&self, node: &impl Spanned) -> &str {
        let span = node.span();
        let (start, end) = (self.start(span), self.end(span));
        self.text.get(start..end).unwrap_or_default()
    }

    /// Whether a top-level item named `name` exists (fn, struct, enum,
    /// trait, mod, type, const or static).
    #[must_use]
    pub fn has_item(&self, name: &str) -> bool {
        self.file
            .items
            .iter()
            .any(|item| item_name(item).is_some_and(|ident| ident == name))
    }

    /// End offset of the last top-level item matching `pred`.
    pub fn last_item_end(&self, pred: impl Fn(&Item) -> bool) -> Option<usize> {
        self.file
            .items
            .iter()
            .filter(|item| pred(item))
            .map(|item| self.end(item.span()))
            .max()
    }

    /// The list `key = [...]` inside the first `#[module(...)]` attribute.
    ///
    /// `Ok(None)` when the attribute has no such key; then
    /// [`Source::module_args_end`] is where to add it.
    ///
    /// # Errors
    ///
    /// [`LocateError::NotFound`] when there's no `#[module]` at all, or
    /// [`LocateError::Parse`] when its arguments aren't `key = value` pairs.
    pub fn module_list(&self, key: &str) -> Result<Option<ListSite>, LocateError> {
        let attr = self.module_attr()?;
        let args = attr
            .parse_args_with(
                syn::punctuated::Punctuated::<MetaNameValue, Token![,]>::parse_terminated,
            )
            .map_err(|err| attr_error("#[module(...)]", &err))?;
        for arg in args {
            if arg.path.is_ident(key)
                && let Expr::Array(array) = &arg.value
            {
                let delim = array.bracket_token.span;
                return Ok(Some(ListSite::new(
                    &self.text,
                    self.end(delim.open()),
                    self.start(delim.close()),
                )));
            }
        }
        Ok(None)
    }

    /// Offset of the closing `)` of the first `#[module(...)]`.
    ///
    /// # Errors
    ///
    /// [`LocateError::NotFound`] when there's no `#[module(...)]`.
    pub fn module_args_end(&self) -> Result<usize, LocateError> {
        let attr = self.module_attr()?;
        match &attr.meta {
            Meta::List(list) => Ok(self.start(list.delimiter.span().close())),
            _ => Err(LocateError::NotFound(
                "#[module] has no argument list".to_string(),
            )),
        }
    }

    /// Every `key = value` of the first `#[module(...)]`, as `(key, value
    /// source text)`.
    ///
    /// # Errors
    ///
    /// [`LocateError::NotFound`] or [`LocateError::Parse`].
    pub fn module_args(&self) -> Result<Vec<(String, String)>, LocateError> {
        let attr = self.module_attr()?;
        let args = attr
            .parse_args_with(
                syn::punctuated::Punctuated::<MetaNameValue, Token![,]>::parse_terminated,
            )
            .map_err(|err| attr_error("#[module(...)]", &err))?;
        Ok(args
            .iter()
            .map(|arg| {
                let key = arg
                    .path
                    .get_ident()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                (key, self.snippet(&arg.value).to_string())
            })
            .collect())
    }

    /// Every name the file's top-level `use` items bring into scope (leaf
    /// names and renames; globs are ignored).
    #[must_use]
    pub fn imported_names(&self) -> std::collections::BTreeSet<String> {
        fn walk(tree: &syn::UseTree, names: &mut std::collections::BTreeSet<String>) {
            match tree {
                syn::UseTree::Path(path) => walk(&path.tree, names),
                syn::UseTree::Name(name) => {
                    names.insert(name.ident.to_string());
                }
                syn::UseTree::Rename(rename) => {
                    names.insert(rename.rename.to_string());
                }
                syn::UseTree::Group(group) => {
                    for tree in &group.items {
                        walk(tree, names);
                    }
                }
                syn::UseTree::Glob(_) => {}
            }
        }
        let mut names = std::collections::BTreeSet::new();
        for item in &self.file.items {
            if let Item::Use(item) = item {
                walk(&item.tree, &mut names);
            }
        }
        names
    }

    fn module_attr(&self) -> Result<&Attribute, LocateError> {
        self.struct_attrs()
            .find(|attr| {
                attr.path()
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == "module")
            })
            .ok_or_else(|| {
                LocateError::NotFound("no #[module(...)] struct in this file".to_string())
            })
    }

    fn struct_attrs(&self) -> impl Iterator<Item = &Attribute> {
        self.file
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Struct(item) => Some(&item.attrs),
                _ => None,
            })
            .flatten()
    }

    /// A nested list inside `#[openapi(...)]`, e.g. `["paths"]` or
    /// `["components", "schemas"]`.
    ///
    /// # Errors
    ///
    /// [`LocateError::NotFound`] when there's no `#[openapi]` or no such
    /// nested list, [`LocateError::Parse`] when it can't be read.
    pub fn openapi_list(&self, path: &[&str]) -> Result<ListSite, LocateError> {
        let attr = self
            .struct_attrs()
            .find(|attr| attr.path().is_ident("openapi"))
            .ok_or_else(|| LocateError::NotFound("no #[openapi(...)] in this file".to_string()))?;
        let found = attr
            .parse_args_with(|input: ParseStream<'_>| nested_list(input, path))
            .map_err(|err| attr_error("#[openapi(...)]", &err))?;
        let (open, close) = found.ok_or_else(|| {
            LocateError::NotFound(format!("#[openapi(...)] has no `{}(...)`", path.join("(")))
        })?;
        Ok(ListSite::new(&self.text, self.end(open), self.start(close)))
    }

    /// The top-level `fn name`.
    ///
    /// # Errors
    ///
    /// [`LocateError::NotFound`].
    pub fn function(&self, name: &str) -> Result<&ItemFn, LocateError> {
        self.file
            .items
            .iter()
            .find_map(|item| match item {
                Item::Fn(item) if item.sig.ident == name => Some(item),
                _ => None,
            })
            .ok_or_else(|| LocateError::NotFound(format!("no top-level `fn {name}`")))
    }

    /// End offset of the last top-level statement of `function` that
    /// matches `pred` (given the statement and its source text).
    pub fn last_stmt_end(
        &self,
        function: &ItemFn,
        pred: impl Fn(&Stmt, &str) -> bool,
    ) -> Option<usize> {
        function
            .block
            .stmts
            .iter()
            .filter(|stmt| pred(stmt, self.snippet(*stmt)))
            .map(|stmt| self.end(stmt.span()))
            .max()
    }

    /// Start offset of the first top-level statement of `function` whose
    /// source text contains `needle`.
    #[must_use]
    pub fn first_stmt_start_containing(&self, function: &ItemFn, needle: &str) -> Option<usize> {
        function
            .block
            .stmts
            .iter()
            .find(|stmt| self.snippet(*stmt).contains(needle))
            .map(|stmt| self.start(stmt.span()))
    }

    /// End offsets (just past the `)`) of every `.method(...)` call inside
    /// `function` whose arguments' text contains `needle`, in source order.
    #[must_use]
    pub fn method_calls(&self, function: &ItemFn, method: &str, needle: &str) -> Vec<usize> {
        let mut calls = MethodCalls {
            source: self,
            method,
            needle,
            found: Vec::new(),
        };
        calls.visit_block(&function.block);
        calls.found.sort_unstable();
        calls.found
    }
}

struct MethodCalls<'a> {
    source: &'a Source,
    method: &'a str,
    needle: &'a str,
    found: Vec<usize>,
}

impl<'ast> Visit<'ast> for MethodCalls<'_> {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == self.method
            && call
                .args
                .iter()
                .any(|arg| self.source.snippet(arg).contains(self.needle))
        {
            self.found
                .push(self.source.end(call.paren_token.span.close()));
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

fn attr_error(what: &str, err: &syn::Error) -> LocateError {
    let start = err.span().start();
    LocateError::Parse {
        what: what.to_string(),
        line: start.line,
        column: start.column + 1,
        message: err.to_string(),
    }
}

const fn item_name(item: &Item) -> Option<&syn::Ident> {
    match item {
        Item::Fn(item) => Some(&item.sig.ident),
        Item::Struct(item) => Some(&item.ident),
        Item::Enum(item) => Some(&item.ident),
        Item::Trait(item) => Some(&item.ident),
        Item::Mod(item) => Some(&item.ident),
        Item::Type(item) => Some(&item.ident),
        Item::Const(item) => Some(&item.ident),
        Item::Static(item) => Some(&item.ident),
        _ => None,
    }
}

/// Walks `a, b(...), c = expr` meta items, descending into the parenthesized
/// group named by `path[0]`, and returns the open/close paren spans of the
/// innermost one.
fn nested_list(input: ParseStream<'_>, path: &[&str]) -> syn::Result<Option<(Span, Span)>> {
    let mut found = None;
    while !input.is_empty() {
        let name = syn::Path::parse_mod_style(input)?;
        if input.peek(syn::token::Paren) {
            let content;
            let paren = syn::parenthesized!(content in input);
            if found.is_none() && name.is_ident(path[0]) {
                if path.len() == 1 {
                    found = Some((paren.span.open(), paren.span.close()));
                    content.parse::<proc_macro2::TokenStream>()?;
                } else {
                    found = nested_list(&content, &path[1..])?;
                }
            } else {
                content.parse::<proc_macro2::TokenStream>()?;
            }
        } else if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            input.parse::<Expr>()?;
        }
        if input.is_empty() {
            break;
        }
        input.parse::<Token![,]>()?;
    }
    Ok(found)
}

/// Checks a snippet parses as a sequence of statements (what gets inserted
/// into a function body).
///
/// # Errors
///
/// The `syn` error message.
pub fn check_stmts(snippet: &str) -> Result<(), String> {
    syn::Block::parse_within
        .parse_str(snippet)
        .map(drop)
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROUTER: &str = r"use macros::module;

#[module(
    controllers = [hello, health],
    imports = [auth::AuthModule, users::UserModule],
    default_service = not_found
)]
pub struct AppModule;
";

    const APIDOC: &str = r#"#[derive(OpenApi)]
#[openapi(
    paths(
        auth::login_action::login,
        users::user_action::me,
    ),
    components(schemas(
        auth::LoginRequest,
        envelope::ApiResponse<Vec<users::PublicUser>>,
    )),
    tags(
        (name = "Auth", description = "Login and registration"),
    ),
    modifiers(&SecurityAddon),
)]
pub struct ApiDoc;
"#;

    #[test]
    fn module_lists_are_found_and_appended() {
        let source = Source::parse("router", ROUTER).unwrap();
        let imports = source.module_list("imports").unwrap().unwrap();
        assert_eq!(imports.items, ["auth::AuthModule", "users::UserModule"]);
        let edit = imports.append(&["posts::PostModule".to_string()]).unwrap();
        assert!(
            apply(ROUTER, vec![edit])
                .contains("imports = [auth::AuthModule, users::UserModule, posts::PostModule],")
        );
        assert!(source.module_list("protected").unwrap().is_none());
        assert_eq!(&ROUTER[source.module_args_end().unwrap()..][..2], ")]");
    }

    #[test]
    fn openapi_nested_lists_are_found_and_appended() {
        let source = Source::parse("apidoc", APIDOC).unwrap();
        let paths = source.openapi_list(&["paths"]).unwrap();
        assert_eq!(paths.items.len(), 2);
        let schemas = source.openapi_list(&["components", "schemas"]).unwrap();
        assert_eq!(
            schemas.items,
            [
                "auth::LoginRequest",
                "envelope::ApiResponse<Vec<users::PublicUser>>"
            ]
        );
        let tags = source.openapi_list(&["tags"]).unwrap();
        assert_eq!(tags.items.len(), 1);

        let edits = vec![
            paths
                .append(&["posts::post_action::list_posts".to_string()])
                .unwrap(),
            schemas.append(&["posts::Post".to_string()]).unwrap(),
        ];
        let updated = apply(APIDOC, edits);
        assert!(
            updated.contains(
                "        users::user_action::me,\n        posts::post_action::list_posts,\n    ),"
            ),
            "{updated}"
        );
        assert!(
            updated.contains("        posts::Post,\n    )),"),
            "{updated}"
        );
        assert!(source.openapi_list(&["nope"]).is_err());
    }

    #[test]
    fn statements_and_method_calls_are_located() {
        let text = "fn run() {\n    let a_service = AService::new(x);\n    let other = 1;\n    let app = App::new().app_data(web::Data::new(a_service.clone())).app_data(web::Data::new(b));\n}\n";
        let source = Source::parse("root", text).unwrap();
        let run = source.function("run").unwrap();
        let end = source
            .last_stmt_end(run, |_, text| text.contains("Service::new("))
            .unwrap();
        assert_eq!(
            &text[..end],
            "fn run() {\n    let a_service = AService::new(x);"
        );
        let calls = source.method_calls(run, "app_data", "_service");
        assert_eq!(calls.len(), 1);
        assert!(text[..calls[0]].ends_with("a_service.clone()))"));
        assert!(
            source
                .first_stmt_start_containing(run, "App::new")
                .is_some()
        );
    }

    #[test]
    fn offsets_count_chars_not_bytes() {
        let text = "// héllo wörld\nfn ünïcode() {}\n";
        let source = Source::parse("u", text).unwrap();
        let end = source.last_item_end(|_| true).unwrap();
        assert_eq!(&text[..end], "// héllo wörld\nfn ünïcode() {}");
    }
}
