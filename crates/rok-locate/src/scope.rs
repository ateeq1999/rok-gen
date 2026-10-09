//! The name-based scope check: which variables a snippet reads, and which
//! names are visible at an insertion point. Deliberately an
//! over-approximation of scope (no type checking): it exists to catch
//! typos and wrong placements early, and `--verify` catches the rest.

use std::collections::BTreeSet;

use syn::parse::Parser;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Expr, FnArg, Item, Pat};

use crate::{LocateError, Source};

/// Variables `snippet` (statements) reads that it doesn't bind itself:
/// single-segment, lowercase path expressions, excluding `self`, callees
/// (`drop(x)` reads `x`, not `drop`) and anything inside macro calls.
///
/// # Errors
///
/// A message when `snippet` isn't valid statements.
pub fn free_variables(snippet: &str) -> Result<BTreeSet<String>, String> {
    let stmts = syn::Block::parse_within
        .parse_str(snippet)
        .map_err(|err| err.to_string())?;
    let mut reads = Reads::default();
    for stmt in &stmts {
        reads.visit_stmt(stmt);
    }
    Ok(reads.used.difference(&reads.bound).cloned().collect())
}

#[derive(Default)]
struct Reads {
    used: BTreeSet<String>,
    bound: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for Reads {
    fn visit_expr(&mut self, expr: &'ast Expr) {
        match expr {
            Expr::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 => {
                let name = path.path.segments[0].ident.to_string();
                if name != "self" && name.starts_with(|c: char| c.is_lowercase() || c == '_') {
                    self.used.insert(name);
                }
            }
            Expr::Call(call) => {
                // The callee is a function name, not a variable read.
                if !matches!(&*call.func, Expr::Path(path) if path.path.segments.len() == 1) {
                    self.visit_expr(&call.func);
                }
                for arg in &call.args {
                    self.visit_expr(arg);
                }
            }
            _ => visit::visit_expr(self, expr),
        }
    }

    fn visit_pat(&mut self, pat: &'ast Pat) {
        bindings(pat, &mut self.bound);
    }
}

/// Every name a pattern binds.
fn bindings(pat: &Pat, out: &mut BTreeSet<String>) {
    struct Names<'a>(&'a mut BTreeSet<String>);
    impl<'ast> Visit<'ast> for Names<'_> {
        fn visit_pat_ident(&mut self, ident: &'ast syn::PatIdent) {
            self.0.insert(ident.ident.to_string());
            visit::visit_pat_ident(self, ident);
        }
    }
    Names(out).visit_pat(pat);
}

impl Source {
    /// Names visible at byte `offset` inside `function`: its parameters
    /// (and `self`), `let` bindings that end before `offset`, the bindings
    /// of closures, loops, `match` arms and `if let`s that contain it, and
    /// the file's top-level items and imports.
    ///
    /// # Errors
    ///
    /// [`LocateError::NotFound`] when there's no such function.
    pub fn names_in_scope(
        &self,
        function: &str,
        offset: usize,
    ) -> Result<BTreeSet<String>, LocateError> {
        let site = self.find_fn(function)?;
        let mut names = self.imported_names();
        names.extend(self.file.items.iter().filter_map(item_name));
        for input in &site.sig.inputs {
            match input {
                FnArg::Receiver(_) => {
                    names.insert("self".to_string());
                }
                FnArg::Typed(typed) => bindings(&typed.pat, &mut names),
            }
        }
        let mut visible = Visible {
            source: self,
            offset,
            names: &mut names,
        };
        visible.visit_block(site.block);
        Ok(names)
    }
}

struct Visible<'a> {
    source: &'a Source,
    offset: usize,
    names: &'a mut BTreeSet<String>,
}

impl Visible<'_> {
    fn contains(&self, node: &impl Spanned) -> bool {
        let span = node.span();
        self.source.start(span) <= self.offset && self.offset <= self.source.end(span)
    }
}

impl<'ast> Visit<'ast> for Visible<'_> {
    fn visit_local(&mut self, local: &'ast syn::Local) {
        if self.source.end(local.span()) <= self.offset {
            bindings(&local.pat, self.names);
        }
        visit::visit_local(self, local);
    }

    fn visit_expr_closure(&mut self, closure: &'ast syn::ExprClosure) {
        if self.contains(&*closure.body) {
            for input in &closure.inputs {
                bindings(input, self.names);
            }
        }
        visit::visit_expr_closure(self, closure);
    }

    fn visit_expr_for_loop(&mut self, expr: &'ast syn::ExprForLoop) {
        if self.contains(&expr.body) {
            bindings(&expr.pat, self.names);
        }
        visit::visit_expr_for_loop(self, expr);
    }

    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        if self.contains(&*arm.body) {
            bindings(&arm.pat, self.names);
        }
        visit::visit_arm(self, arm);
    }

    fn visit_expr_if(&mut self, expr: &'ast syn::ExprIf) {
        if let Expr::Let(binding) = &*expr.cond
            && self.contains(&expr.then_branch)
        {
            bindings(&binding.pat, self.names);
        }
        visit::visit_expr_if(self, expr);
    }

    fn visit_expr_while(&mut self, expr: &'ast syn::ExprWhile) {
        if let Expr::Let(binding) = &*expr.cond
            && self.contains(&expr.body)
        {
            bindings(&binding.pat, self.names);
        }
        visit::visit_expr_while(self, expr);
    }
}

fn item_name(item: &Item) -> Option<String> {
    match item {
        Item::Fn(item) => Some(item.sig.ident.to_string()),
        Item::Const(item) => Some(item.ident.to_string()),
        Item::Static(item) => Some(item.ident.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_variables_skip_callees_macros_self_and_own_bindings() {
        let free = free_variables(
            "let total = count(items);\ndrop(self.cache.get(&key));\nprintln!(\"{hidden}\");\nlet f = |x| x + offset;\nSome(total)",
        )
        .unwrap();
        assert_eq!(
            free.into_iter().collect::<Vec<_>>(),
            ["items", "key", "offset"]
        );
    }

    #[test]
    fn scope_includes_params_earlier_lets_and_enclosing_bindings() {
        let text = "fn run(&self, id: u32) {\n    let a = 1;\n    for item in list {\n        let b = 2;\n    }\n    let c = 3;\n}\n";
        let source = Source::parse("t", text).unwrap();
        let inside_loop = text.find("let b").unwrap();
        let names = source.names_in_scope("run", inside_loop).unwrap();
        for name in ["self", "id", "a", "item", "run"] {
            assert!(names.contains(name), "{name}: {names:?}");
        }
        assert!(!names.contains("c"), "{names:?}");
        let before_a = text.find("let a").unwrap();
        assert!(
            !source
                .names_in_scope("run", before_a)
                .unwrap()
                .contains("a")
        );
    }
}
