//! All-or-nothing file changes.
//!
//! A [`Transaction`] collects new and edited files in memory, formats every
//! Rust file with rustfmt (never skipped), and then either renders a diff
//! (`--dry-run`) or writes everything, rolling back every write if any
//! write or the optional `cargo check` fails.
//!
//! Text is handled `\n`-normalized; each existing file's CRLF/LF style is
//! detected on read and restored on write.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Attempts per file write, for files briefly locked by editors,
/// rust-analyzer or antivirus (mostly on Windows).
const WRITE_ATTEMPTS: u32 = 4;

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("could not read {rel}: {source}")]
    Read {
        rel: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{rel} already exists")]
    Exists { rel: String },
    #[error("{rel} doesn't exist")]
    Missing { rel: String },
    #[error("rustfmt rejected {rel} (a generator bug; nothing was written):\n{message}")]
    Format { rel: String, message: String },
    #[error("could not write {rel}: {source}; every change was rolled back")]
    Write {
        rel: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`cargo check` failed after writing; every change was rolled back:\n{output}")]
    Verify { output: String },
}

#[derive(Clone, Debug)]
struct Change {
    /// The file as it was on disk (raw, original line endings).
    before: Option<String>,
    crlf: bool,
    /// The new content, `\n`-normalized.
    after: String,
    /// The file is removed on commit.
    deleted: bool,
}

/// One file's pending change, `\n`-normalized (for the journal).
#[derive(Clone, Debug)]
pub struct ChangeView {
    pub rel: String,
    pub before: Option<String>,
    /// `None` when the file is being deleted.
    pub after: Option<String>,
}

/// Pending changes to files under one root.
#[derive(Debug)]
pub struct Transaction {
    root: PathBuf,
    changes: BTreeMap<String, Change>,
    warnings: Vec<String>,
}

/// What a successful commit wrote, so it can be undone.
#[derive(Debug, Default)]
pub struct Committed {
    written: Vec<(PathBuf, Option<String>)>,
    created_dirs: Vec<PathBuf>,
}

impl Transaction {
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            changes: BTreeMap::new(),
            warnings: Vec::new(),
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether `rel` exists, counting files created earlier in this
    /// transaction.
    #[must_use]
    pub fn exists(&self, rel: &str) -> bool {
        self.changes.contains_key(rel) || self.root.join(rel).is_file()
    }

    /// The current (`\n`-normalized) text of `rel`: pending content if this
    /// transaction already changed it, otherwise what's on disk.
    ///
    /// # Errors
    ///
    /// [`EditError::Missing`] or [`EditError::Read`].
    pub fn read(&mut self, rel: &str) -> Result<String, EditError> {
        if let Some(change) = self.changes.get(rel) {
            return Ok(change.after.clone());
        }
        let path = self.root.join(rel);
        if !path.is_file() {
            return Err(EditError::Missing {
                rel: rel.to_string(),
            });
        }
        let raw = std::fs::read_to_string(&path).map_err(|source| EditError::Read {
            rel: rel.to_string(),
            source,
        })?;
        let crlf = raw.contains("\r\n");
        let normalized = raw.replace("\r\n", "\n");
        self.changes.insert(
            rel.to_string(),
            Change {
                before: Some(raw),
                crlf,
                after: normalized.clone(),
                deleted: false,
            },
        );
        Ok(normalized)
    }

    /// Adds a new file.
    ///
    /// # Errors
    ///
    /// [`EditError::Exists`] if it's on disk or already added.
    pub fn create(&mut self, rel: &str, content: impl Into<String>) -> Result<(), EditError> {
        if self.exists(rel) {
            return Err(EditError::Exists {
                rel: rel.to_string(),
            });
        }
        self.changes.insert(
            rel.to_string(),
            Change {
                before: None,
                crlf: false,
                after: content.into(),
                deleted: false,
            },
        );
        Ok(())
    }

    /// Replaces the content of an existing file.
    ///
    /// # Errors
    ///
    /// [`EditError::Missing`] or [`EditError::Read`].
    pub fn update(&mut self, rel: &str, content: impl Into<String>) -> Result<(), EditError> {
        self.read(rel)?;
        if let Some(change) = self.changes.get_mut(rel) {
            change.after = content.into();
        }
        Ok(())
    }

    /// Formats every changed `.rs` file with rustfmt, using the project's
    /// `rustfmt.toml` and edition. Warns about edited files that weren't
    /// rustfmt-clean before, since their diff includes formatting-only
    /// changes.
    ///
    /// # Errors
    ///
    /// [`EditError::Format`] when the new content doesn't format (i.e.
    /// isn't valid Rust).
    pub fn format(&mut self) -> Result<(), EditError> {
        let formatter = Rustfmt::for_project(&self.root);
        let mut warnings = Vec::new();
        for (rel, change) in self.changes.iter_mut().filter(|(rel, change)| {
            !change.deleted
                && Path::new(rel.as_str())
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
        }) {
            if let Some(before) = &change.before {
                let normalized = before.replace("\r\n", "\n");
                if formatter.format(&normalized).ok().as_deref() != Some(normalized.as_str()) {
                    warnings.push(format!(
                        "{rel} wasn't rustfmt-clean before this change; its diff includes \
                         formatting-only changes"
                    ));
                }
            }
            change.after =
                formatter
                    .format(&change.after)
                    .map_err(|message| EditError::Format {
                        rel: rel.clone(),
                        message,
                    })?;
        }
        self.warnings.extend(warnings);
        Ok(())
    }

    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Removes an existing file.
    ///
    /// # Errors
    ///
    /// [`EditError::Missing`] or [`EditError::Read`].
    pub fn delete(&mut self, rel: &str) -> Result<(), EditError> {
        self.read(rel)?;
        if let Some(change) = self.changes.get_mut(rel) {
            change.deleted = true;
        }
        Ok(())
    }

    /// New files, relative to the root.
    #[must_use]
    pub fn created(&self) -> Vec<String> {
        self.changes
            .iter()
            .filter(|(_, change)| change.before.is_none() && !change.deleted)
            .map(|(rel, _)| rel.clone())
            .collect()
    }

    /// Existing files whose content actually changes.
    #[must_use]
    pub fn changed(&self) -> Vec<String> {
        self.changes
            .iter()
            .filter(|(_, change)| {
                !change.deleted
                    && change
                        .before
                        .as_ref()
                        .is_some_and(|before| before.replace("\r\n", "\n") != change.after)
            })
            .map(|(rel, _)| rel.clone())
            .collect()
    }

    /// Existing files being removed.
    #[must_use]
    pub fn deleted(&self) -> Vec<String> {
        self.changes
            .iter()
            .filter(|(_, change)| change.deleted && change.before.is_some())
            .map(|(rel, _)| rel.clone())
            .collect()
    }

    /// Every pending change, `\n`-normalized.
    #[must_use]
    pub fn changes(&self) -> Vec<ChangeView> {
        self.changes
            .iter()
            .map(|(rel, change)| ChangeView {
                rel: rel.clone(),
                before: change
                    .before
                    .as_deref()
                    .map(|text| text.replace("\r\n", "\n")),
                after: (!change.deleted).then(|| change.after.clone()),
            })
            .collect()
    }

    /// A unified diff of every change.
    #[must_use]
    pub fn diff(&self) -> String {
        let mut output = String::new();
        for (rel, change) in &self.changes {
            let before = change
                .before
                .as_deref()
                .map(|text| text.replace("\r\n", "\n"))
                .unwrap_or_default();
            let after = if change.deleted {
                ""
            } else {
                change.after.as_str()
            };
            if before == after && !change.deleted {
                continue;
            }
            let old = if change.before.is_some() {
                format!("a/{rel}")
            } else {
                "/dev/null".to_string()
            };
            let new = if change.deleted {
                "/dev/null".to_string()
            } else {
                format!("b/{rel}")
            };
            let diff = similar::TextDiff::from_lines(&before, after);
            output.push_str(
                &diff
                    .unified_diff()
                    .context_radius(3)
                    .header(&old, &new)
                    .to_string(),
            );
        }
        output
    }

    /// Writes every change: each file goes to a temp file next to it, then
    /// is renamed into place. Any failure rolls back what was written.
    ///
    /// # Errors
    ///
    /// [`EditError::Write`], after rolling back.
    pub fn commit(&self) -> Result<Committed, EditError> {
        let mut committed = Committed::default();
        for (rel, change) in &self.changes {
            let path = self.root.join(rel);
            if change.deleted {
                if let Err(source) = std::fs::remove_file(&path) {
                    rollback(&committed);
                    return Err(EditError::Write {
                        rel: rel.clone(),
                        source,
                    });
                }
                committed.written.push((path, change.before.clone()));
                continue;
            }
            let unchanged = change
                .before
                .as_ref()
                .is_some_and(|before| before.replace("\r\n", "\n") == change.after);
            if unchanged {
                continue;
            }
            let content = if change.crlf {
                change.after.replace('\n', "\r\n")
            } else {
                change.after.clone()
            };
            if let Err(source) = write_file(&path, &content, &mut committed.created_dirs) {
                rollback(&committed);
                return Err(EditError::Write {
                    rel: rel.clone(),
                    source,
                });
            }
            committed.written.push((path, change.before.clone()));
        }
        Ok(committed)
    }

    /// [`Transaction::commit`], then `cargo check --workspace --all-targets`;
    /// rolls everything back if the check fails.
    ///
    /// # Errors
    ///
    /// [`EditError::Write`] or [`EditError::Verify`].
    pub fn commit_verified(&self) -> Result<Committed, EditError> {
        let committed = self.commit()?;
        let output = Command::new("cargo")
            .args(["check", "--workspace", "--all-targets", "--quiet"])
            .current_dir(&self.root)
            .output();
        match output {
            Ok(output) if output.status.success() => Ok(committed),
            Ok(output) => {
                rollback(&committed);
                let mut text = String::from_utf8_lossy(&output.stderr).into_owned();
                text.push_str(&String::from_utf8_lossy(&output.stdout));
                Err(EditError::Verify {
                    output: tail(&text, 80),
                })
            }
            Err(err) => {
                rollback(&committed);
                Err(EditError::Verify {
                    output: format!("could not run cargo: {err}"),
                })
            }
        }
    }
}

/// Restores every written file (or removes it if it was new), then removes
/// directories the commit created, if they're empty again.
pub fn rollback(committed: &Committed) {
    for (path, before) in committed.written.iter().rev() {
        // Best effort: a failed restore can't be recovered from here either.
        let _ = before.as_ref().map_or_else(
            || std::fs::remove_file(path),
            |content| std::fs::write(path, content),
        );
    }
    for dir in committed.created_dirs.iter().rev() {
        let _ = std::fs::remove_dir(dir);
    }
}

fn write_file(path: &Path, content: &str, created_dirs: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        // Collected before `create_dir_all`, which makes them exist.
        #[allow(clippy::needless_collect)]
        let missing: Vec<PathBuf> = parent
            .ancestors()
            .take_while(|dir| !dir.exists())
            .map(Path::to_path_buf)
            .collect();
        std::fs::create_dir_all(parent)?;
        created_dirs.extend(missing.into_iter().rev());
    }
    let mut temp = path.as_os_str().to_owned();
    temp.push(".rok-tmp");
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, content)?;
    let mut attempt = 0;
    loop {
        match std::fs::rename(&temp, path) {
            Ok(()) => return Ok(()),
            Err(_) if attempt + 1 < WRITE_ATTEMPTS => {
                std::thread::sleep(Duration::from_millis(50 * 4_u64.pow(attempt)));
                attempt += 1;
            }
            Err(err) => {
                let _ = std::fs::remove_file(&temp);
                return Err(err);
            }
        }
    }
}

/// rustfmt in stdin mode with the project's config and edition.
struct Rustfmt {
    config_dir: Option<PathBuf>,
    edition: String,
}

impl Rustfmt {
    fn for_project(root: &Path) -> Self {
        let config_dir = ["rustfmt.toml", ".rustfmt.toml"]
            .iter()
            .any(|name| root.join(name).is_file())
            .then(|| root.to_path_buf());
        Self {
            config_dir,
            edition: project_edition(root),
        }
    }

    fn format(&self, text: &str) -> Result<String, String> {
        use std::io::Write;

        let mut command = Command::new("rustfmt");
        command.args(["--edition", &self.edition]);
        if let Some(dir) = &self.config_dir {
            command.arg("--config-path").arg(dir);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("could not run rustfmt: {err}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .map_err(|err| format!("could not write to rustfmt: {err}"))?;
        }
        let output = child
            .wait_with_output()
            .map_err(|err| format!("rustfmt failed: {err}"))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"))
        } else {
            Err(String::from_utf8_lossy(&output.stderr).into_owned())
        }
    }
}

/// `[workspace.package].edition` or `[package].edition`, else 2024.
fn project_edition(root: &Path) -> String {
    let manifest: Option<toml::Table> = std::fs::read_to_string(root.join("Cargo.toml"))
        .ok()
        .and_then(|text| toml::from_str(&text).ok());
    let edition = |table: Option<&toml::Value>| {
        table
            .and_then(|table| table.get("edition"))
            .and_then(toml::Value::as_str)
            .map(str::to_string)
    };
    manifest
        .as_ref()
        .and_then(|manifest| {
            edition(manifest.get("workspace").and_then(|ws| ws.get("package")))
                .or_else(|| edition(manifest.get("package")))
        })
        .unwrap_or_else(|| "2024".to_string())
}

fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.trim_end().lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}
