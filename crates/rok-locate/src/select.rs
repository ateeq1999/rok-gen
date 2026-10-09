//! Resolving an [`At`] selector to concrete places in a function body, and
//! turning those places into edits.

use rok_plan::integrate::{At, Branch, StmtIndex, StmtMatcher, WithinStep};
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Block, Expr, ImplItem, Item, Pat, Stmt};

use crate::{Edit, LocateError, Source};

/// A function found by name.
pub struct FnSite<'a> {
    pub name: String,
    pub block: &'a Block,
    pub sig: &'a syn::Signature,
}

/// A block's extent and its top-level statements.
#[derive(Clone, Debug)]
pub struct BlockSite {
    /// Offset just after `{`.
    pub open: usize,
    /// Offset of `}`.
    pub close: usize,
    pub stmts: Vec<StmtSite>,
}

/// One top-level statement of a block.
#[derive(Clone, Debug)]
pub struct StmtSite {
    pub index: usize,
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub text: String,
    /// The block's final expression (no trailing `;`).
    pub tail: bool,
}

impl StmtSite {
    /// `L12 [1] self.posts.delete(id).await?;` (first line only).
    #[must_use]
    pub fn describe(&self) -> String {
        let first = self.text.lines().next().unwrap_or_default().trim();
        let tail = if self.tail { "  <tail>" } else { "" };
        format!("L{} [{}] {first}{tail}", self.line, self.index)
    }
}

/// Where a selector points.
#[derive(Clone, Debug)]
pub enum Place {
    Before(StmtSite),
    After(StmtSite),
    Stmt(StmtSite),
    Start(BlockSite),
    End(BlockSite),
}

impl Source {
    /// A top-level function (`run`) or a method (`PostService::delete`,
    /// from an inherent or trait impl).
    ///
    /// # Errors
    ///
    /// [`LocateError::NotFound`] when there's none or more than one.
    pub fn find_fn(&self, path: &str) -> Result<FnSite<'_>, LocateError> {
        let mut found = Vec::new();
        match path.split_once("::") {
            None => {
                for item in &self.file.items {
                    if let Item::Fn(function) = item
                        && function.sig.ident == path
                    {
                        found.push(FnSite {
                            name: path.to_string(),
                            block: &function.block,
                            sig: &function.sig,
                        });
                    }
                }
            }
            Some((owner, method)) => {
                for item in &self.file.items {
                    let Item::Impl(implementation) = item else {
                        continue;
                    };
                    let syn::Type::Path(self_ty) = &*implementation.self_ty else {
                        continue;
                    };
                    if self_ty
                        .path
                        .segments
                        .last()
                        .is_none_or(|last| last.ident != owner)
                    {
                        continue;
                    }
                    for impl_item in &implementation.items {
                        if let ImplItem::Fn(function) = impl_item
                            && function.sig.ident == method
                        {
                            found.push(FnSite {
                                name: path.to_string(),
                                block: &function.block,
                                sig: &function.sig,
                            });
                        }
                    }
                }
            }
        }
        match found.len() {
            1 => Ok(found.remove(0)),
            0 => Err(LocateError::NotFound(format!(
                "no function `{path}`; available: {}",
                self.function_names().join(", ")
            ))),
            n => Err(LocateError::NotFound(format!(
                "`{path}` matches {n} functions (e.g. an inherent and a trait impl); \
                 rename or narrow it"
            ))),
        }
    }

    /// Every addressable function name in the file.
    #[must_use]
    pub fn function_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for item in &self.file.items {
            match item {
                Item::Fn(function) => names.push(function.sig.ident.to_string()),
                Item::Impl(implementation) => {
                    let syn::Type::Path(self_ty) = &*implementation.self_ty else {
                        continue;
                    };
                    let Some(owner) = self_ty.path.segments.last() else {
                        continue;
                    };
                    for impl_item in &implementation.items {
                        if let ImplItem::Fn(function) = impl_item {
                            names.push(format!("{}::{}", owner.ident, function.sig.ident));
                        }
                    }
                }
                _ => {}
            }
        }
        names
    }

    /// The block's extent and statements.
    #[must_use]
    pub fn block_site(&self, block: &Block) -> BlockSite {
        let last = block.stmts.len().saturating_sub(1);
        BlockSite {
            open: self.end(block.brace_token.span.open()),
            close: self.start(block.brace_token.span.close()),
            stmts: block
                .stmts
                .iter()
                .enumerate()
                .map(|(index, stmt)| {
                    let span = stmt.span();
                    StmtSite {
                        index,
                        start: self.start(span),
                        end: self.end(span),
                        line: span.start().line,
                        text: self.snippet(stmt).to_string(),
                        tail: index == last && matches!(stmt, Stmt::Expr(_, None)),
                    }
                })
                .collect(),
        }
    }

    /// Resolves `at` to one or more places (several only with `all`).
    ///
    /// # Errors
    ///
    /// [`LocateError::NotFound`] for a missing function, nested block or
    /// statement, an ambiguous match, or a selector that names no place
    /// (or several kinds of place).
    pub fn resolve(&self, at: &At) -> Result<Vec<Place>, LocateError> {
        let function = self.find_fn(&at.function)?;
        let mut block = function.block;
        for step in &at.within {
            block = descend(self, block, step)?;
        }
        let site = self.block_site(block);

        let chosen = [
            at.before.is_some(),
            at.after.is_some(),
            at.stmt.is_some(),
            at.start,
            at.end,
        ]
        .iter()
        .filter(|set| **set)
        .count();
        if chosen != 1 {
            return Err(LocateError::NotFound(
                "set exactly one of `before`, `after`, `stmt`, `start`, `end`".to_string(),
            ));
        }
        if at.start {
            return Ok(vec![Place::Start(site)]);
        }
        if at.end {
            return Ok(vec![Place::End(site)]);
        }
        let (matcher, wrap): (&StmtMatcher, fn(StmtSite) -> Place) =
            match (&at.before, &at.after, &at.stmt) {
                (Some(matcher), _, _) => (matcher, Place::Before),
                (_, Some(matcher), _) => (matcher, Place::After),
                (_, _, Some(matcher)) => (matcher, Place::Stmt),
                _ => return Err(LocateError::NotFound("no statement matcher".to_string())),
            };
        let candidates = self.match_stmts(block, &site, matcher);
        let chosen_stmts = select(candidates, at, &site, &function.name)?;
        Ok(chosen_stmts.into_iter().map(wrap).collect())
    }

    fn match_stmts(&self, block: &Block, site: &BlockSite, matcher: &StmtMatcher) -> Vec<StmtSite> {
        site.stmts
            .iter()
            .zip(&block.stmts)
            .filter(|(stmt_site, stmt)| match matcher {
                StmtMatcher::Index(StmtIndex::Position(position)) => stmt_site.index == *position,
                StmtMatcher::Index(StmtIndex::Named(_)) => stmt_site.tail,
                StmtMatcher::Let(name) => let_binds(stmt, name),
                StmtMatcher::Contains(needle) => stmt_site.text.contains(needle.as_str()),
                StmtMatcher::Call(path) => self.calls(stmt).contains(&squash(path)),
            })
            .map(|(stmt_site, _)| stmt_site.clone())
            .collect()
    }

    /// Every call a statement makes, as `receiver.method` or `path`, with
    /// whitespace removed.
    fn calls(&self, stmt: &Stmt) -> Vec<String> {
        struct Calls<'a> {
            source: &'a Source,
            found: Vec<String>,
        }
        impl<'ast> Visit<'ast> for Calls<'_> {
            fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
                self.found.push(format!(
                    "{}.{}",
                    squash(self.source.snippet(&*call.receiver)),
                    call.method
                ));
                syn::visit::visit_expr_method_call(self, call);
            }
            fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
                self.found.push(squash(self.source.snippet(&*call.func)));
                syn::visit::visit_expr_call(self, call);
            }
        }
        let mut calls = Calls {
            source: self,
            found: Vec::new(),
        };
        calls.visit_stmt(stmt);
        calls.found
    }

    /// The edit inserting `code` (one or more statements) at `place`,
    /// indented like its neighbours.
    #[must_use]
    pub fn insert_edit(&self, place: &Place, code: &str) -> Edit {
        match place {
            Place::Before(stmt) => {
                let indent = self.indent_at(stmt.start);
                Edit::new(
                    stmt.start,
                    format!("{}\n{indent}", indent_lines(code, &indent)),
                )
            }
            Place::After(stmt) | Place::Stmt(stmt) => {
                let indent = self.indent_at(stmt.start);
                Edit::new(
                    stmt.end,
                    format!("\n{indent}{}", indent_lines(code, &indent)),
                )
            }
            Place::Start(block) => {
                let indent = self.inner_indent(block);
                Edit::new(
                    block.open,
                    format!("\n{indent}{}", indent_lines(code, &indent)),
                )
            }
            Place::End(block) => match block.stmts.last() {
                Some(tail) if tail.tail => self.insert_edit(&Place::Before(tail.clone()), code),
                Some(last) => self.insert_edit(&Place::After(last.clone()), code),
                None => self.insert_edit(&Place::Start(block.clone()), code),
            },
        }
    }

    /// The edit replacing the statement at `place` with `code`.
    ///
    /// # Errors
    ///
    /// [`LocateError::NotFound`] unless `place` is a [`Place::Stmt`].
    pub fn replace_edit(&self, place: &Place, code: &str) -> Result<Edit, LocateError> {
        let Place::Stmt(stmt) = place else {
            return Err(LocateError::NotFound(
                "`replace` needs `at.stmt` to name the statement to replace".to_string(),
            ));
        };
        let indent = self.indent_at(stmt.start);
        Ok(Edit::replace(
            stmt.start,
            stmt.end,
            indent_lines(code, &indent),
        ))
    }

    /// The whitespace between the start of `offset`'s line and `offset`.
    fn indent_at(&self, offset: usize) -> String {
        let text = self.text();
        let line_start = text[..offset].rfind('\n').map_or(0, |index| index + 1);
        text[line_start..offset]
            .chars()
            .take_while(|c| c.is_whitespace())
            .collect()
    }

    fn inner_indent(&self, block: &BlockSite) -> String {
        block.stmts.first().map_or_else(
            || format!("{}    ", self.indent_at(block.close)),
            |first| self.indent_at(first.start),
        )
    }
}

fn select(
    matched: Vec<StmtSite>,
    at: &At,
    site: &BlockSite,
    function: &str,
) -> Result<Vec<StmtSite>, LocateError> {
    let listing = |stmts: &[StmtSite]| {
        stmts.iter().fold(String::new(), |mut text, stmt| {
            text.push_str("\n  ");
            text.push_str(&stmt.describe());
            text
        })
    };
    if matched.is_empty() {
        return Err(LocateError::NotFound(format!(
            "no statement in `{function}` matches; its statements are:{}",
            listing(&site.stmts)
        )));
    }
    if at.all {
        return Ok(matched);
    }
    if let Some(occurrence) = at.occurrence {
        let count = matched.len();
        return matched
            .into_iter()
            .nth(occurrence.saturating_sub(1))
            .map(|stmt| vec![stmt])
            .ok_or_else(|| {
                LocateError::NotFound(format!(
                    "occurrence {occurrence} requested, but only {count} statements match"
                ))
            });
    }
    if matched.len() > 1 {
        return Err(LocateError::NotFound(format!(
            "{} statements in `{function}` match; add `occurrence` (1-based) or `all`:{}",
            matched.len(),
            listing(&matched)
        )));
    }
    Ok(matched)
}

fn descend<'a>(
    source: &Source,
    block: &'a Block,
    step: &WithinStep,
) -> Result<&'a Block, LocateError> {
    let kinds = [
        ("if", step.if_),
        ("match", step.match_),
        ("for", step.for_),
        ("while", step.while_),
        ("loop", step.loop_),
        ("closure", step.closure),
        ("block", step.block),
    ];
    let mut set = kinds.iter().filter(|(_, index)| index.is_some());
    let (Some((kind, Some(index))), None) = (set.next(), set.next()) else {
        return Err(LocateError::NotFound(
            "each `within` step sets exactly one of if/match/for/while/loop/closure/block"
                .to_string(),
        ));
    };
    let candidates: Vec<&Expr> = block
        .stmts
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::Expr(expr, _) => Some(expr),
            Stmt::Local(local) => local.init.as_ref().map(|init| &*init.expr),
            _ => None,
        })
        .filter(|expr| kind_of(expr) == Some(*kind))
        .collect();
    let expr = candidates.get(*index).ok_or_else(|| {
        LocateError::NotFound(format!(
            "`within` asks for {kind} #{index}, but the block has {} {kind} statements",
            candidates.len()
        ))
    })?;
    let not_block = |what: &str| {
        LocateError::NotFound(format!(
            "{what} isn't a `{{ ... }}` block; it can't be entered"
        ))
    };
    match expr {
        Expr::If(expr_if) => match step.branch.unwrap_or_default() {
            Branch::Then => Ok(&expr_if.then_branch),
            Branch::Else => match expr_if.else_branch.as_ref().map(|(_, expr)| &**expr) {
                Some(Expr::Block(block)) => Ok(&block.block),
                Some(_) => Err(not_block("this `else` (an `else if`)")),
                None => Err(LocateError::NotFound("this `if` has no `else`".to_string())),
            },
        },
        Expr::Match(expr_match) => {
            let wanted =
                step.arm.as_deref().map(squash).ok_or_else(|| {
                    LocateError::NotFound("a `match` step needs `arm`".to_string())
                })?;
            let arm = expr_match
                .arms
                .iter()
                .find(|arm| squash(source.snippet(&arm.pat)) == wanted)
                .ok_or_else(|| {
                    let arms: Vec<String> = expr_match
                        .arms
                        .iter()
                        .map(|arm| source.snippet(&arm.pat).to_string())
                        .collect();
                    LocateError::NotFound(format!(
                        "no arm `{wanted}`; arms are: {}",
                        arms.join(", ")
                    ))
                })?;
            match &*arm.body {
                Expr::Block(block) => Ok(&block.block),
                _ => Err(not_block("this arm's body")),
            }
        }
        Expr::ForLoop(expr) => Ok(&expr.body),
        Expr::While(expr) => Ok(&expr.body),
        Expr::Loop(expr) => Ok(&expr.body),
        Expr::Closure(closure) => match &*closure.body {
            Expr::Block(block) => Ok(&block.block),
            _ => Err(not_block("this closure's body")),
        },
        Expr::Block(block) => Ok(&block.block),
        Expr::Unsafe(block) => Ok(&block.block),
        _ => Err(not_block("this statement")),
    }
}

/// The `within` kind of an expression, if it has one.
pub const fn kind_of(expr: &Expr) -> Option<&'static str> {
    match expr {
        Expr::If(_) => Some("if"),
        Expr::Match(_) => Some("match"),
        Expr::ForLoop(_) => Some("for"),
        Expr::While(_) => Some("while"),
        Expr::Loop(_) => Some("loop"),
        Expr::Closure(_) => Some("closure"),
        Expr::Block(_) | Expr::Unsafe(_) => Some("block"),
        _ => None,
    }
}

fn let_binds(stmt: &Stmt, name: &str) -> bool {
    let Stmt::Local(local) = stmt else {
        return false;
    };
    let pat = match &local.pat {
        Pat::Type(typed) => &*typed.pat,
        pat => pat,
    };
    matches!(pat, Pat::Ident(ident) if ident.ident == name)
}

pub fn squash(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// `code` with every line after the first indented by `indent`.
fn indent_lines(code: &str, indent: &str) -> String {
    let mut lines = code.trim().lines();
    let mut output = lines.next().unwrap_or_default().to_string();
    for line in lines {
        output.push('\n');
        if !line.trim().is_empty() {
            output.push_str(indent);
        }
        output.push_str(line);
    }
    output
}

/// Collapses whitespace runs to single spaces (for one-line display).
pub fn squash_type(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A call's argument list, and whether the call sits in test-only code.
#[derive(Clone, Debug)]
pub struct CallSite {
    pub args: crate::ListSite,
    pub in_test: bool,
    pub line: usize,
}

impl Source {
    /// The field list of `struct name { ... }`.
    ///
    /// # Errors
    ///
    /// [`LocateError::NotFound`] when there's no such struct with named
    /// fields.
    pub fn struct_fields(&self, name: &str) -> Result<crate::ListSite, LocateError> {
        self.file
            .items
            .iter()
            .find_map(|item| match item {
                Item::Struct(item) if item.ident == name => match &item.fields {
                    syn::Fields::Named(fields) => Some(crate::ListSite::new(
                        self.text(),
                        self.end(fields.brace_token.span.open()),
                        self.start(fields.brace_token.span.close()),
                    )),
                    _ => None,
                },
                _ => None,
            })
            .ok_or_else(|| {
                LocateError::NotFound(format!("no `struct {name} {{ ... }}` in this file"))
            })
    }

    /// A function's parameter list.
    #[must_use]
    pub fn fn_params(&self, function: &FnSite<'_>) -> crate::ListSite {
        crate::ListSite::new(
            self.text(),
            self.end(function.sig.paren_token.span.open()),
            self.start(function.sig.paren_token.span.close()),
        )
    }

    /// The field lists of every `Self { ... }` / `Name { ... }` literal in
    /// `function`'s body.
    #[must_use]
    pub fn struct_literals(&self, function: &FnSite<'_>, name: &str) -> Vec<crate::ListSite> {
        struct Literals<'a> {
            source: &'a Source,
            name: &'a str,
            found: Vec<crate::ListSite>,
        }
        impl<'ast> Visit<'ast> for Literals<'_> {
            fn visit_expr_struct(&mut self, expr: &'ast syn::ExprStruct) {
                let matches = expr
                    .path
                    .segments
                    .last()
                    .is_some_and(|last| last.ident == "Self" || last.ident == self.name);
                if matches {
                    self.found.push(crate::ListSite::new(
                        self.source.text(),
                        self.source.end(expr.brace_token.span.open()),
                        self.source.start(expr.brace_token.span.close()),
                    ));
                }
                syn::visit::visit_expr_struct(self, expr);
            }
        }
        let mut literals = Literals {
            source: self,
            name,
            found: Vec::new(),
        };
        literals.visit_block(function.block);
        literals.found
    }

    /// Every call whose callee path ends with `suffix` (e.g.
    /// `PostService::new`), anywhere in the file.
    #[must_use]
    pub fn call_sites(&self, suffix: &str) -> Vec<CallSite> {
        struct Calls<'a> {
            source: &'a Source,
            suffix: String,
            depth_in_test: usize,
            found: Vec<CallSite>,
        }
        impl Calls<'_> {
            fn enter(&mut self, attrs: &[syn::Attribute]) -> bool {
                let test = attrs.iter().any(|attr| {
                    attr.path().is_ident("cfg")
                        && matches!(&attr.meta, syn::Meta::List(list) if list.tokens.to_string().contains("test"))
                        || attr.path().segments.last().is_some_and(|last| last.ident == "test")
                });
                if test {
                    self.depth_in_test += 1;
                }
                test
            }
        }
        impl<'ast> Visit<'ast> for Calls<'_> {
            fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
                let test = self.enter(&item.attrs);
                syn::visit::visit_item_mod(self, item);
                if test {
                    self.depth_in_test -= 1;
                }
            }
            fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
                let test = self.enter(&item.attrs);
                syn::visit::visit_item_fn(self, item);
                if test {
                    self.depth_in_test -= 1;
                }
            }
            fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
                let callee = squash(self.source.snippet(&*call.func));
                if callee == self.suffix || callee.ends_with(&format!("::{}", self.suffix)) {
                    self.found.push(CallSite {
                        args: crate::ListSite::new(
                            self.source.text(),
                            self.source.end(call.paren_token.span.open()),
                            self.source.start(call.paren_token.span.close()),
                        ),
                        in_test: self.depth_in_test > 0,
                        line: call.paren_token.span.open().start().line,
                    });
                }
                syn::visit::visit_expr_call(self, call);
            }
        }
        let mut calls = Calls {
            source: self,
            suffix: squash(suffix),
            depth_in_test: 0,
            found: Vec::new(),
        };
        calls.visit_file(&self.file);
        calls.found
    }
}

impl Source {
    /// The field lists of every `Name { ... }` literal in the file that
    /// lists its fields explicitly (literals with `..rest` are skipped:
    /// they don't need a new field).
    #[must_use]
    pub fn struct_literal_sites(&self, name: &str) -> Vec<crate::ListSite> {
        struct Literals<'a> {
            source: &'a Source,
            name: &'a str,
            found: Vec<crate::ListSite>,
        }
        impl<'ast> Visit<'ast> for Literals<'_> {
            fn visit_expr_struct(&mut self, expr: &'ast syn::ExprStruct) {
                let named = expr
                    .path
                    .segments
                    .last()
                    .is_some_and(|last| last.ident == self.name);
                if named && expr.rest.is_none() && expr.dot2_token.is_none() {
                    self.found.push(crate::ListSite::new(
                        self.source.text(),
                        self.source.end(expr.brace_token.span.open()),
                        self.source.start(expr.brace_token.span.close()),
                    ));
                }
                syn::visit::visit_expr_struct(self, expr);
            }
        }
        let mut literals = Literals {
            source: self,
            name,
            found: Vec::new(),
        };
        literals.visit_file(&self.file);
        literals.found
    }
}

/// A trait or impl block: where to append items, and what it already has.
#[derive(Clone, Debug)]
pub struct ItemsSite {
    /// For impls, the implementing type (`PgCommentRepository`).
    pub owner: String,
    /// Offset of the closing `}`.
    pub close: usize,
    /// Names of the functions it already declares.
    pub functions: Vec<String>,
}

impl Source {
    /// `trait name { ... }`, at the top level or in an inline module.
    ///
    /// # Errors
    ///
    /// [`LocateError::NotFound`].
    pub fn trait_site(&self, name: &str) -> Result<ItemsSite, LocateError> {
        let mut found = None;
        walk_items(&self.file.items, &mut |item| {
            if let Item::Trait(item) = item
                && item.ident == name
                && found.is_none()
            {
                found = Some(ItemsSite {
                    owner: name.to_string(),
                    close: self.start(item.brace_token.span.close()),
                    functions: item
                        .items
                        .iter()
                        .filter_map(|item| match item {
                            syn::TraitItem::Fn(function) => Some(function.sig.ident.to_string()),
                            _ => None,
                        })
                        .collect(),
                });
            }
        });
        found.ok_or_else(|| LocateError::NotFound(format!("no `trait {name}` in this file")))
    }

    /// Every `impl name for Type { ... }` in the file, including inline
    /// modules (e.g. a `#[cfg(test)] mod fake`).
    #[must_use]
    pub fn impls_of(&self, name: &str) -> Vec<ItemsSite> {
        let mut found = Vec::new();
        walk_items(&self.file.items, &mut |item| {
            let Item::Impl(implementation) = item else {
                return;
            };
            let implements = implementation
                .trait_
                .as_ref()
                .and_then(|(_, path, _)| path.segments.last())
                .is_some_and(|last| last.ident == name);
            let owner = match &*implementation.self_ty {
                syn::Type::Path(path) => {
                    path.path.segments.last().map(|last| last.ident.to_string())
                }
                _ => None,
            };
            if let (true, Some(owner)) = (implements, owner) {
                found.push(ItemsSite {
                    owner,
                    close: self.start(implementation.brace_token.span.close()),
                    functions: implementation
                        .items
                        .iter()
                        .filter_map(|item| match item {
                            ImplItem::Fn(function) => Some(function.sig.ident.to_string()),
                            _ => None,
                        })
                        .collect(),
                });
            }
        });
        found
    }
}

/// Visits every item, descending into inline modules.
fn walk_items<'a>(items: &'a [Item], visit: &mut impl FnMut(&'a Item)) {
    for item in items {
        visit(item);
        if let Item::Mod(module) = item
            && let Some((_, inner)) = &module.content
        {
            walk_items(inner, visit);
        }
    }
}
