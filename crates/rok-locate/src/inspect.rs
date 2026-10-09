//! `cargo rok inspect`: every addressable place in a file.

use serde::Serialize;
use syn::{Block, Expr, ImplItem, Item, Stmt};

use crate::select::kind_of;
use crate::{LocateError, Source};

#[derive(Clone, Debug, Serialize)]
pub struct Inspection {
    pub structs: Vec<StructInfo>,
    pub functions: Vec<FnInfo>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StructInfo {
    pub name: String,
    pub line: usize,
    pub fields: Vec<FieldInfo>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FieldInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub line: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct FnInfo {
    /// `run` or `PostService::delete`: what `fn` in a selector takes.
    pub name: String,
    pub line: usize,
    pub signature: String,
    pub statements: Vec<StmtInfo>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StmtInfo {
    pub index: usize,
    pub line: usize,
    /// The statement's first line.
    pub text: String,
    pub tail: bool,
    /// When the statement is an `if`/`match`/loop/closure/block: the
    /// `within` step that enters it, e.g. `{"if": 0}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub within: Option<serde_json::Value>,
}

impl Source {
    /// Structs, fields, functions and their numbered statements.
    /// `function` keeps just that one.
    ///
    /// # Errors
    ///
    /// [`LocateError::NotFound`] when `function` names nothing.
    pub fn inspect(&self, function: Option<&str>) -> Result<Inspection, LocateError> {
        let mut structs = Vec::new();
        let mut functions = Vec::new();
        for item in &self.file.items {
            match item {
                Item::Struct(item) if function.is_none() => structs.push(StructInfo {
                    name: item.ident.to_string(),
                    line: item.ident.span().start().line,
                    fields: item
                        .fields
                        .iter()
                        .filter_map(|field| {
                            field.ident.as_ref().map(|ident| FieldInfo {
                                name: ident.to_string(),
                                ty: crate::select::squash_type(self.snippet(&field.ty)),
                                line: ident.span().start().line,
                            })
                        })
                        .collect(),
                }),
                Item::Fn(item) => {
                    functions.push(self.fn_info(
                        item.sig.ident.to_string(),
                        &item.sig,
                        &item.block,
                    ));
                }
                Item::Impl(implementation) => {
                    let syn::Type::Path(self_ty) = &*implementation.self_ty else {
                        continue;
                    };
                    let Some(owner) = self_ty.path.segments.last() else {
                        continue;
                    };
                    for impl_item in &implementation.items {
                        if let ImplItem::Fn(method) = impl_item {
                            functions.push(self.fn_info(
                                format!("{}::{}", owner.ident, method.sig.ident),
                                &method.sig,
                                &method.block,
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(wanted) = function {
            functions.retain(|info| info.name == wanted);
            if functions.is_empty() {
                return Err(LocateError::NotFound(format!(
                    "no function `{wanted}`; available: {}",
                    self.function_names().join(", ")
                )));
            }
        }
        Ok(Inspection { structs, functions })
    }

    fn fn_info(&self, name: String, sig: &syn::Signature, block: &Block) -> FnInfo {
        let site = self.block_site(block);
        let mut seen: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
        let statements = site
            .stmts
            .iter()
            .zip(&block.stmts)
            .map(|(stmt_site, stmt)| {
                let expr = match stmt {
                    Stmt::Expr(expr, _) => Some(expr),
                    Stmt::Local(local) => local.init.as_ref().map(|init| &*init.expr),
                    _ => None,
                };
                let within = expr.and_then(|expr: &Expr| kind_of(expr)).map(|kind| {
                    let count = seen.entry(kind).or_insert(0);
                    let step = serde_json::json!({ kind: *count });
                    *count += 1;
                    step
                });
                StmtInfo {
                    index: stmt_site.index,
                    line: stmt_site.line,
                    text: stmt_site
                        .text
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                    tail: stmt_site.tail,
                    within,
                }
            })
            .collect();
        FnInfo {
            name,
            line: sig.ident.span().start().line,
            signature: crate::select::squash_type(self.snippet(sig)),
            statements,
        }
    }
}

impl Inspection {
    /// The human-readable listing.
    #[must_use]
    pub fn render(&self) -> String {
        use std::fmt::Write as _;

        let mut text = String::new();
        for info in &self.structs {
            let _ = writeln!(text, "struct {}  L{}", info.name, info.line);
            for field in &info.fields {
                let _ = writeln!(text, "    {}: {}  L{}", field.name, field.ty, field.line);
            }
        }
        for info in &self.functions {
            let _ = writeln!(text, "fn {}  L{}", info.name, info.line);
            for stmt in &info.statements {
                let tail = if stmt.tail { "  <tail>" } else { "" };
                let within = stmt
                    .within
                    .as_ref()
                    .map(|step| format!("  within {step}"))
                    .unwrap_or_default();
                let _ = writeln!(
                    text,
                    "    [{}] L{}  {}{tail}{within}",
                    stmt.index, stmt.line, stmt.text
                );
            }
        }
        text
    }
}
