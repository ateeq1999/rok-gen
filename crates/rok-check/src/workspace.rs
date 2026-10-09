//! The workspace as the architecture rules see it: its member crates and
//! the path dependencies between them, read straight from the manifests
//! (no `cargo metadata`, so no network or lockfile resolution is needed).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use toml::{Table, Value};

use crate::arch::ArchError;

/// One workspace member crate.
#[derive(Debug)]
pub struct Member {
    pub name: String,
    pub dir: PathBuf,
    /// The manifest, relative to the project root.
    pub manifest_rel: String,
    pub manifest_text: String,
    /// Normal and build dependencies on other members: member name → the
    /// key it's declared under (they differ for renamed dependencies).
    pub deps: BTreeMap<String, String>,
}

/// Where the workspace is read from: the disk, or pending changes on top of
/// it. Paths are absolute (under the project root).
pub trait Files {
    /// # Errors
    ///
    /// When the file doesn't exist or can't be read.
    fn read(&self, path: &Path) -> std::io::Result<String>;
    /// The direct children (files and directories) of `dir`.
    ///
    /// # Errors
    ///
    /// When `dir` can't be listed.
    fn children(&self, dir: &Path) -> std::io::Result<Vec<PathBuf>>;
    fn is_file(&self, path: &Path) -> bool;
}

/// The files as they are on disk.
pub struct Disk;

impl Files for Disk {
    fn read(&self, path: &Path) -> std::io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn children(&self, dir: &Path) -> std::io::Result<Vec<PathBuf>> {
        Ok(std::fs::read_dir(dir)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect())
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }
}

#[derive(Debug)]
pub struct Workspace {
    pub members: Vec<Member>,
}

impl Workspace {
    pub fn load(root: &Path) -> Result<Self, ArchError> {
        Self::load_from(root, &Disk)
    }

    /// [`Workspace::load`], reading through `files`.
    pub fn load_from(root: &Path, files: &dyn Files) -> Result<Self, ArchError> {
        let (root_text, root_manifest) = read_manifest(files, &root.join("Cargo.toml"))?;
        let workspace = root_manifest.get("workspace").and_then(Value::as_table);
        let workspace_paths: BTreeSet<String> = workspace
            .and_then(|ws| ws.get("dependencies"))
            .and_then(Value::as_table)
            .map(|deps| {
                deps.iter()
                    .filter(|(_, spec)| spec.get("path").is_some())
                    .map(|(key, _)| key.clone())
                    .collect()
            })
            .unwrap_or_default();

        let mut dirs = Vec::new();
        if root_manifest.contains_key("package") {
            dirs.push(root.to_path_buf());
        }
        let patterns = workspace
            .and_then(|ws| ws.get("members"))
            .and_then(Value::as_array)
            .map(|members| members.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .unwrap_or_default();
        for pattern in patterns {
            dirs.extend(expand_member(files, root, pattern)?);
        }

        let mut raw = Vec::new();
        for dir in dirs {
            let path = dir.join("Cargo.toml");
            let (text, manifest) = if dir == root {
                (root_text.clone(), root_manifest.clone())
            } else {
                read_manifest(files, &path)?
            };
            let Some(name) = manifest
                .get("package")
                .and_then(|package| package.get("name"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            let deps = path_dependencies(&manifest, &workspace_paths);
            raw.push((name.to_string(), dir, relative(root, &path), text, deps));
        }

        let names: BTreeSet<String> = raw.iter().map(|(name, ..)| name.clone()).collect();
        let members = raw
            .into_iter()
            .map(|(name, dir, manifest_rel, manifest_text, deps)| Member {
                deps: deps
                    .into_iter()
                    .filter(|(package, _)| names.contains(package) && *package != name)
                    .collect(),
                name,
                dir,
                manifest_rel,
                manifest_text,
            })
            .collect();
        Ok(Self { members })
    }

    pub fn member(&self, name: &str) -> Option<&Member> {
        self.members.iter().find(|member| member.name == name)
    }
}

impl Member {
    /// The 1-based line declaring dependency `key` in this manifest, or 1.
    pub fn dependency_line(&self, key: &str) -> usize {
        self.manifest_text
            .lines()
            .position(|line| {
                line.trim_start()
                    .strip_prefix(key)
                    .is_some_and(|rest| rest.starts_with([' ', '=', '.']))
            })
            .map_or(1, |index| index + 1)
    }
}

/// `path` relative to `root`, with `/` separators.
pub fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn read_manifest(files: &dyn Files, path: &Path) -> Result<(String, Table), ArchError> {
    let text = files.read(path).map_err(|source| ArchError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let table = toml::from_str(&text).map_err(|err: toml::de::Error| ArchError::Manifest {
        path: path.to_path_buf(),
        message: err.to_string(),
    })?;
    Ok((text, table))
}

/// Expands one `workspace.members` entry. Supports exact paths and a
/// trailing `/*` (every direct subdirectory with a `Cargo.toml`).
fn expand_member(files: &dyn Files, root: &Path, pattern: &str) -> Result<Vec<PathBuf>, ArchError> {
    let Some(parent) = pattern.strip_suffix("/*") else {
        return Ok(vec![root.join(pattern)]);
    };
    let parent = root.join(parent);
    let entries = files.children(&parent).map_err(|source| ArchError::Read {
        path: parent.clone(),
        source,
    })?;
    let mut dirs: Vec<PathBuf> = entries
        .into_iter()
        .filter(|dir| files.is_file(&dir.join("Cargo.toml")))
        .collect();
    dirs.sort();
    Ok(dirs)
}

/// Package name → declared key, for every path (or workspace-path)
/// dependency in `[dependencies]`, `[build-dependencies]` and their
/// `[target.*]` forms. Dev-dependencies are deliberately excluded: test-only
/// edges aren't architecture.
fn path_dependencies(
    manifest: &Table,
    workspace_paths: &BTreeSet<String>,
) -> BTreeMap<String, String> {
    let mut tables: Vec<&Table> = Vec::new();
    for section in ["dependencies", "build-dependencies"] {
        if let Some(table) = manifest.get(section).and_then(Value::as_table) {
            tables.push(table);
        }
    }
    if let Some(targets) = manifest.get("target").and_then(Value::as_table) {
        for target in targets.values() {
            for section in ["dependencies", "build-dependencies"] {
                if let Some(table) = target.get(section).and_then(Value::as_table) {
                    tables.push(table);
                }
            }
        }
    }

    let mut deps = BTreeMap::new();
    for table in tables {
        for (key, spec) in table {
            let is_path = spec.get("path").is_some()
                || (spec.get("workspace").and_then(Value::as_bool) == Some(true)
                    && workspace_paths.contains(key));
            if !is_path {
                continue;
            }
            let package = spec
                .get("package")
                .and_then(Value::as_str)
                .unwrap_or(key)
                .to_string();
            deps.insert(package, key.clone());
        }
    }
    deps
}
