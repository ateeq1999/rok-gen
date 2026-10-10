//! Recipes: `cargo rok add`, `cargo rok remove` and `cargo rok list`.
//!
//! A recipe is an optional capability (auth, mail, storage) shipped as one
//! workspace crate, `crates/<recipe>`, rendered for the chosen driver. Adding
//! one also declares it (and the third-party crates its driver needs) in
//! `[workspace.dependencies]`, appends its env keys to `.env.example` (never
//! to `.env`), optionally adds the driver's Terraform module, and records it
//! under `[recipes.<name>]` in `rok.toml`. Removing one reverses all of that,
//! and refuses while another recipe or crate still depends on it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;
use std::str::FromStr;

use rok_edit::Transaction;
use rok_plan::outcome::Outcome;
use rok_plan::project::{CONFIG_FILE, InstalledRecipe, Project};
use rok_plan::recipe::{AddArgs, RemoveArgs};
use rok_render::{Renderer, SyntaxKind};
use serde_json::json;

use super::{Apply, GenError, Generated, finish_labeled, manifest, templates};

/// The example env file recipes add their keys to.
const ENV_EXAMPLE: &str = ".env.example";
/// Where a recipe's Terraform module goes, under its name.
const TERRAFORM_DIR: &str = "infra/terraform/modules";

/// A built-in recipe.
pub struct Recipe {
    pub name: &'static str,
    pub version: &'static str,
    pub summary: &'static str,
    /// Recipes that must be installed first.
    pub requires: &'static [&'static str],
    /// The first one is the default.
    pub drivers: &'static [Driver],
    /// `[workspace.dependencies]` entries every driver needs, as
    /// `(key, spec)`.
    pub deps: &'static [(&'static str, &'static str)],
}

/// One way to back a recipe.
pub struct Driver {
    pub name: &'static str,
    pub summary: &'static str,
    /// The type (or function) the composition root builds.
    pub adapter: &'static str,
    /// `(key, example value)` for `.env.example`.
    pub env: &'static [(&'static str, &'static str)],
    /// `[workspace.dependencies]` entries this driver adds, as `(key, spec)`.
    pub deps: &'static [(&'static str, &'static str)],
    /// The Terraform module's template directory, if the driver has one.
    pub terraform: Option<&'static str>,
}

const TOKIO: (&str, &str) = (
    "tokio",
    r#"{ version = "1", features = ["rt-multi-thread", "macros"] }"#,
);
const SERDE: (&str, &str) = ("serde", r#"{ version = "1", features = ["derive"] }"#);
const MAIL_FROM: (&str, &str) = ("MAIL_FROM", "no-reply@example.com");

/// Every built-in recipe, by name.
pub const CATALOG: &[Recipe] = &[
    Recipe {
        name: "auth",
        version: "0.1.0",
        summary: "sign-in and verification email",
        requires: &["mail"],
        drivers: &[
            Driver {
                name: "session",
                summary: "encrypted cookie sessions (actix-session)",
                adapter: "auth::session_middleware",
                env: &[("SESSION_SECRET", "")],
                deps: &[
                    (
                        "actix-session",
                        r#"{ version = "0.10", features = ["cookie-session"] }"#,
                    ),
                    ("actix-web", r#""4""#),
                ],
                terraform: None,
            },
            Driver {
                name: "jwt",
                summary: "HS256 bearer tokens (jsonwebtoken)",
                adapter: "auth::JwtAuth",
                env: &[("JWT_SECRET", ""), ("JWT_TTL_SECONDS", "3600")],
                deps: &[("jsonwebtoken", r#""9""#), SERDE],
                terraform: None,
            },
        ],
        deps: &[
            ("thiserror", r#""2""#),
            ("uuid", r#"{ version = "1", features = ["serde", "v4"] }"#),
            TOKIO,
        ],
    },
    Recipe {
        name: "mail",
        version: "0.1.0",
        summary: "outgoing email behind a Mailer port",
        requires: &[],
        drivers: &[
            Driver {
                name: "resend",
                summary: "the Resend HTTP API",
                adapter: "ResendMailer",
                env: &[("RESEND_API_KEY", ""), MAIL_FROM],
                deps: &[
                    (
                        "reqwest",
                        r#"{ version = "0.12", default-features = false, features = ["json", "rustls-tls"] }"#,
                    ),
                    SERDE,
                ],
                terraform: None,
            },
            Driver {
                name: "smtp",
                summary: "any SMTP relay (lettre)",
                adapter: "SmtpMailer",
                env: &[
                    ("SMTP_HOST", "localhost"),
                    ("SMTP_PORT", "587"),
                    ("SMTP_USERNAME", ""),
                    ("SMTP_PASSWORD", ""),
                    MAIL_FROM,
                ],
                deps: &[(
                    "lettre",
                    r#"{ version = "0.11", default-features = false, features = ["builder", "hostname", "pool", "smtp-transport", "tokio1", "tokio1-native-tls"] }"#,
                )],
                terraform: None,
            },
            Driver {
                name: "log",
                summary: "log messages instead of sending them (development)",
                adapter: "LogMailer",
                env: &[MAIL_FROM],
                deps: &[],
                terraform: None,
            },
        ],
        deps: &[
            ("async-trait", r#""0.1""#),
            ("thiserror", r#""2""#),
            ("tracing", r#""0.1""#),
            TOKIO,
        ],
    },
    Recipe {
        name: "storage",
        version: "0.1.0",
        summary: "object storage behind a Storage port",
        requires: &[],
        drivers: &[
            Driver {
                name: "s3",
                summary: "an S3 bucket (aws-sdk-s3)",
                adapter: "S3Storage",
                env: &[
                    ("S3_BUCKET", ""),
                    ("AWS_REGION", "us-east-1"),
                    ("AWS_ACCESS_KEY_ID", ""),
                    ("AWS_SECRET_ACCESS_KEY", ""),
                ],
                deps: &[
                    (
                        "aws-config",
                        r#"{ version = "1", features = ["behavior-version-latest"] }"#,
                    ),
                    ("aws-sdk-s3", r#""1""#),
                ],
                terraform: Some("storage-s3"),
            },
            Driver {
                name: "local",
                summary: "files in a local directory",
                adapter: "LocalStorage",
                env: &[("STORAGE_DIR", "./storage")],
                deps: &[],
                terraform: None,
            },
        ],
        deps: &[("async-trait", r#""0.1""#), ("thiserror", r#""2""#), TOKIO],
    },
];

/// The Terraform files every module template directory has.
const TERRAFORM_FILES: [&str; 3] = ["main.tf", "variables.tf", "outputs.tf"];

/// The built-in recipe called `name`.
///
/// # Errors
///
/// [`GenError::Invalid`] naming the available recipes.
pub fn find(name: &str) -> Result<&'static Recipe, GenError> {
    CATALOG
        .iter()
        .find(|recipe| recipe.name == name)
        .ok_or_else(|| {
            GenError::Invalid(format!(
                "no recipe `{name}`; available: {}",
                CATALOG
                    .iter()
                    .map(|recipe| recipe.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })
}

impl Recipe {
    /// The driver called `name`, or the default one.
    ///
    /// # Errors
    ///
    /// [`GenError::Invalid`] naming the recipe's drivers.
    pub fn driver(&'static self, name: Option<&str>) -> Result<&'static Driver, GenError> {
        let Some(name) = name else {
            return self.drivers.first().ok_or_else(|| {
                GenError::Invalid(format!("recipe `{}` has no drivers", self.name))
            });
        };
        self.drivers
            .iter()
            .find(|driver| driver.name == name)
            .ok_or_else(|| {
                GenError::Invalid(format!(
                    "recipe `{}` has no driver `{name}`; choose one of: {}",
                    self.name,
                    self.drivers
                        .iter()
                        .map(|driver| driver.name)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })
    }

    fn crate_dir(&self) -> String {
        format!("crates/{}", self.name)
    }

    fn terraform_dir(&self) -> String {
        format!("{TERRAFORM_DIR}/{}", self.name)
    }
}

/// One recipe `add` installs.
struct Install {
    recipe: &'static Recipe,
    driver: &'static Driver,
    terraform: bool,
}

/// Checks `cargo rok new --recipe/--driver` choices before anything is
/// written.
///
/// # Errors
///
/// [`GenError::Invalid`] for an unknown recipe or driver.
pub fn validate(recipes: &[String], drivers: &BTreeMap<String, String>) -> Result<(), GenError> {
    for name in recipes {
        find(name)?;
    }
    for (name, driver) in drivers {
        find(name)?.driver(Some(driver))?;
    }
    Ok(())
}

/// `cargo rok add`: installs `args.recipe`, after whatever it requires that
/// isn't installed yet, as one transaction.
///
/// # Errors
///
/// [`GenError::Invalid`] for an unknown recipe or driver, or a driver
/// without a Terraform module when one was asked for;
/// [`GenError::Conflict`] when it's already installed or its crate
/// directory is taken.
pub fn add(project: &Project, args: &AddArgs) -> Result<Generated, GenError> {
    let target = find(&args.recipe)?;
    if let Some(installed) = project.config.recipes.get(target.name) {
        return Err(GenError::Conflict(format!(
            "recipe `{}` is already installed (driver `{}`); `cargo rok remove {}` first to \
             switch drivers",
            target.name, installed.driver, target.name
        )));
    }
    for name in args.drivers.keys() {
        find(name)?;
    }

    let mut order = Vec::new();
    requirements_first(target, &project.config.recipes, &mut order);
    let installs = order
        .into_iter()
        .map(|recipe| {
            let is_target = recipe.name == target.name;
            let choice = if is_target {
                args.driver.as_deref()
            } else {
                None
            }
            .or_else(|| args.drivers.get(recipe.name).map(String::as_str));
            let driver = recipe.driver(choice)?;
            let terraform = is_target && args.terraform;
            if terraform && driver.terraform.is_none() {
                return Err(GenError::Invalid(format!(
                    "the `{}` driver of `{}` has no Terraform module",
                    driver.name, recipe.name
                )));
            }
            Ok(Install {
                recipe,
                driver,
                terraform,
            })
        })
        .collect::<Result<Vec<_>, GenError>>()?;

    let mut tx = Transaction::new(&project.root);
    let root_manifest = tx.read("Cargo.toml")?;
    if !root_manifest.contains("\"crates/*\"") {
        return Err(GenError::Invalid(
            "workspace.members must include \"crates/*\" so the recipe's crate is a member"
                .to_string(),
        ));
    }
    let renderer = templates::renderer()?;
    for install in &installs {
        install_into(&mut tx, &renderer, install)?;
    }

    let mut generated = finish_labeled(
        "add",
        Some(format!("recipe {}", target.name)),
        tx,
        Apply {
            dry_run: args.dry_run,
            verify: args.verify,
        },
    )?;
    let installed: Vec<serde_json::Value> = installs
        .iter()
        .map(|install| {
            json!({
                "recipe": install.recipe.name,
                "version": install.recipe.version,
                "driver": install.driver.name,
                "terraform": install.terraform,
            })
        })
        .collect();
    if let Some(data) = generated.outcome.data.as_object_mut() {
        data.insert("installed".to_string(), json!(installed));
    }
    let mut summary = String::new();
    for install in &installs {
        let _ = write!(
            summary,
            "recipe {} {} (driver {})",
            install.recipe.name, install.recipe.version, install.driver.name
        );
        if install.recipe.name != target.name {
            let _ = write!(summary, ", required by {}", target.name);
        }
        summary.push('\n');
    }
    if !args.dry_run {
        for install in &installs {
            let _ = writeln!(
                summary,
                "next: fill in its keys in {ENV_EXAMPLE} (copy them to .env) and build \
                 `{}` in the composition root",
                install.driver.adapter
            );
        }
    }
    generated.human.insert_str(0, &summary);
    Ok(generated)
}

/// `recipe`'s missing requirements (depth first), then `recipe`.
fn requirements_first(
    recipe: &'static Recipe,
    installed: &BTreeMap<String, InstalledRecipe>,
    order: &mut Vec<&'static Recipe>,
) {
    if order.iter().any(|planned| planned.name == recipe.name) {
        return;
    }
    for name in recipe.requires {
        if installed.contains_key(*name) {
            continue;
        }
        if let Ok(required) = find(name) {
            requirements_first(required, installed, order);
        }
    }
    order.push(recipe);
}

fn context(recipe: &Recipe, driver: &Driver) -> serde_json::Value {
    json!({
        "recipe": recipe.name,
        "version": recipe.version,
        "driver": driver.name,
        "adapter": driver.adapter,
    })
}

/// Every file `install` creates, as `(path, content)`.
fn render_files(
    renderer: &Renderer,
    recipe: &Recipe,
    driver: &Driver,
    terraform: bool,
) -> Result<Vec<(String, String)>, GenError> {
    let context = context(recipe, driver);
    let dir = recipe.crate_dir();
    let mut files = vec![
        (
            format!("{dir}/Cargo.toml"),
            renderer.render(&format!("recipes/{}/Cargo.toml", recipe.name), &context)?,
        ),
        (
            format!("{dir}/src/lib.rs"),
            renderer.render_rust(
                &format!("recipes/{}/lib.rs", recipe.name),
                &context,
                SyntaxKind::File,
            )?,
        ),
    ];
    if let (true, Some(module)) = (terraform, driver.terraform) {
        for file in TERRAFORM_FILES {
            files.push((
                format!("{}/{file}", recipe.terraform_dir()),
                renderer.render(&format!("recipes/terraform/{module}/{file}"), &context)?,
            ));
        }
    }
    Ok(files)
}

fn install_into(
    tx: &mut Transaction,
    renderer: &Renderer,
    install: &Install,
) -> Result<(), GenError> {
    let Install {
        recipe,
        driver,
        terraform,
    } = install;
    let dir = recipe.crate_dir();
    if tx.root().join(&dir).exists() || tx.exists(&format!("{dir}/Cargo.toml")) {
        return Err(GenError::Conflict(format!(
            "`{dir}` already exists, so the `{}` recipe can't be added",
            recipe.name
        )));
    }
    for (path, content) in render_files(renderer, recipe, driver, *terraform)? {
        tx.create(&path, content)?;
    }

    let mut root = tx.read("Cargo.toml")?;
    let internal = manifest::internal_crates(&root);
    root = manifest::add_dependency(
        &root,
        "workspace.dependencies",
        recipe.name,
        &format!("{} = {{ path = \"{dir}\" }}", recipe.name),
        &internal,
    );
    for (key, spec) in recipe.deps.iter().chain(driver.deps) {
        root = manifest::append_dependency(
            &root,
            "workspace.dependencies",
            key,
            &format!("{key} = {spec}"),
        );
    }
    tx.update("Cargo.toml", root)?;

    let env = tx.peek(ENV_EXAMPLE).unwrap_or_default();
    let updated = add_env_keys(&env, recipe, driver);
    if tx.exists(ENV_EXAMPLE) {
        tx.update(ENV_EXAMPLE, updated)?;
    } else {
        tx.create(ENV_EXAMPLE, updated)?;
    }

    let config = tx.read(CONFIG_FILE)?;
    tx.update(
        CONFIG_FILE,
        add_config_section(&config, recipe, driver, *terraform),
    )?;
    Ok(())
}

/// `cargo rok remove`: uninstalls a recipe nothing depends on.
///
/// # Errors
///
/// [`GenError::Invalid`] when it isn't installed; [`GenError::Conflict`]
/// when another recipe or crate depends on it, or (without `force`) when
/// its files were changed since it was added.
pub fn remove(project: &Project, args: &RemoveArgs, force: bool) -> Result<Generated, GenError> {
    let installed = &project.config.recipes;
    let Some(entry) = installed.get(&args.recipe) else {
        let names: Vec<&str> = installed.keys().map(String::as_str).collect();
        return Err(GenError::Invalid(format!(
            "recipe `{}` isn't installed; installed: {}",
            args.recipe,
            if names.is_empty() {
                "none".to_string()
            } else {
                names.join(", ")
            }
        )));
    };
    let recipe = find(&args.recipe)?;
    let driver = recipe.driver(Some(&entry.driver))?;

    refuse_dependents(project, recipe)?;

    let mut tx = Transaction::new(&project.root);
    let mut owned = files_under(&project.root, &recipe.crate_dir());
    if entry.terraform {
        owned.extend(files_under(&project.root, &recipe.terraform_dir()));
    }
    if !force {
        refuse_modified(&project.root, recipe, driver, entry, &owned)?;
    }
    for file in &owned {
        tx.delete(file)?;
    }

    let root = tx.read("Cargo.toml")?;
    tx.update(
        "Cargo.toml",
        manifest::remove_dependency(&root, "workspace.dependencies", recipe.name),
    )?;

    if let Some(env) = tx.peek(ENV_EXAMPLE) {
        let kept: BTreeSet<&str> = installed
            .iter()
            .filter(|(name, _)| *name != recipe.name)
            .filter_map(|(name, other)| find(name).ok()?.driver(Some(&other.driver)).ok())
            .flat_map(|other| other.env.iter().map(|(key, _)| *key))
            .collect();
        let updated = remove_env_keys(&env, recipe, driver, &kept);
        if updated.trim().is_empty() {
            tx.delete(ENV_EXAMPLE)?;
        } else {
            tx.update(ENV_EXAMPLE, updated)?;
        }
    }

    let config = tx.read(CONFIG_FILE)?;
    tx.update(CONFIG_FILE, remove_config_section(&config, recipe.name))?;

    let mut generated = finish_labeled(
        "remove",
        Some(format!("recipe {}", recipe.name)),
        tx,
        Apply {
            dry_run: args.dry_run,
            verify: args.verify,
        },
    )?;
    generated.human.insert_str(
        0,
        &format!(
            "recipe {} {} (driver {})\n",
            recipe.name, entry.version, entry.driver
        ),
    );
    Ok(generated)
}

/// Refuses while an installed recipe requires `recipe`, or a workspace
/// crate other than its own depends on its crate.
fn refuse_dependents(project: &Project, recipe: &Recipe) -> Result<(), GenError> {
    let recipes: Vec<&str> = project
        .config
        .recipes
        .keys()
        .filter_map(|name| find(name).ok())
        .filter(|other| other.requires.contains(&recipe.name))
        .map(|other| other.name)
        .collect();
    let recipe_crates: BTreeSet<String> = recipes
        .iter()
        .map(|name| format!("crates/{name}/Cargo.toml"))
        .chain(std::iter::once(format!(
            "{}/Cargo.toml",
            recipe.crate_dir()
        )))
        .collect();
    let crates: Vec<String> = member_manifests(&project.root)
        .into_iter()
        .filter(|(rel, _)| !recipe_crates.contains(rel))
        .filter(|(_, text)| depends_on(text, recipe.name))
        .map(|(rel, _)| rel)
        .collect();
    if recipes.is_empty() && crates.is_empty() {
        return Ok(());
    }
    let mut reasons = Vec::new();
    if !recipes.is_empty() {
        reasons.push(format!(
            "required by recipe{} {}",
            if recipes.len() == 1 { "" } else { "s" },
            recipes.join(", ")
        ));
    }
    if !crates.is_empty() {
        reasons.push(format!("depended on by {}", crates.join(", ")));
    }
    Err(GenError::Conflict(format!(
        "can't remove recipe `{}`: it's {}; remove those first",
        recipe.name,
        reasons.join(", and ")
    )))
}

/// Refuses when the recipe's files differ (beyond formatting) from what its
/// installed driver renders, or files were added among them.
fn refuse_modified(
    root: &Path,
    recipe: &Recipe,
    driver: &Driver,
    entry: &InstalledRecipe,
    owned: &[String],
) -> Result<(), GenError> {
    let renderer = templates::renderer()?;
    // As `add` wrote them: Rust goes through rustfmt, which may reorder
    // imports (and so change the token sequence).
    let expected: BTreeMap<String, String> =
        render_files(&renderer, recipe, driver, entry.terraform)?
            .into_iter()
            .map(|(rel, text)| {
                let formatted = if is_rust(&rel) {
                    rok_edit::format_rust(root, &text).unwrap_or(text)
                } else {
                    text
                };
                (rel, formatted)
            })
            .collect();
    let modified: Vec<&String> = owned
        .iter()
        .filter(|rel| {
            let actual = std::fs::read_to_string(root.join(rel.as_str())).ok();
            match (expected.get(rel.as_str()), actual) {
                (Some(expected), Some(actual)) => !same_content(rel, expected, &actual),
                _ => true,
            }
        })
        .collect();
    if modified.is_empty() {
        return Ok(());
    }
    let mut message = format!(
        "these files of recipe `{}` were changed or added since it was installed",
        recipe.name
    );
    if entry.version != recipe.version {
        let _ = write!(
            message,
            " (it was installed at version {}, this is {})",
            entry.version, recipe.version
        );
    }
    let _ = write!(
        message,
        ": {}; pass --force to remove them anyway",
        modified
            .iter()
            .map(|rel| rel.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    Err(GenError::Conflict(message))
}

/// Equal up to formatting: Rust by tokens, anything else by
/// whitespace-separated words.
fn same_content(rel: &str, expected: &str, actual: &str) -> bool {
    if is_rust(rel) {
        let tokens = |text: &str| proc_macro2::TokenStream::from_str(text).map(|t| t.to_string());
        matches!((tokens(expected), tokens(actual)), (Ok(a), Ok(b)) if a == b)
    } else {
        expected.split_whitespace().eq(actual.split_whitespace())
    }
}

fn is_rust(rel: &str) -> bool {
    Path::new(rel)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
}

/// Every file under `rel_dir` (skipping `target/`), relative to `root`,
/// sorted.
fn files_under(root: &Path, rel_dir: &str) -> Vec<String> {
    let mut files: Vec<String> = walkdir::WalkDir::new(root.join(rel_dir))
        .into_iter()
        .filter_entry(|entry| entry.file_name() != "target")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            entry
                .path()
                .strip_prefix(root)
                .ok()
                .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        })
        .collect();
    files.sort();
    files
}

/// `(manifest path, text)` for the root manifest and every workspace member.
fn member_manifests(root: &Path) -> Vec<(String, String)> {
    let read = |rel: &str| std::fs::read_to_string(root.join(rel)).ok();
    let Some(root_text) = read("Cargo.toml") else {
        return Vec::new();
    };
    let mut rels = BTreeSet::new();
    let table: Option<toml::Table> = toml::from_str(&root_text).ok();
    let patterns: Vec<String> = table
        .as_ref()
        .and_then(|table| table.get("workspace"))
        .and_then(|workspace| workspace.get("members"))
        .and_then(toml::Value::as_array)
        .map(|members| {
            members
                .iter()
                .filter_map(toml::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    for pattern in patterns {
        let pattern = pattern.trim_end_matches('/');
        if let Some(parent) = pattern.strip_suffix("/*") {
            for entry in std::fs::read_dir(root.join(parent))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
            {
                if let Some(name) = entry.file_name().to_str() {
                    rels.insert(format!("{parent}/{name}/Cargo.toml"));
                }
            }
        } else {
            rels.insert(format!("{pattern}/Cargo.toml"));
        }
    }
    std::iter::once(("Cargo.toml".to_string(), root_text))
        .chain(
            rels.into_iter()
                .filter_map(|rel| read(&rel).map(|text| (rel, text))),
        )
        .collect()
}

/// Whether a manifest declares a normal, dev, build or target-specific
/// dependency on the crate `name` (by key or by `package = "..."`).
fn depends_on(manifest_text: &str, name: &str) -> bool {
    let Ok(table) = toml::from_str::<toml::Table>(manifest_text) else {
        return false;
    };
    let in_tables = |tables: &toml::Table| {
        ["dependencies", "dev-dependencies", "build-dependencies"]
            .iter()
            .filter_map(|section| tables.get(*section).and_then(toml::Value::as_table))
            .any(|deps| {
                deps.iter().any(|(key, spec)| {
                    key == name
                        || spec
                            .get("package")
                            .and_then(toml::Value::as_str)
                            .is_some_and(|package| package == name)
                })
            })
    };
    in_tables(&table)
        || table
            .get("target")
            .and_then(toml::Value::as_table)
            .is_some_and(|targets| {
                targets
                    .values()
                    .filter_map(toml::Value::as_table)
                    .any(in_tables)
            })
}

/// The key an env file line assigns, if any.
fn env_key(line: &str) -> Option<&str> {
    let line = line.trim();
    let line = line.strip_prefix("export ").unwrap_or(line);
    if line.starts_with('#') {
        return None;
    }
    line.split_once('=').map(|(key, _)| key.trim())
}

fn env_header(recipe: &Recipe, driver: &Driver) -> String {
    format!("# {} recipe ({})", recipe.name, driver.name)
}

/// `text` with a block of the driver's env keys appended, skipping keys
/// already there.
fn add_env_keys(text: &str, recipe: &Recipe, driver: &Driver) -> String {
    let present: BTreeSet<&str> = text.lines().filter_map(env_key).collect();
    let missing: Vec<&(&str, &str)> = driver
        .env
        .iter()
        .filter(|(key, _)| !present.contains(key))
        .collect();
    if missing.is_empty() {
        return text.to_string();
    }
    let mut output = text.trim_end().to_string();
    if !output.is_empty() {
        output.push_str("\n\n");
    }
    output.push_str(&env_header(recipe, driver));
    output.push('\n');
    for (key, value) in missing {
        let _ = writeln!(output, "{key}={value}");
    }
    output
}

/// `text` without the driver's header and the env keys no other installed
/// recipe (`kept`) uses.
fn remove_env_keys(text: &str, recipe: &Recipe, driver: &Driver, kept: &BTreeSet<&str>) -> String {
    let header = env_header(recipe, driver);
    let removed: BTreeSet<&str> = driver
        .env
        .iter()
        .map(|(key, _)| *key)
        .filter(|key| !kept.contains(key))
        .collect();
    let mut output = String::new();
    let mut blank = true;
    for line in text.lines() {
        if line.trim() == header || env_key(line).is_some_and(|key| removed.contains(key)) {
            continue;
        }
        let is_blank = line.trim().is_empty();
        if is_blank && blank {
            continue;
        }
        blank = is_blank;
        output.push_str(line);
        output.push('\n');
    }
    let trimmed = output.trim_end();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    }
}

fn add_config_section(text: &str, recipe: &Recipe, driver: &Driver, terraform: bool) -> String {
    let mut output = text.trim_end().to_string();
    let _ = write!(
        output,
        "\n\n[recipes.{}]\nversion = \"{}\"\ndriver = \"{}\"\n",
        recipe.name, recipe.version, driver.name
    );
    if terraform {
        output.push_str("terraform = true\n");
    }
    output
}

/// `text` without the `[recipes.<name>]` table.
fn remove_config_section(text: &str, name: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let header = format!("[recipes.{name}]");
    let Some(start) = lines.iter().position(|line| line.trim() == header) else {
        return text.to_string();
    };
    let end = lines[start + 1..]
        .iter()
        .position(|line| line.trim_start().starts_with('['))
        .map_or(lines.len(), |offset| start + 1 + offset);
    let before = lines[..start].join("\n");
    let before = before.trim_end();
    let after = lines[end..].join("\n");
    match (before.is_empty(), after.is_empty()) {
        (_, true) => format!("{before}\n"),
        (true, false) => format!("{after}\n"),
        (false, false) => format!("{before}\n\n{after}\n"),
    }
}

/// `cargo rok list`: every built-in recipe, and which are installed.
#[must_use]
pub fn list(project: &Project) -> Generated {
    let installed = &project.config.recipes;
    let mut rows = Vec::new();
    let mut entries = Vec::new();
    for recipe in CATALOG {
        let current = installed.get(recipe.name);
        let drivers: Vec<String> = recipe
            .drivers
            .iter()
            .enumerate()
            .map(|(index, driver)| {
                let mut label = driver.name.to_string();
                if index == 0 {
                    label.push('*');
                }
                if driver.terraform.is_some() {
                    label.push_str(" (terraform)");
                }
                label
            })
            .collect();
        let update = current.is_some_and(|entry| entry.version != recipe.version);
        rows.push([
            recipe.name.to_string(),
            recipe.version.to_string(),
            current.map_or_else(
                || "-".to_string(),
                |entry| {
                    let mut text = format!("{} {}", entry.version, entry.driver);
                    if update {
                        text.push_str(" (update available)");
                    }
                    text
                },
            ),
            drivers.join(", "),
            recipe.requires.join(", "),
        ]);
        entries.push(json!({
            "name": recipe.name,
            "version": recipe.version,
            "summary": recipe.summary,
            "requires": recipe.requires,
            "drivers": recipe.drivers.iter().enumerate().map(|(index, driver)| json!({
                "name": driver.name,
                "summary": driver.summary,
                "default": index == 0,
                "terraform": driver.terraform.is_some(),
                "env": driver.env.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "installed": current,
            "update_available": update,
        }));
    }
    let unknown: Vec<&String> = installed
        .keys()
        .filter(|name| find(name).is_err())
        .collect();

    let mut human = table(
        &["recipe", "version", "installed", "drivers", "requires"],
        &rows,
    );
    human.push_str("* default driver\n");
    for name in &unknown {
        let _ = writeln!(
            human,
            "warning: installed recipe `{name}` isn't one this rok-gen knows"
        );
    }
    let mut outcome = Outcome::new("list", true);
    outcome.data = json!({ "recipes": entries, "unknown_installed": unknown });
    Generated { outcome, human }
}

fn table<const N: usize>(headers: &[&str; N], rows: &[[String; N]]) -> String {
    let mut widths = headers.map(str::len);
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.len());
        }
    }
    let mut text = String::new();
    let mut line = |cells: Vec<&str>| {
        let mut out = String::new();
        for (cell, width) in cells.iter().zip(widths) {
            let _ = write!(out, "{cell:width$}  ");
        }
        text.push_str(out.trim_end());
        text.push('\n');
    };
    line(headers.to_vec());
    for row in rows {
        line(row.iter().map(String::as_str).collect());
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail() -> (&'static Recipe, &'static Driver) {
        let recipe = find("mail").unwrap();
        (recipe, recipe.driver(Some("resend")).unwrap())
    }

    #[test]
    fn every_driver_of_every_recipe_renders() {
        let renderer = templates::renderer().unwrap();
        for recipe in CATALOG {
            for requirement in recipe.requires {
                find(requirement).unwrap();
            }
            for driver in recipe.drivers {
                if let Err(err) =
                    render_files(&renderer, recipe, driver, driver.terraform.is_some())
                {
                    panic!("{} / {}: {err}", recipe.name, driver.name);
                }
            }
        }
    }

    #[test]
    fn env_keys_are_appended_once_and_removed_unless_shared() {
        let (recipe, driver) = mail();
        let text = add_env_keys("DATABASE_URL=postgres://\n", recipe, driver);
        assert_eq!(
            text,
            "DATABASE_URL=postgres://\n\n# mail recipe (resend)\nRESEND_API_KEY=\nMAIL_FROM=no-reply@example.com\n"
        );
        assert_eq!(add_env_keys(&text, recipe, driver), text);

        assert_eq!(
            remove_env_keys(&text, recipe, driver, &BTreeSet::new()),
            "DATABASE_URL=postgres://\n"
        );
        assert_eq!(
            remove_env_keys(&text, recipe, driver, &BTreeSet::from(["MAIL_FROM"])),
            "DATABASE_URL=postgres://\n\nMAIL_FROM=no-reply@example.com\n"
        );
    }

    #[test]
    fn config_sections_round_trip() {
        let (recipe, driver) = mail();
        let base = "[rok]\ncli = \"^0.1\"\n\n[architecture]\nallow = []\n";
        let added = add_config_section(base, recipe, driver, false);
        assert!(added.ends_with("\n\n[recipes.mail]\nversion = \"0.1.0\"\ndriver = \"resend\"\n"));
        assert_eq!(remove_config_section(&added, "mail"), base);

        let middle = "[rok]\n\n[recipes.mail]\ndriver = \"x\"\n\n[recipes.auth]\ndriver = \"y\"\n";
        assert_eq!(
            remove_config_section(middle, "mail"),
            "[rok]\n\n[recipes.auth]\ndriver = \"y\"\n"
        );
    }

    #[test]
    fn dependencies_are_found_in_every_section_and_by_package() {
        assert!(depends_on(
            "[dependencies]\nmail.workspace = true\n",
            "mail"
        ));
        assert!(depends_on(
            "[dev-dependencies]\nmail = { workspace = true, features = [\"fake\"] }\n",
            "mail"
        ));
        assert!(depends_on(
            "[dependencies]\nmailer = { package = \"mail\", path = \"../mail\" }\n",
            "mail"
        ));
        assert!(depends_on(
            "[target.'cfg(unix)'.dependencies]\nmail.workspace = true\n",
            "mail"
        ));
        assert!(!depends_on(
            "[dependencies]\nmailx.workspace = true\n",
            "mail"
        ));
    }

    #[test]
    fn formatting_differences_are_not_modifications() {
        assert!(same_content(
            "a.rs",
            "fn f() { 1 }",
            "fn f()\n{\n    1\n}\n"
        ));
        assert!(!same_content("a.rs", "fn f() { 1 }", "fn f() { 2 }"));
        assert!(same_content("a.toml", "a = 1\nb = 2", "a = 1\n\nb = 2\n"));
    }
}
