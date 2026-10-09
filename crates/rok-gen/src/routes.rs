//! `cargo rok routes`: the route table, read from source. Every handler
//! listed in a `#[module(controllers/protected = [...])]` is joined with its
//! `#[get("...")]`-style attribute and the module's prefix and middleware.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use rok_locate::Source;
use rok_plan::project::Project;
use serde::Serialize;
use syn::{Expr, Item, Lit};

const ROUTE_MACROS: [&str; 9] = [
    "get", "post", "put", "patch", "delete", "head", "options", "trace", "connect",
];

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Route {
    pub path: String,
    pub method: String,
    /// `crate::handler`.
    pub handler: String,
    /// Behind the module's middleware (e.g. `require_auth`).
    pub guarded: bool,
    pub file: String,
}

/// A handler's own route attribute.
struct Handler {
    methods: Vec<String>,
    path: String,
    file: String,
}

/// One `#[module]`'s registrations.
#[derive(Default)]
struct Module {
    prefix: String,
    open: Vec<String>,
    guarded: Vec<String>,
}

/// Every registered route in `project`, sorted by path then method.
pub fn routes(project: &Project) -> Vec<Route> {
    let mut handlers: BTreeMap<(String, String), Handler> = BTreeMap::new();
    let mut modules: Vec<(String, Module)> = Vec::new();

    for (krate, src) in source_dirs(&project.root) {
        for entry in walkdir::WalkDir::new(&src)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "rs"))
        {
            let rel = entry
                .path()
                .strip_prefix(&project.root)
                .unwrap_or_else(|_| entry.path())
                .to_string_lossy()
                .replace('\\', "/");
            let Ok(text) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            let Ok(source) = Source::parse(&rel, &text) else {
                continue;
            };
            collect_handlers(&source, &krate, &rel, &mut handlers);
            if let Some(module) = read_module(&source) {
                modules.push((krate.clone(), module));
            }
        }
    }

    let mut routes = Vec::new();
    for (krate, module) in &modules {
        let entries = module
            .open
            .iter()
            .map(|name| (name, false))
            .chain(module.guarded.iter().map(|name| (name, true)));
        for (name, guarded) in entries {
            let Some(handler) = handlers.get(&(krate.clone(), name.clone())) else {
                continue;
            };
            for method in &handler.methods {
                routes.push(Route {
                    path: join_path(&module.prefix, &handler.path),
                    method: method.to_ascii_uppercase(),
                    handler: format!("{krate}::{name}"),
                    guarded,
                    file: handler.file.clone(),
                });
            }
        }
    }
    routes.sort();
    routes.dedup();
    routes
}

/// The table for `--output human`.
pub fn table(routes: &[Route]) -> String {
    let width = |pick: fn(&Route) -> &str| {
        routes
            .iter()
            .map(|route| pick(route).len())
            .max()
            .unwrap_or(0)
    };
    let (method_w, path_w, handler_w) = (
        width(|route| &route.method).max(6),
        width(|route| &route.path).max(4),
        width(|route| &route.handler).max(7),
    );
    let mut text = format!(
        "{:method_w$}  {:path_w$}  {:handler_w$}  GUARD\n",
        "METHOD", "PATH", "HANDLER"
    );
    for route in routes {
        let _ = writeln!(
            text,
            "{:method_w$}  {:path_w$}  {:handler_w$}  {}",
            route.method,
            route.path,
            route.handler,
            if route.guarded { "auth" } else { "-" }
        );
    }
    let _ = writeln!(text, "{} routes", routes.len());
    text
}

/// `(crate name, src dir)` for the root package and every `crates/*`.
fn source_dirs(root: &Path) -> Vec<(String, std::path::PathBuf)> {
    let mut dirs = Vec::new();
    let mut add = |dir: &Path| {
        let manifest: Option<toml::Table> = std::fs::read_to_string(dir.join("Cargo.toml"))
            .ok()
            .and_then(|text| toml::from_str(&text).ok());
        let name = manifest
            .as_ref()
            .and_then(|manifest| manifest.get("package"))
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str);
        if let Some(name) = name {
            dirs.push((name.replace('-', "_"), dir.join("src")));
        }
    };
    add(root);
    if let Ok(entries) = std::fs::read_dir(root.join("crates")) {
        let mut crate_dirs: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        crate_dirs.sort();
        for dir in crate_dirs {
            add(&dir);
        }
    }
    dirs
}

fn collect_handlers(
    source: &Source,
    krate: &str,
    rel: &str,
    handlers: &mut BTreeMap<(String, String), Handler>,
) {
    for item in &source.file.items {
        let Item::Fn(function) = item else {
            continue;
        };
        for attr in &function.attrs {
            let Some(last) = attr.path().segments.last() else {
                continue;
            };
            let method = last.ident.to_string();
            if !ROUTE_MACROS.contains(&method.as_str()) {
                continue;
            }
            let Ok(Lit::Str(path)) = attr.parse_args_with(|input: syn::parse::ParseStream<'_>| {
                let lit: Lit = input.parse()?;
                input.parse::<proc_macro2::TokenStream>()?;
                Ok(lit)
            }) else {
                continue;
            };
            handlers
                .entry((krate.to_string(), function.sig.ident.to_string()))
                .or_insert_with(|| Handler {
                    methods: Vec::new(),
                    path: path.value(),
                    file: rel.to_string(),
                })
                .methods
                .push(method);
        }
    }
}

fn read_module(source: &Source) -> Option<Module> {
    let args = source.module_args().ok()?;
    let mut module = Module::default();
    let mut controllers = Vec::new();
    let mut protected = Vec::new();
    let mut middleware = false;
    for (key, value) in args {
        match key.as_str() {
            "prefix" => module.prefix = value.trim_matches('"').to_string(),
            "controllers" => controllers = names_in(&value),
            "protected" => protected = names_in(&value),
            "middleware" => middleware = true,
            _ => {}
        }
    }
    // `middleware` wraps the whole scope, or only `protected` when present
    // (see the `#[module]` macro's docs).
    if middleware && protected.is_empty() {
        module.guarded = controllers;
    } else {
        module.open = controllers;
        module.guarded = protected;
    }
    Some(module)
}

fn names_in(array: &str) -> Vec<String> {
    let Ok(Expr::Array(array)) = syn::parse_str::<Expr>(array) else {
        return Vec::new();
    };
    array
        .elems
        .iter()
        .filter_map(|element| match element {
            Expr::Path(path) => path
                .path
                .segments
                .last()
                .map(|segment| segment.ident.to_string()),
            _ => None,
        })
        .collect()
}

fn join_path(prefix: &str, path: &str) -> String {
    let joined = format!(
        "{}/{}",
        prefix.trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    let trimmed = joined.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}
