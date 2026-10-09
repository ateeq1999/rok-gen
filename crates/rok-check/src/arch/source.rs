//! Source-level rules, from parsing every `src/**/*.rs` file with `syn`:
//! `service-purity`, `adapter-construction`, `handler-openapi`,
//! `handler-registered`, `handler-apidoc`, `dto-derives`, `validated-json`,
//! `naming`, and `parse` for files that don't parse.
//!
//! Anything under `#[cfg(test)]` is ignored: tests may construct adapters
//! and fakes freely.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use heck::ToUpperCamelCase;
use proc_macro2::Span;
use rok_plan::project::Project;
use syn::punctuated::Punctuated;
use syn::visit::{self, Visit};
use syn::{Attribute, Expr, FnArg, ItemFn, Meta, MetaNameValue, Token, Type};

use crate::finding::{Finding, Rule};
use crate::workspace::{Workspace, relative};

/// actix-web's route attribute macros.
const ROUTE_MACROS: [&str; 10] = [
    "get", "post", "put", "patch", "delete", "head", "options", "trace", "connect", "route",
];

/// One parsed source file.
struct Source {
    member: String,
    is_feature: bool,
    rel: String,
    /// Module path inside its crate: `src/dto/login.rs` → `["dto", "login"]`.
    module_path: Vec<String>,
    ast: syn::File,
}

impl Source {
    fn stem(&self) -> &str {
        self.rel
            .rsplit('/')
            .next()
            .and_then(|name| name.strip_suffix(".rs"))
            .unwrap_or_default()
    }
}

pub fn check(
    workspace: &Workspace,
    features: &BTreeSet<String>,
    project: &Project,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let sources = parse_sources(workspace, features, &project.root, &mut findings);

    let adapters = adapter_types(&sources);
    let composition_root = format!(
        "{}/",
        project.config.project.composition_root.trim_matches('/')
    );
    let apidoc_file = project.config.project.apidoc.as_deref();

    let mut facts = BTreeMap::new();
    for source in &sources {
        let mut collector = Collector::new(&adapters);
        collector.visit_file(&source.ast);
        facts.insert(source.rel.as_str(), collector);
    }

    let mut registered: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    let mut apidoc_paths: Option<BTreeSet<Vec<String>>> = None;
    for source in &sources {
        let collected = &facts[source.rel.as_str()];
        registered
            .entry(source.member.as_str())
            .or_default()
            .extend(collected.module_entries.iter().cloned());
        let apidoc_here = apidoc_file.is_none_or(|file| file == source.rel);
        if apidoc_here && collected.declares_openapi {
            apidoc_paths
                .get_or_insert_default()
                .extend(collected.openapi_paths.iter().cloned());
        }
        for (span, message) in &collected.attribute_errors {
            findings.push(finding(source, *span, Rule::Parse, message.clone()));
        }
    }

    for source in &sources {
        let collected = &facts[source.rel.as_str()];

        let in_root = source.rel.starts_with(&composition_root);
        for (name, span) in &collected.constructions {
            let declared_here = adapters.get(name).is_some_and(|file| *file == source.rel);
            if !in_root && !declared_here {
                findings.push(finding(
                    source,
                    *span,
                    Rule::AdapterConstruction,
                    format!(
                        "`{name}` is a concrete adapter; construct it only in the composition \
                         root (`{composition_root}`) and inject it as `Arc<dyn Trait>`"
                    ),
                ));
            }
        }

        if !source.is_feature {
            continue;
        }

        if source.stem().ends_with("_service") {
            for span in &collected.sqlx_uses {
                findings.push(finding(
                    source,
                    *span,
                    Rule::ServicePurity,
                    "services depend on repository traits only; move this `sqlx` usage into the \
                     repository's Postgres adapter"
                        .to_string(),
                ));
            }
        }

        check_handlers(
            source,
            collected,
            registered.get(source.member.as_str()),
            apidoc_paths.as_ref(),
            &mut findings,
        );
        check_dtos(source, collected, &mut findings);
        check_naming(source, collected, &mut findings);
    }

    findings
}

fn check_handlers(
    source: &Source,
    collected: &Collector<'_>,
    registered: Option<&BTreeSet<String>>,
    apidoc_paths: Option<&BTreeSet<Vec<String>>>,
    findings: &mut Vec<Finding>,
) {
    let crate_ident = source.member.replace('-', "_");
    for handler in &collected.handlers {
        let name = &handler.name;
        if !handler.has_openapi {
            findings.push(finding(
                source,
                handler.span,
                Rule::HandlerOpenapi,
                format!("handler `{name}` has no #[utoipa::path(...)], so it's missing from the API docs"),
            ));
        }
        if !registered.is_some_and(|names| names.contains(name)) {
            findings.push(finding(
                source,
                handler.span,
                Rule::HandlerRegistered,
                format!(
                    "handler `{name}` isn't listed in any #[module(controllers/protected = [...])] \
                     in `{}`, so it's unreachable",
                    source.member
                ),
            ));
        }
        let mut path = vec![crate_ident.clone()];
        path.extend(source.module_path.iter().cloned());
        path.push(name.clone());
        let message = match apidoc_paths {
            None => Some(
                "no #[openapi(paths(...))] found in the workspace to document this handler"
                    .to_string(),
            ),
            Some(paths) if !paths.contains(&path) => Some(format!(
                "`{}` is missing from ApiDoc's paths(...), so it's missing from the OpenAPI spec",
                path.join("::")
            )),
            Some(_) => None,
        };
        if let Some(message) = message {
            findings.push(finding(source, handler.span, Rule::HandlerApidoc, message));
        }
        for span in &handler.json_params {
            findings.push(finding(
                source,
                *span,
                Rule::ValidatedJson,
                format!(
                    "handler `{name}` takes an unvalidated `Json<T>` body; use \
                     `validation::ValidatedJson<T>`"
                ),
            ));
        }
    }
}

fn check_dtos(source: &Source, collected: &Collector<'_>, findings: &mut Vec<Finding>) {
    let in_dto = source.module_path.len() >= 2 && source.module_path[0] == "dto";
    if !in_dto {
        return;
    }
    for item in &collected.types {
        let derives = |name: &str| item.derives.contains(name);
        let mut required = Vec::new();
        if derives("Deserialize") || derives("Serialize") {
            required.push("ToSchema");
        }
        if derives("Deserialize") && item.is_struct {
            required.push("Validate");
        }
        let missing: Vec<&str> = required.into_iter().filter(|name| !derives(name)).collect();
        if !missing.is_empty() {
            findings.push(finding(
                source,
                item.span,
                Rule::DtoDerives,
                format!(
                    "DTO `{}` must also derive {}",
                    item.name,
                    missing.join(" and ")
                ),
            ));
        }
    }
}

fn check_naming(source: &Source, collected: &Collector<'_>, findings: &mut Vec<Finding>) {
    let stem = source.stem();
    let expected = [
        ("_module", "struct"),
        ("_service", "struct"),
        ("_repository", "trait"),
    ]
    .into_iter()
    .find(|(suffix, _)| stem.ends_with(suffix));
    let Some((_, kind)) = expected else {
        return;
    };
    let name = stem.to_upper_camel_case();
    let declared = collected
        .types
        .iter()
        .any(|item| item.name == name && item.kind == kind);
    if !declared {
        findings.push(Finding {
            file: source.rel.clone(),
            line: 1,
            column: 1,
            rule: Rule::Naming,
            message: format!("`{stem}.rs` should declare `{kind} {name}`"),
        });
    }
}

fn parse_sources(
    workspace: &Workspace,
    features: &BTreeSet<String>,
    root: &Path,
    findings: &mut Vec<Finding>,
) -> Vec<Source> {
    let mut sources = Vec::new();
    for member in &workspace.members {
        let src = member.dir.join("src");
        let mut files: Vec<_> = walkdir::WalkDir::new(&src)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(walkdir::DirEntry::into_path)
            .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
            .collect();
        files.sort();
        for path in files {
            let rel = relative(root, &path);
            let parsed = std::fs::read_to_string(&path)
                .map_err(|err| (1, 1, err.to_string()))
                .and_then(|text| {
                    syn::parse_file(&text).map_err(|err| {
                        let start = err.span().start();
                        (start.line, start.column + 1, err.to_string())
                    })
                });
            match parsed {
                Ok(ast) => sources.push(Source {
                    member: member.name.clone(),
                    is_feature: features.contains(&member.name),
                    module_path: module_path(&src, &path),
                    rel,
                    ast,
                }),
                Err((line, column, message)) => findings.push(Finding {
                    file: rel,
                    line,
                    column,
                    rule: Rule::Parse,
                    message: format!("could not parse: {message}"),
                }),
            }
        }
    }
    sources
}

fn module_path(src: &Path, file: &Path) -> Vec<String> {
    let mut segments: Vec<String> = file
        .strip_prefix(src)
        .unwrap_or(file)
        .with_extension("")
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    if segments.len() == 1 && matches!(segments[0].as_str(), "lib" | "main") {
        segments.clear();
    }
    if segments.last().is_some_and(|last| last == "mod") {
        segments.pop();
    }
    segments
}

/// `Pg*` structs declared at the top level of `*_repository.rs` files:
/// adapter name → declaring file.
fn adapter_types(sources: &[Source]) -> BTreeMap<String, String> {
    let mut adapters = BTreeMap::new();
    for source in sources
        .iter()
        .filter(|source| source.stem().ends_with("_repository"))
    {
        for item in &source.ast.items {
            if let syn::Item::Struct(item) = item {
                let name = item.ident.to_string();
                if is_adapter_name(&name) {
                    adapters.insert(name, source.rel.clone());
                }
            }
        }
    }
    adapters
}

fn is_adapter_name(name: &str) -> bool {
    name.strip_prefix("Pg")
        .and_then(|rest| rest.chars().next())
        .is_some_and(char::is_uppercase)
}

fn finding(source: &Source, span: Span, rule: Rule, message: String) -> Finding {
    let start = span.start();
    Finding {
        file: source.rel.clone(),
        line: start.line.max(1),
        column: start.column + 1,
        rule,
        message,
    }
}

struct Handler {
    name: String,
    span: Span,
    has_openapi: bool,
    json_params: Vec<Span>,
}

struct TypeItem {
    name: String,
    kind: &'static str,
    is_struct: bool,
    span: Span,
    derives: BTreeSet<String>,
}

/// Everything the rules need from one file, gathered in a single pass.
struct Collector<'a> {
    adapters: &'a BTreeMap<String, String>,
    handlers: Vec<Handler>,
    types: Vec<TypeItem>,
    sqlx_uses: Vec<Span>,
    constructions: Vec<(String, Span)>,
    module_entries: BTreeSet<String>,
    declares_openapi: bool,
    openapi_paths: BTreeSet<Vec<String>>,
    attribute_errors: Vec<(Span, String)>,
    /// Nesting depth below the file's top level; types and naming only
    /// consider top-level items.
    depth: usize,
}

impl<'a> Collector<'a> {
    const fn new(adapters: &'a BTreeMap<String, String>) -> Self {
        Self {
            adapters,
            handlers: Vec::new(),
            types: Vec::new(),
            sqlx_uses: Vec::new(),
            constructions: Vec::new(),
            module_entries: BTreeSet::new(),
            declares_openapi: false,
            openapi_paths: BTreeSet::new(),
            attribute_errors: Vec::new(),
            depth: 0,
        }
    }

    fn record_type(&mut self, name: &syn::Ident, kind: &'static str, attrs: &[Attribute]) {
        if self.depth == 0 {
            self.types.push(TypeItem {
                name: name.to_string(),
                kind,
                is_struct: kind == "struct",
                span: name.span(),
                derives: derives(attrs),
            });
        }
    }

    fn record_struct_attrs(&mut self, attrs: &[Attribute]) {
        for attr in attrs {
            let last = attr
                .path()
                .segments
                .last()
                .map(|segment| segment.ident.to_string());
            match last.as_deref() {
                Some("module") => self.record_module(attr),
                Some("openapi") => self.record_openapi(attr),
                _ => {}
            }
        }
    }

    fn record_module(&mut self, attr: &Attribute) {
        let args = attr.parse_args_with(Punctuated::<MetaNameValue, Token![,]>::parse_terminated);
        let Ok(args) = args else {
            self.attribute_errors.push((
                attr.pound_token.span,
                "could not read #[module(...)] arguments; expected `key = value` pairs".to_string(),
            ));
            return;
        };
        for arg in args {
            let is_list = arg.path.is_ident("controllers") || arg.path.is_ident("protected");
            if let (true, Expr::Array(array)) = (is_list, &arg.value) {
                for element in &array.elems {
                    if let Expr::Path(path) = element
                        && let Some(last) = path.path.segments.last()
                    {
                        self.module_entries.insert(last.ident.to_string());
                    }
                }
            }
        }
    }

    fn record_openapi(&mut self, attr: &Attribute) {
        self.declares_openapi = true;
        let mut paths = Vec::new();
        let result = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("paths") {
                return meta.parse_nested_meta(|inner| {
                    paths.push(
                        inner
                            .path
                            .segments
                            .iter()
                            .map(|segment| segment.ident.to_string())
                            .collect(),
                    );
                    Ok(())
                });
            }
            if meta.input.peek(Token![=]) {
                meta.value()?.parse::<Expr>()?;
            } else if meta.input.peek(syn::token::Paren) {
                let content;
                syn::parenthesized!(content in meta.input);
                content.parse::<proc_macro2::TokenStream>()?;
            }
            Ok(())
        });
        match result {
            Ok(()) => self.openapi_paths.extend(paths),
            Err(err) => self.attribute_errors.push((
                err.span(),
                format!("could not read #[openapi(...)] arguments: {err}"),
            )),
        }
    }

    fn record_handler(&mut self, item: &ItemFn) {
        if !item.attrs.iter().any(is_route_attr) {
            return;
        }
        let json_params = item
            .sig
            .inputs
            .iter()
            .filter_map(|input| match input {
                FnArg::Typed(typed) => json_type_span(&typed.ty),
                FnArg::Receiver(_) => None,
            })
            .collect();
        self.handlers.push(Handler {
            name: item.sig.ident.to_string(),
            span: item.sig.ident.span(),
            has_openapi: item.attrs.iter().any(|attr| {
                let segments: Vec<String> = attr
                    .path()
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect();
                segments == ["utoipa", "path"]
            }),
            json_params,
        });
    }
}

impl<'ast> Visit<'ast> for Collector<'_> {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if is_cfg_test(&item.attrs) {
            return;
        }
        self.depth += 1;
        visit::visit_item_mod(self, item);
        self.depth -= 1;
    }

    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        if is_cfg_test(&item.attrs) {
            return;
        }
        self.record_handler(item);
        self.depth += 1;
        visit::visit_item_fn(self, item);
        self.depth -= 1;
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if is_cfg_test(&item.attrs) {
            return;
        }
        self.depth += 1;
        visit::visit_item_impl(self, item);
        self.depth -= 1;
    }

    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        if is_cfg_test(&item.attrs) {
            return;
        }
        self.record_type(&item.ident, "struct", &item.attrs);
        self.record_struct_attrs(&item.attrs);
        visit::visit_item_struct(self, item);
    }

    fn visit_item_enum(&mut self, item: &'ast syn::ItemEnum) {
        if is_cfg_test(&item.attrs) {
            return;
        }
        self.record_type(&item.ident, "enum", &item.attrs);
        visit::visit_item_enum(self, item);
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        if is_cfg_test(&item.attrs) {
            return;
        }
        self.record_type(&item.ident, "trait", &item.attrs);
        visit::visit_item_trait(self, item);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        if is_cfg_test(&item.attrs) {
            return;
        }
        let root = match &item.tree {
            syn::UseTree::Path(path) => Some(&path.ident),
            syn::UseTree::Name(name) => Some(&name.ident),
            syn::UseTree::Rename(rename) => Some(&rename.ident),
            syn::UseTree::Glob(_) | syn::UseTree::Group(_) => None,
        };
        if let Some(ident) = root.filter(|ident| *ident == "sqlx") {
            self.sqlx_uses.push(ident.span());
        }
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if let Some(first) = path.segments.first()
            && first.ident == "sqlx"
        {
            self.sqlx_uses.push(first.ident.span());
        }
        visit::visit_path(self, path);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let Expr::Path(func) = &*call.func {
            let segments = &func.path.segments;
            if segments.len() >= 2 {
                let owner = &segments[segments.len() - 2].ident;
                if self.adapters.contains_key(&owner.to_string()) {
                    self.constructions.push((owner.to_string(), owner.span()));
                }
            }
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_struct(&mut self, expr: &'ast syn::ExprStruct) {
        if let Some(last) = expr.path.segments.last()
            && self.adapters.contains_key(&last.ident.to_string())
        {
            self.constructions
                .push((last.ident.to_string(), last.ident.span()));
        }
        visit::visit_expr_struct(self, expr);
    }
}

fn is_cfg_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("cfg") {
            return false;
        }
        let Meta::List(list) = &attr.meta else {
            return false;
        };
        let tokens = list.tokens.to_string();
        tokens
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|word| word == "test")
            && !tokens.contains("not")
    })
}

fn is_route_attr(attr: &Attribute) -> bool {
    let path = attr.path();
    let Some(last) = path.segments.last() else {
        return false;
    };
    let qualified_ok = path.segments.len() == 1
        || path
            .segments
            .first()
            .is_some_and(|first| first.ident == "actix_web" || first.ident == "web");
    qualified_ok
        && ROUTE_MACROS.iter().any(|name| last.ident == name)
        && matches!(attr.meta, Meta::List(_))
}

fn json_type_span(ty: &Type) -> Option<Span> {
    let Type::Path(path) = ty else {
        return None;
    };
    let last = path.path.segments.last()?;
    (last.ident == "Json").then(|| last.ident.span())
}

fn derives(attrs: &[Attribute]) -> BTreeSet<String> {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("derive"))
        .filter_map(|attr| {
            attr.parse_args_with(Punctuated::<syn::Path, Token![,]>::parse_terminated)
                .ok()
        })
        .flatten()
        .filter_map(|path| {
            path.segments
                .last()
                .map(|segment| segment.ident.to_string())
        })
        .collect()
}
