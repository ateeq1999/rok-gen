//! Every name a generator derives from user input, computed once.

use heck::ToUpperCamelCase;
use rok_plan::make::FieldSpec;
use rok_render::naming;
use serde::Serialize;

use super::GenError;

/// Rust types (as `rok_render::naming::rust_type` produces them) that are
/// `Copy`, so generated code moves them instead of cloning.
const COPY_TYPES: &[&str] = &[
    "Uuid",
    "i16",
    "i32",
    "i64",
    "bool",
    "f32",
    "f64",
    "DateTime<Utc>",
    "NaiveDate",
];

/// Columns every generated table already has.
const RESERVED_FIELDS: &[&str] = &["id", "created_at"];

/// Names for one feature crate, e.g. `posts`.
#[derive(Clone, Debug, Serialize)]
#[allow(non_snake_case)]
pub struct FeatureNames {
    /// Crate, table, route prefix and service field: `posts`.
    pub feature: String,
    /// File prefix: `post`.
    pub s: String,
    /// Type prefix: `Post`.
    pub S: String,
    /// `Posts`.
    pub P: String,
    /// The feature's error type: `PostsError`.
    pub E: String,
    pub table: String,
    /// `OpenAPI` tag: `Posts`.
    pub tag: String,
    /// For prose: `blog post`.
    pub s_words: String,
    /// For prose: `blog posts`.
    pub feature_words: String,
}

impl FeatureNames {
    pub fn new(feature: &str) -> Result<Self, GenError> {
        snake_ident(feature, "feature name")?;
        let s = naming::singular(feature);
        let s = if s.is_empty() { feature.to_string() } else { s };
        let pascal_plural = feature.to_upper_camel_case();
        Ok(Self {
            feature: feature.to_string(),
            S: s.to_upper_camel_case(),
            E: format!("{pascal_plural}Error"),
            tag: pascal_plural.clone(),
            P: pascal_plural,
            table: feature.to_string(),
            s_words: s.replace('_', " "),
            feature_words: feature.replace('_', " "),
            s,
        })
    }

    /// Names for another entity inside `feature`: crate-level names (crate,
    /// error type, tag) from the feature, entity-level ones (file and type
    /// prefix, table, prose) from `entity`, e.g. `revision` in `posts` →
    /// `revision_repository.rs`, `RevisionRepository`, table `revisions`.
    pub fn for_entity(feature: &str, entity: &str, table: Option<&str>) -> Result<Self, GenError> {
        let mut names = Self::new(feature)?;
        snake_ident(entity, "entity name")?;
        let plural = naming::plural(entity);
        if let Some(table) = table {
            snake_ident(table, "table name")?;
        }
        names.s = entity.to_string();
        names.S = entity.to_upper_camel_case();
        names.table = table.map_or_else(|| plural.clone(), str::to_string);
        names.s_words = entity.replace('_', " ");
        names.feature_words = plural.replace('_', " ");
        Ok(names)
    }
}

/// One field, with everything templates need precomputed.
#[derive(Clone, Debug, Serialize)]
// Template context: each flag is a separate yes/no a template branches on.
#[allow(clippy::struct_excessive_bools)]
pub struct Field {
    pub name: String,
    /// The field's type, `Option<_>`-wrapped when optional.
    pub ty: String,
    /// The type without `Option`.
    pub base: String,
    pub sql: String,
    pub optional: bool,
    /// Whether `ty` is `Copy`.
    pub copy: bool,
    /// Whether `base` is `Copy`.
    pub base_copy: bool,
    pub is_string: bool,
    /// A Rust expression of type `ty`, for generated tests.
    pub sample: String,
}

/// Resolves field specs. `require_sql` rejects types with no Postgres column
/// type (entities need one; DTOs don't).
pub fn fields(specs: &[FieldSpec], require_sql: bool) -> Result<Vec<Field>, GenError> {
    let mut seen = Vec::new();
    specs
        .iter()
        .map(|spec| {
            snake_ident(&spec.name, "field name")?;
            if RESERVED_FIELDS.contains(&spec.name.as_str()) {
                return Err(GenError::Invalid(format!(
                    "field `{}` is generated automatically; remove it",
                    spec.name
                )));
            }
            if seen.contains(&spec.name) {
                return Err(GenError::Invalid(format!(
                    "field `{}` is listed twice",
                    spec.name
                )));
            }
            seen.push(spec.name.clone());

            let base = naming::rust_type(&spec.ty);
            let sql = match naming::sql_type(&base) {
                Ok(sql) => sql,
                Err(message) if require_sql => {
                    return Err(GenError::Invalid(format!(
                        "field `{}`: {message}",
                        spec.name
                    )));
                }
                Err(_) => String::new(),
            };
            let base_copy = COPY_TYPES.contains(&base.as_str());
            let sample = if spec.optional {
                "None".to_string()
            } else {
                sample(&base)
            };
            Ok(Field {
                name: spec.name.clone(),
                ty: if spec.optional {
                    format!("Option<{base}>")
                } else {
                    base.clone()
                },
                is_string: base == "String",
                sql,
                optional: spec.optional,
                copy: base_copy,
                base_copy,
                sample,
                base,
            })
        })
        .collect()
}

fn sample(base: &str) -> String {
    match base {
        "String" => "\"example\".to_string()".to_string(),
        "Uuid" => "Uuid::new_v4()".to_string(),
        "i16" | "i32" | "i64" => "1".to_string(),
        "bool" => "true".to_string(),
        "f32" | "f64" => "1.0".to_string(),
        "DateTime<Utc>" => "chrono::Utc::now()".to_string(),
        "NaiveDate" => "chrono::NaiveDate::default()".to_string(),
        "serde_json::Value" => "serde_json::Value::Null".to_string(),
        _ => "Default::default()".to_string(),
    }
}

/// Requires a lowercase `snake_case` identifier that isn't a keyword.
pub fn snake_ident(name: &str, what: &str) -> Result<(), GenError> {
    let valid = name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valid {
        return Err(GenError::Invalid(format!(
            "{what} `{name}` must be snake_case (lowercase letters, digits, `_`)"
        )));
    }
    if naming::rust_ident(name).ok().as_deref() != Some(name) {
        return Err(GenError::Invalid(format!(
            "{what} `{name}` is a Rust keyword; pick another"
        )));
    }
    Ok(())
}

/// Requires a `PascalCase` type name.
pub fn pascal_ident(name: &str, what: &str) -> Result<(), GenError> {
    let valid = name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && name.chars().all(|c| c.is_ascii_alphanumeric());
    if valid {
        Ok(())
    } else {
        Err(GenError::Invalid(format!(
            "{what} `{name}` must be PascalCase (e.g. `PublishPostRequest`)"
        )))
    }
}
