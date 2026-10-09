//! The naming and type filters every template uses instead of hand-rolled
//! transforms: `snake`, `pascal`, `camel`, `kebab`, `singular`, `plural`,
//! `rust_type`, `sql_type`, `rust_ident`, and `braces`.

use std::collections::HashMap;

use heck::{ToKebabCase, ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};
use serde_json::Value;

use crate::Registry;

/// Rust keywords that can be written as raw identifiers (`r#type`).
const RAW_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "dyn", "else", "enum", "extern", "false",
    "fn", "for", "gen", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "static", "struct", "trait", "true", "type", "unsafe", "use", "where",
    "while", "abstract", "become", "box", "do", "final", "macro", "override", "priv", "try",
    "typeof", "unsized", "virtual", "yield",
];

/// Keywords that can't be raw identifiers at all.
const RESERVED: &[&str] = &["self", "Self", "super", "crate", "_"];

pub fn register(engine: &mut impl Registry) {
    string_filter(engine, "snake", |s| Ok(s.to_snake_case()));
    string_filter(engine, "pascal", |s| Ok(s.to_upper_camel_case()));
    string_filter(engine, "camel", |s| Ok(s.to_lower_camel_case()));
    string_filter(engine, "kebab", |s| Ok(s.to_kebab_case()));
    string_filter(engine, "singular", |s| Ok(singular(s)));
    string_filter(engine, "plural", |s| Ok(plural(s)));
    string_filter(engine, "rust_type", |s| Ok(rust_type(s)));
    string_filter(engine, "sql_type", sql_type);
    string_filter(engine, "rust_ident", rust_ident);
    engine.filter("braces", braces);
}

fn string_filter<F>(engine: &mut impl Registry, name: &'static str, transform: F)
where
    F: Fn(&str) -> Result<String, String> + Send + Sync + 'static,
{
    engine.filter(name, move |value: &Value, _: &HashMap<String, Value>| {
        let text = value
            .as_str()
            .ok_or_else(|| tera::Error::msg(format!("`{name}` expects a string, got {value}")))?;
        transform(text)
            .map(Value::String)
            .map_err(|message| tera::Error::msg(format!("`{name}`: {message}")))
    });
}

/// Field-type shorthands from payloads (`"uuid"`, `"datetime"`) to Rust
/// types; anything else is assumed to already be a Rust type.
#[must_use]
pub fn rust_type(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "string" | "text" | "str" => "String".to_string(),
        "uuid" => "Uuid".to_string(),
        "int" | "integer" | "i32" => "i32".to_string(),
        "bigint" | "long" | "i64" => "i64".to_string(),
        "bool" | "boolean" => "bool".to_string(),
        "float" | "double" | "f64" => "f64".to_string(),
        "datetime" | "timestamp" => "DateTime<Utc>".to_string(),
        "date" => "NaiveDate".to_string(),
        "json" => "serde_json::Value".to_string(),
        _ => name.to_string(),
    }
}

/// Postgres column types for the Rust types `rust_type` produces.
/// Nullability (`Option<T>`) maps to the inner type; the template decides
/// `NOT NULL`.
///
/// # Errors
///
/// A message when there is no Postgres equivalent.
pub fn sql_type(name: &str) -> Result<String, String> {
    let rust = rust_type(name.trim());
    let inner = rust
        .strip_prefix("Option<")
        .and_then(|rest| rest.strip_suffix('>'))
        .unwrap_or(&rust)
        .trim();
    let sql = match inner {
        "String" => "TEXT",
        "Uuid" => "UUID",
        "i16" => "SMALLINT",
        "i32" => "INTEGER",
        "i64" => "BIGINT",
        "bool" => "BOOLEAN",
        "f32" => "REAL",
        "f64" => "DOUBLE PRECISION",
        "DateTime<Utc>" => "TIMESTAMPTZ",
        "NaiveDate" => "DATE",
        "serde_json::Value" => "JSONB",
        "Vec<u8>" => "BYTEA",
        other => return Err(format!("no Postgres type for `{other}`")),
    };
    Ok(sql.to_string())
}

/// A Rust identifier for `name`: keywords become raw identifiers (`r#type`).
///
/// # Errors
///
/// A message for names that can never be identifiers (`self`, `1st`, ...).
pub fn rust_ident(name: &str) -> Result<String, String> {
    if RESERVED.contains(&name) {
        return Err(format!("`{name}` can't be used as an identifier"));
    }
    if RAW_KEYWORDS.contains(&name) {
        return Ok(format!("r#{name}"));
    }
    let valid_start = name
        .chars()
        .next()
        .is_some_and(|c| c == '_' || c.is_alphabetic());
    if valid_start && name.chars().all(|c| c == '_' || c.is_alphanumeric()) {
        Ok(name.to_string())
    } else {
        Err(format!("`{name}` isn't a valid identifier"))
    }
}

/// `"A"` → `{A}`; `["A", "B"]` → `{A, B}`. Avoids writing `{{{` in Tera.
#[allow(clippy::unnecessary_wraps)]
fn braces(value: &Value, _: &HashMap<String, Value>) -> tera::Result<Value> {
    let inner = match value {
        Value::Array(items) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map_or_else(|| item.to_string(), str::to_string)
            })
            .collect::<Vec<_>>()
            .join(", "),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    Ok(Value::String(format!("{{{inner}}}")))
}

/// `posts` → `post`.
#[must_use]
pub fn singular(word: &str) -> String {
    pluralizer::pluralize(word, 1, false)
}

/// `post` → `posts`.
#[must_use]
pub fn plural(word: &str) -> String {
    pluralizer::pluralize(word, 2, false)
}
