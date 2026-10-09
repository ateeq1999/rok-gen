//! `import(path="...")` / `imports()`: collect `use` lines from anywhere in
//! a template (including components and includes) and render them once,
//! deduplicated, sorted and grouped std → external → crate, at the
//! `imports()` call (or at the top of the output when there is none).
//!
//! Tera functions are stateless, so both functions emit sentinels that
//! [`resolve`] rewrites after rendering.

use std::collections::{BTreeSet, HashMap};

use serde_json::Value;

use crate::Registry;

const IMPORT_OPEN: &str = "\u{1}rok-import:";
const SENTINEL_CLOSE: char = '\u{1}';
const PLACEHOLDER: &str = "\u{1}rok-imports\u{1}";

pub fn register(engine: &mut impl Registry) {
    engine.function("import", |args: &HashMap<String, Value>| {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .ok_or_else(|| tera::Error::msg("`import` needs a `path` string argument"))?;
        let path = path.trim_start_matches("use ").trim_end_matches(';').trim();
        Ok(Value::String(format!(
            "{IMPORT_OPEN}{path}{SENTINEL_CLOSE}"
        )))
    });
    engine.function("imports", |_: &HashMap<String, Value>| {
        Ok(Value::String(PLACEHOLDER.to_string()))
    });
}

/// Strips every import sentinel (dropping lines left blank by it) and
/// writes the collected `use` block in place of the placeholder.
pub fn resolve(raw: &str) -> String {
    let mut paths = BTreeSet::new();
    let mut lines = Vec::new();
    for line in raw.split_inclusive('\n') {
        let mut kept = String::with_capacity(line.len());
        let mut rest = line;
        let mut had_sentinel = false;
        while let Some(start) = rest.find(IMPORT_OPEN) {
            kept.push_str(&rest[..start]);
            let after = &rest[start + IMPORT_OPEN.len()..];
            let end = after.find(SENTINEL_CLOSE).unwrap_or(after.len());
            paths.insert(after[..end].to_string());
            rest = after
                .get(end + SENTINEL_CLOSE.len_utf8()..)
                .unwrap_or_default();
            had_sentinel = true;
        }
        kept.push_str(rest);
        if !(had_sentinel && kept.trim().is_empty()) {
            lines.push(kept);
        }
    }
    let body: String = lines.concat();

    let block = use_block(&paths);
    if body.contains(PLACEHOLDER) {
        return body.replace(PLACEHOLDER, block.trim_end_matches('\n'));
    }
    if block.is_empty() {
        body
    } else {
        format!("{block}\n{}", body.trim_start_matches('\n'))
    }
}

fn use_block(paths: &BTreeSet<String>) -> String {
    let mut groups: [Vec<&str>; 3] = Default::default();
    for path in paths {
        let root = path.split("::").next().unwrap_or_default();
        let group = match root {
            "std" | "core" | "alloc" => 0,
            "crate" | "super" | "self" => 2,
            _ => 1,
        };
        groups[group].push(path);
    }
    groups
        .iter()
        .filter(|group| !group.is_empty())
        .map(|group| {
            group.iter().fold(String::new(), |mut block, path| {
                block.push_str("use ");
                block.push_str(path);
                block.push_str(";\n");
                block
            })
        })
        .collect::<Vec<_>>()
        .join("\n")
}
