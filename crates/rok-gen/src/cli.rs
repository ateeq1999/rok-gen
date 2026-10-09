//! Command-line shape. Every flag here is optional and only overrides the
//! JSON payload when actually passed; the full argument set (and its
//! defaults) lives in the `rok-plan` types.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{Map, Value};

#[derive(Debug, Parser)]
#[command(
    name = "cargo rok",
    bin_name = "cargo rok",
    version,
    about = "Generators, structural feature integration and quality gates for actix-web API workspaces"
)]
pub struct Cli {
    /// How results are printed.
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Human)]
    pub output: OutputFormat,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the quality gates (all implemented gates when none are named).
    Check(CheckCmd),
    /// Manage git hooks.
    #[command(subcommand)]
    Hooks(HooksCmd),
    /// Print the JSON Schema of a command payload, or of `rok.toml` (`project`).
    Schema {
        /// A command (`check`, `hooks-install`, `make:feature`, ...) or `project`.
        name: String,
    },
    /// Create a new project from the reference template.
    #[command(name = "new")]
    New(NewCmd),
    /// Generate a CRUD feature crate and wire it into the workspace.
    #[command(name = "make:feature")]
    MakeFeature(MakeFeatureCmd),
    /// Add a documented, registered handler stub to a feature.
    #[command(name = "make:action")]
    MakeAction(MakeActionCmd),
    /// Add a DTO to a feature.
    #[command(name = "make:dto")]
    MakeDto(MakeDtoCmd),
    /// Add the next numbered SQL migration.
    #[command(name = "make:migration")]
    MakeMigration(MakeMigrationCmd),
    /// Add an entity (stored, insert and update shapes) to a feature.
    #[command(name = "make:entity")]
    MakeEntity(MakeEntityCmd),
    /// Add an entity with its repository port, Postgres adapter, fake and table.
    #[command(name = "make:repository")]
    MakeRepository(MakeRepositoryCmd),
    /// Add a service over repository ports, wired into the composition root.
    #[command(name = "make:service")]
    MakeService(MakeServiceCmd),
    /// Add a feature's error type to the shared error crate.
    #[command(name = "make:error")]
    MakeError(MakeErrorCmd),
    /// Print every registered route.
    Routes,
    /// List a file's structs, fields, functions and numbered statements.
    Inspect(InspectCmd),
    /// Apply an integration plan (from `-f`/`-j`) as one transaction.
    Integrate(IntegrateCmd),
    /// Run a list of commands (from `-f`/`-j`) as one batch that lands whole or not at all.
    Apply(ApplyCmd),
    /// Revert a recorded transaction (default: the latest).
    Undo(UndoCmd),
    /// Revert every integration of <source> into <target>.
    Unlink(UnlinkCmd),
    /// List the recorded transactions.
    Journal,
}

#[derive(Debug, Args)]
pub struct InspectCmd {
    #[command(flatten)]
    pub input: InputFlags,
    /// A file alias (`posts/service`, `@root`, ...) or path.
    #[arg(value_name = "FILE")]
    pub target: Option<String>,
    /// Only this function (`run`, `PostService::delete`).
    #[arg(long = "fn")]
    pub function: Option<String>,
}

#[derive(Debug, Args)]
pub struct IntegrateCmd {
    #[command(flatten)]
    pub input: InputFlags,
    /// Allow `replace` steps. Command line only: a plan can't set it.
    #[arg(long)]
    pub force: bool,
    /// With --force, replace even in files with uncommitted changes.
    #[arg(long)]
    pub allow_dirty: bool,
    /// Build the plan by answering questions instead of writing JSON.
    #[arg(long, conflicts_with_all = ["preset", "json", "file"])]
    pub wizard: bool,
    /// A built-in plan template, e.g. `has-many`.
    #[arg(long)]
    pub preset: Option<String>,
    /// A template variable as `key=value`, e.g. `source=comments`. Repeatable.
    #[arg(long = "var", value_name = "KEY=VALUE")]
    pub vars: Vec<String>,
    /// Write the (expanded) plan to this path instead of applying it.
    #[arg(long, value_name = "PATH")]
    pub emit_plan: Option<String>,
    #[command(flatten)]
    pub apply: ApplyFlags,
}

#[derive(Debug, Args)]
pub struct ApplyCmd {
    #[command(flatten)]
    pub input: InputFlags,
    /// Allow `replace` steps. Command line only: a batch can't set it.
    #[arg(long)]
    pub force: bool,
    /// With --force, replace even in files with uncommitted changes.
    #[arg(long)]
    pub allow_dirty: bool,
    #[command(flatten)]
    pub apply: ApplyFlags,
}

impl ApplyCmd {
    pub fn flag_layer(&self) -> Value {
        let mut layer = Map::new();
        self.apply.add_to(&mut layer);
        Value::Object(layer)
    }
}

#[derive(Debug, Args)]
pub struct UndoCmd {
    /// The transaction id (see `cargo rok journal`); default: the latest.
    pub id: Option<String>,
    /// Print the diff; write nothing.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct UnlinkCmd {
    /// The integrated feature, e.g. `comments`.
    pub source: String,
    /// The feature it was integrated into, e.g. `posts`.
    pub target: String,
    /// Print the diff; write nothing.
    #[arg(long)]
    pub dry_run: bool,
}

impl InspectCmd {
    pub fn flag_layer(&self) -> Value {
        let mut layer = Map::new();
        str_into(&mut layer, "file", self.target.as_deref());
        str_into(&mut layer, "fn", self.function.as_deref());
        Value::Object(layer)
    }
}

impl IntegrateCmd {
    /// # Errors
    ///
    /// A message for a malformed `--var`.
    pub fn flag_layer(&self) -> Result<Value, String> {
        let mut layer = Map::new();
        str_into(&mut layer, "preset", self.preset.as_deref());
        str_into(&mut layer, "emit_plan", self.emit_plan.as_deref());
        if !self.vars.is_empty() {
            let mut vars = Map::new();
            for var in &self.vars {
                let (key, value) = var
                    .split_once('=')
                    .ok_or_else(|| format!("`{var}`: expected `key=value`"))?;
                vars.insert(
                    key.trim().to_string(),
                    Value::String(value.trim().to_string()),
                );
            }
            layer.insert("vars".to_string(), Value::Object(vars));
        }
        self.apply.add_to(&mut layer);
        Ok(Value::Object(layer))
    }
}

#[derive(Debug, Args)]
pub struct MakeEntityCmd {
    #[command(flatten)]
    pub input: InputFlags,
    /// The feature crate, e.g. `posts`.
    pub feature: Option<String>,
    /// `snake_case` entity name, e.g. `revision`.
    pub name: Option<String>,
    /// A field as `name:type`, `?` suffix for optional. Repeatable.
    #[arg(long = "field", value_name = "NAME:TYPE")]
    pub fields: Vec<String>,
    #[command(flatten)]
    pub apply: ApplyFlags,
}

#[derive(Debug, Args)]
pub struct MakeRepositoryCmd {
    #[command(flatten)]
    pub input: InputFlags,
    /// The feature crate, e.g. `posts`.
    pub feature: Option<String>,
    /// `snake_case` entity name, e.g. `revision`.
    pub name: Option<String>,
    /// A field as `name:type`, `?` suffix for optional. Repeatable.
    #[arg(long = "field", value_name = "NAME:TYPE")]
    pub fields: Vec<String>,
    /// Table name (default: the plural of the entity name).
    #[arg(long)]
    pub table: Option<String>,
    #[command(flatten)]
    pub apply: ApplyFlags,
}

#[derive(Debug, Args)]
pub struct MakeServiceCmd {
    #[command(flatten)]
    pub input: InputFlags,
    /// The feature crate, e.g. `posts`.
    pub feature: Option<String>,
    /// `snake_case` service name, e.g. `publishing`.
    pub name: Option<String>,
    /// An entity whose repository the service depends on. Repeatable.
    #[arg(long = "repository", value_name = "ENTITY")]
    pub repositories: Vec<String>,
    #[command(flatten)]
    pub apply: ApplyFlags,
}

#[derive(Debug, Args)]
pub struct MakeErrorCmd {
    #[command(flatten)]
    pub input: InputFlags,
    /// The feature, e.g. `posts` → `PostsError`.
    pub feature: Option<String>,
    #[command(flatten)]
    pub apply: ApplyFlags,
}

impl MakeEntityCmd {
    /// # Errors
    ///
    /// A message for a malformed `--field`.
    pub fn flag_layer(&self) -> Result<Value, String> {
        let mut layer = Map::new();
        str_into(&mut layer, "feature", self.feature.as_deref());
        str_into(&mut layer, "name", self.name.as_deref());
        fields_into(&mut layer, &self.fields)?;
        self.apply.add_to(&mut layer);
        Ok(Value::Object(layer))
    }
}

impl MakeRepositoryCmd {
    /// # Errors
    ///
    /// A message for a malformed `--field`.
    pub fn flag_layer(&self) -> Result<Value, String> {
        let mut layer = Map::new();
        str_into(&mut layer, "feature", self.feature.as_deref());
        str_into(&mut layer, "name", self.name.as_deref());
        fields_into(&mut layer, &self.fields)?;
        str_into(&mut layer, "table", self.table.as_deref());
        self.apply.add_to(&mut layer);
        Ok(Value::Object(layer))
    }
}

impl MakeServiceCmd {
    pub fn flag_layer(&self) -> Value {
        let mut layer = Map::new();
        str_into(&mut layer, "feature", self.feature.as_deref());
        str_into(&mut layer, "name", self.name.as_deref());
        if !self.repositories.is_empty() {
            layer.insert(
                "repositories".to_string(),
                Value::Array(
                    self.repositories
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
        self.apply.add_to(&mut layer);
        Value::Object(layer)
    }
}

impl MakeErrorCmd {
    pub fn flag_layer(&self) -> Value {
        let mut layer = Map::new();
        str_into(&mut layer, "feature", self.feature.as_deref());
        self.apply.add_to(&mut layer);
        Value::Object(layer)
    }
}

/// `--dry-run` / `--verify`, shared by the generators.
#[derive(Debug, Args)]
pub struct ApplyFlags {
    /// Print the diff; write nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Run `cargo check` after writing; roll back if it fails.
    #[arg(long)]
    pub verify: bool,
}

#[derive(Debug, Args)]
pub struct MakeFeatureCmd {
    #[command(flatten)]
    pub input: InputFlags,
    /// Plural, `snake_case` crate name, e.g. `posts`.
    pub name: Option<String>,
    /// An entity field as `name:type`, `?` suffix for optional (`summary:string?`). Repeatable.
    #[arg(long = "field", value_name = "NAME:TYPE")]
    pub fields: Vec<String>,
    /// none, auth or admin.
    #[arg(long)]
    pub guard: Option<String>,
    #[command(flatten)]
    pub apply: ApplyFlags,
}

#[derive(Debug, Args)]
pub struct MakeActionCmd {
    #[command(flatten)]
    pub input: InputFlags,
    /// The feature crate, e.g. `posts`.
    pub feature: Option<String>,
    /// `snake_case` handler name, e.g. `publish_post`.
    pub name: Option<String>,
    /// get, post, put, patch or delete.
    #[arg(long)]
    pub method: Option<String>,
    /// Route path relative to the module prefix, e.g. `/{id}/publish`.
    #[arg(long)]
    pub path: Option<String>,
    /// `admin` adds an `AdminUser` extractor.
    #[arg(long)]
    pub guard: Option<String>,
    /// Request body DTO; generated when it doesn't exist.
    #[arg(long)]
    pub body: Option<String>,
    #[command(flatten)]
    pub apply: ApplyFlags,
}

#[derive(Debug, Args)]
pub struct MakeDtoCmd {
    #[command(flatten)]
    pub input: InputFlags,
    /// The feature crate, e.g. `posts`.
    pub feature: Option<String>,
    /// `PascalCase` type name, e.g. `PublishPostRequest`.
    pub name: Option<String>,
    /// request or response.
    #[arg(long)]
    pub kind: Option<String>,
    /// A field as `name:type`, `?` suffix for optional. Repeatable.
    #[arg(long = "field", value_name = "NAME:TYPE")]
    pub fields: Vec<String>,
    #[command(flatten)]
    pub apply: ApplyFlags,
}

#[derive(Debug, Args)]
pub struct MakeMigrationCmd {
    #[command(flatten)]
    pub input: InputFlags,
    /// `snake_case` description, e.g. `add_post_slug`.
    pub name: Option<String>,
    /// The migration body.
    #[arg(long)]
    pub sql: Option<String>,
    /// Print the diff; write nothing.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct NewCmd {
    #[command(flatten)]
    pub input: InputFlags,
    /// Project name (also the workspace package name).
    pub name: Option<String>,
    /// Recipes to include, e.g. `auth`, `mail`. Repeatable.
    #[arg(long, value_name = "RECIPE")]
    pub recipe: Vec<String>,
    /// Driver choices for recipes, e.g. `auth=session`, `mail=resend`. Repeatable.
    #[arg(long = "driver", value_name = "RECIPE=DRIVER")]
    pub drivers: Vec<String>,
    /// Skip git repository initialization.
    #[arg(long)]
    pub no_git: bool,
    /// Skip GitHub Actions CI workflow.
    #[arg(long)]
    pub no_ci: bool,
    /// Skip pre-commit hook installation.
    #[arg(long)]
    pub no_hooks: bool,
    /// Skip supply-chain config (deny.toml).
    #[arg(long)]
    pub no_supply_chain: bool,
    /// Skip project skills setup.
    #[arg(long)]
    pub no_skills: bool,
    /// Print the diff; write nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Run `cargo check` after writing; roll everything back if it fails.
    #[arg(long)]
    pub verify: bool,
}

impl NewCmd {
    /// # Errors
    ///
    /// A message for a malformed `--driver`.
    pub fn flag_layer(&self) -> Result<Value, String> {
        let mut layer = Map::new();
        str_into(&mut layer, "name", self.name.as_deref());
        if !self.recipe.is_empty() {
            layer.insert(
                "recipes".to_string(),
                Value::Array(
                    self.recipe
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
        if !self.drivers.is_empty() {
            let mut drivers = Map::new();
            for driver in &self.drivers {
                let (key, value) = driver
                    .split_once('=')
                    .ok_or_else(|| format!("`{driver}`: expected `recipe=driver`"))?;
                drivers.insert(
                    key.trim().to_string(),
                    Value::String(value.trim().to_string()),
                );
            }
            layer.insert("drivers".to_string(), Value::Object(drivers));
        }
        bool_into(&mut layer, "git", !self.no_git);
        bool_into(&mut layer, "ci", !self.no_ci);
        bool_into(&mut layer, "hooks", !self.no_hooks);
        bool_into(&mut layer, "supply_chain", !self.no_supply_chain);
        bool_into(&mut layer, "skills", !self.no_skills);
        bool_into(&mut layer, "dry_run", self.dry_run);
        bool_into(&mut layer, "verify", self.verify);
        Ok(Value::Object(layer))
    }
}

#[derive(Debug, Subcommand)]
pub enum HooksCmd {
    /// Install a pre-commit hook that runs the fast gates.
    Install(HooksInstallCmd),
}

/// The input flags every payload-taking command shares.
#[derive(Debug, Args)]
pub struct InputFlags {
    /// The payload as inline JSON.
    #[arg(short = 'j', long = "json", value_name = "JSON")]
    pub json: Option<String>,
    /// The payload from a JSON/JSONC, YAML or TOML file, or `-` for stdin.
    #[arg(short = 'f', long = "file", value_name = "PATH")]
    pub file: Option<PathBuf>,
    /// Print the merged payload and exit without running the command.
    #[arg(long)]
    pub print_payload: bool,
}

#[derive(Debug, Args)]
pub struct CheckCmd {
    #[command(flatten)]
    pub input: InputFlags,
    /// Gates to run: fmt, lint, test, deny, machete, sqlx, openapi, arch, coverage.
    #[arg(value_name = "GATE")]
    pub gates: Vec<String>,
    /// Keep running after a gate fails.
    #[arg(long)]
    pub keep_going: bool,
    /// Emit GitHub Actions annotations.
    #[arg(long)]
    pub ci: bool,
}

#[derive(Debug, Args)]
pub struct HooksInstallCmd {
    #[command(flatten)]
    pub input: InputFlags,
    /// Gates the hook runs (default: fmt lint arch).
    #[arg(value_name = "GATE")]
    pub gates: Vec<String>,
    /// Replace an existing pre-commit hook that rok didn't install.
    #[arg(long)]
    pub overwrite: bool,
}

impl CheckCmd {
    /// Only the flags that were passed, as a payload layer.
    pub fn flag_layer(&self) -> Value {
        let mut layer = Map::new();
        gates_into(&mut layer, &self.gates);
        bool_into(&mut layer, "keep_going", self.keep_going);
        bool_into(&mut layer, "ci", self.ci);
        Value::Object(layer)
    }
}

impl HooksInstallCmd {
    pub fn flag_layer(&self) -> Value {
        let mut layer = Map::new();
        gates_into(&mut layer, &self.gates);
        bool_into(&mut layer, "overwrite", self.overwrite);
        Value::Object(layer)
    }
}

impl MakeFeatureCmd {
    /// # Errors
    ///
    /// A message for a malformed `--field`.
    pub fn flag_layer(&self) -> Result<Value, String> {
        let mut layer = Map::new();
        str_into(&mut layer, "name", self.name.as_deref());
        fields_into(&mut layer, &self.fields)?;
        str_into(&mut layer, "guard", self.guard.as_deref());
        self.apply.add_to(&mut layer);
        Ok(Value::Object(layer))
    }
}

impl MakeActionCmd {
    pub fn flag_layer(&self) -> Value {
        let mut layer = Map::new();
        str_into(&mut layer, "feature", self.feature.as_deref());
        str_into(&mut layer, "name", self.name.as_deref());
        str_into(&mut layer, "method", self.method.as_deref());
        str_into(&mut layer, "path", self.path.as_deref());
        str_into(&mut layer, "guard", self.guard.as_deref());
        str_into(&mut layer, "body", self.body.as_deref());
        self.apply.add_to(&mut layer);
        Value::Object(layer)
    }
}

impl MakeDtoCmd {
    /// # Errors
    ///
    /// A message for a malformed `--field`.
    pub fn flag_layer(&self) -> Result<Value, String> {
        let mut layer = Map::new();
        str_into(&mut layer, "feature", self.feature.as_deref());
        str_into(&mut layer, "name", self.name.as_deref());
        str_into(&mut layer, "kind", self.kind.as_deref());
        fields_into(&mut layer, &self.fields)?;
        self.apply.add_to(&mut layer);
        Ok(Value::Object(layer))
    }
}

impl MakeMigrationCmd {
    pub fn flag_layer(&self) -> Value {
        let mut layer = Map::new();
        str_into(&mut layer, "name", self.name.as_deref());
        str_into(&mut layer, "sql", self.sql.as_deref());
        bool_into(&mut layer, "dry_run", self.dry_run);
        Value::Object(layer)
    }
}

impl ApplyFlags {
    fn add_to(&self, layer: &mut Map<String, Value>) {
        bool_into(layer, "dry_run", self.dry_run);
        bool_into(layer, "verify", self.verify);
    }
}

fn str_into(layer: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        layer.insert(key.to_string(), Value::String(value.to_string()));
    }
}

fn fields_into(layer: &mut Map<String, Value>, fields: &[String]) -> Result<(), String> {
    if fields.is_empty() {
        return Ok(());
    }
    let parsed = fields
        .iter()
        .map(|spec| {
            rok_plan::make::FieldSpec::parse_flag(spec)
                .map(|field| serde_json::to_value(field).unwrap_or_default())
        })
        .collect::<Result<Vec<_>, _>>()?;
    layer.insert("fields".to_string(), Value::Array(parsed));
    Ok(())
}

fn gates_into(layer: &mut Map<String, Value>, gates: &[String]) {
    if !gates.is_empty() {
        layer.insert(
            "gates".to_string(),
            Value::Array(gates.iter().cloned().map(Value::String).collect()),
        );
    }
}

fn bool_into(layer: &mut Map<String, Value>, key: &str, set: bool) {
    if set {
        layer.insert(key.to_string(), Value::Bool(true));
    }
}
