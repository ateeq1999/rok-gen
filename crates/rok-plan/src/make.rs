//! `cargo rok make:*` arguments.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One entity/DTO field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FieldSpec {
    /// `snake_case` field (and column) name.
    pub name: String,
    /// A shorthand (`string`, `text`, `uuid`, `int`, `bigint`, `bool`,
    /// `float`, `datetime`, `date`, `json`) or a Rust type.
    #[serde(rename = "type")]
    pub ty: String,
    /// Nullable column / `Option<T>` field.
    #[serde(default)]
    pub optional: bool,
}

impl FieldSpec {
    /// Parses the `--field` flag form: `name:type`, with a trailing `?` on
    /// the type for optional (`summary:string?`).
    ///
    /// # Errors
    ///
    /// A message when the spec has no `:` or an empty part.
    pub fn parse_flag(spec: &str) -> Result<Self, String> {
        let (name, ty) = spec
            .split_once(':')
            .ok_or_else(|| format!("`{spec}`: expected `name:type`, e.g. `title:string`"))?;
        let (ty, optional) = ty
            .strip_suffix('?')
            .map_or((ty, false), |inner| (inner, true));
        if name.trim().is_empty() || ty.trim().is_empty() {
            return Err(format!("`{spec}`: name and type can't be empty"));
        }
        Ok(Self {
            name: name.trim().to_string(),
            ty: ty.trim().to_string(),
            optional,
        })
    }
}

/// Who may call generated routes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Guard {
    /// Public.
    None,
    /// Behind `session::require_auth`.
    #[default]
    Auth,
    /// Behind `require_auth`, plus a `session::AdminUser` extractor.
    Admin,
}

/// HTTP method of a generated action.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    #[default]
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Post => "post",
            Self::Put => "put",
            Self::Patch => "patch",
            Self::Delete => "delete",
        }
    }
}

/// Whether a DTO is read from a request body or written to a response.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DtoKind {
    #[default]
    Request,
    Response,
}

/// Arguments for `cargo rok make:feature`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MakeFeatureArgs {
    /// Plural, `snake_case` crate name, e.g. `posts`.
    pub name: String,
    /// Entity fields, in column order (`id` and `created_at` are added).
    pub fields: Vec<FieldSpec>,
    /// Who may call the generated CRUD routes.
    pub guard: Guard,
    /// Print the diff; write nothing.
    pub dry_run: bool,
    /// Run `cargo check` after writing; roll everything back if it fails.
    pub verify: bool,
}

/// Arguments for `cargo rok make:action`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MakeActionArgs {
    /// The feature crate, e.g. `posts`.
    pub feature: String,
    /// `snake_case` handler name, e.g. `publish_post`.
    pub name: String,
    pub method: Method,
    /// Route path relative to the module prefix, e.g. `/{id}/publish`.
    pub path: String,
    /// Extra guard for this handler. `admin` adds an `AdminUser`
    /// extractor; route-level auth comes from the module.
    pub guard: Guard,
    /// Request body DTO (`PascalCase`). Generated when it doesn't exist.
    pub body: Option<String>,
    pub dry_run: bool,
    pub verify: bool,
}

/// Arguments for `cargo rok make:dto`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MakeDtoArgs {
    /// The feature crate, e.g. `posts`.
    pub feature: String,
    /// `PascalCase` type name, e.g. `PublishPostRequest`.
    pub name: String,
    pub kind: DtoKind,
    pub fields: Vec<FieldSpec>,
    pub dry_run: bool,
    pub verify: bool,
}

/// Arguments for `cargo rok make:migration`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MakeMigrationArgs {
    /// `snake_case` description, e.g. `add_post_slug`.
    pub name: String,
    /// The migration body. A commented placeholder when empty.
    pub sql: String,
    pub dry_run: bool,
}

/// Arguments for `cargo rok make:entity`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MakeEntityArgs {
    /// The feature crate, e.g. `posts`.
    pub feature: String,
    /// `snake_case` entity name, e.g. `revision`.
    pub name: String,
    pub fields: Vec<FieldSpec>,
    pub dry_run: bool,
    pub verify: bool,
}

/// Arguments for `cargo rok make:repository`: a new entity with its port,
/// Postgres adapter, in-memory fake and table.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MakeRepositoryArgs {
    /// The feature crate, e.g. `posts`.
    pub feature: String,
    /// `snake_case` entity name, e.g. `revision`.
    pub name: String,
    pub fields: Vec<FieldSpec>,
    /// Table name. Defaults to the plural of `name`.
    pub table: Option<String>,
    pub dry_run: bool,
    pub verify: bool,
}

/// Arguments for `cargo rok make:service`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MakeServiceArgs {
    /// The feature crate, e.g. `posts`.
    pub feature: String,
    /// `snake_case` service name, e.g. `publishing` → `PublishingService`.
    pub name: String,
    /// Entities (in this feature) whose repository ports the service
    /// depends on, e.g. `["post", "revision"]`.
    pub repositories: Vec<String>,
    pub dry_run: bool,
    pub verify: bool,
}

/// Arguments for `cargo rok make:error`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MakeErrorArgs {
    /// The feature, e.g. `posts` → `PostsError`.
    pub feature: String,
    pub dry_run: bool,
    pub verify: bool,
}

/// Arguments for `cargo rok new`: create a new project from the reference template.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct NewArgs {
    /// Project name (also the workspace package name).
    pub name: String,
    /// Recipes to include, e.g. `["auth", "mail"]`.
    pub recipes: Vec<String>,
    /// Driver choices for recipes, e.g. `{"auth": "session", "mail": "resend"}`.
    pub drivers: std::collections::BTreeMap<String, String>,
    /// Initialize git repository.
    #[serde(default = "default_true")]
    pub git: bool,
    /// Create GitHub Actions CI workflow.
    #[serde(default = "default_true")]
    pub ci: bool,
    /// Install pre-commit hook.
    #[serde(default = "default_true")]
    pub hooks: bool,
    /// Create supply-chain config (deny.toml).
    #[serde(default = "default_true")]
    pub supply_chain: bool,
    /// Set up project skills.
    #[serde(default = "default_true")]
    pub skills: bool,
    /// Print the diff; write nothing.
    pub dry_run: bool,
    /// Run `cargo check` after writing; roll everything back if it fails.
    pub verify: bool,
}

fn default_true() -> bool {
    true
}
