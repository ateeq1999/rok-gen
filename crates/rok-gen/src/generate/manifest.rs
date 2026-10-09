//! Adding dependency lines to `Cargo.toml` as text, so comments, grouping
//! and ordering survive: a workspace-internal dependency goes into the
//! block of other internal ones, in sorted position.

use std::collections::BTreeSet;

/// The keys of `[workspace.dependencies]` that are path dependencies, i.e.
/// workspace-internal crates.
pub fn internal_crates(root_manifest: &str) -> BTreeSet<String> {
    let table: Option<toml::Table> = toml::from_str(root_manifest).ok();
    table
        .as_ref()
        .and_then(|table| table.get("workspace"))
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(toml::Value::as_table)
        .map(|deps| {
            deps.iter()
                .filter(|(_, spec)| spec.get("path").is_some())
                .map(|(key, _)| key.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// `text` with `line` (declaring `key`) added to `[section]`: among the
/// section's lines for `internal` crates, sorted by key; right after the
/// header if there are none; in a new section at the end if the section
/// doesn't exist. Unchanged if `key` is already declared there.
pub fn add_dependency(
    text: &str,
    section: &str,
    key: &str,
    line: &str,
    internal: &BTreeSet<String>,
) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let header = format!("[{section}]");
    let Some(start) = lines
        .iter()
        .position(|candidate| candidate.trim() == header)
    else {
        let mut output = text.trim_end().to_string();
        for part in ["\n\n", &header, "\n", line, "\n"] {
            output.push_str(part);
        }
        return output;
    };
    let end = lines[start + 1..]
        .iter()
        .position(|candidate| candidate.trim_start().starts_with('['))
        .map_or(lines.len(), |offset| start + 1 + offset);

    let key_of = |candidate: &str| -> Option<String> {
        let trimmed = candidate.trim_start();
        let end = trimmed.find(['=', '.', ' '])?;
        Some(trimmed[..end].trim().to_string())
    };
    if lines[start + 1..end]
        .iter()
        .any(|candidate| key_of(candidate).as_deref() == Some(key))
    {
        return text.to_string();
    }

    let internal_lines: Vec<(usize, String)> = (start + 1..end)
        .filter_map(|index| {
            key_of(lines[index])
                .filter(|existing| internal.contains(existing))
                .map(|existing| (index, existing))
        })
        .collect();
    let insert_at = internal_lines
        .iter()
        .find(|(_, existing)| existing.as_str() > key)
        .map(|(index, _)| *index)
        .or_else(|| internal_lines.last().map(|(index, _)| index + 1))
        .unwrap_or(start + 1);

    let mut output: Vec<&str> = lines[..insert_at].to_vec();
    output.push(line);
    output.extend_from_slice(&lines[insert_at..]);
    let mut joined = output.join("\n");
    if text.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.dependencies]\nauth = { path = \"crates/auth\" }\nusers = { path = \"crates/users\" }\n\nactix-web = \"4\"\n\n[package]\nname = \"app\"\n\n[dependencies]\nauth.workspace = true\nusers.workspace = true\n\nactix-web.workspace = true\n";

    fn internal() -> BTreeSet<String> {
        internal_crates(ROOT)
    }

    #[test]
    fn internal_crates_are_the_path_dependencies() {
        assert_eq!(
            internal().into_iter().collect::<Vec<_>>(),
            ["auth", "users"]
        );
    }

    #[test]
    fn lines_go_into_the_internal_block_in_sorted_order() {
        let out = add_dependency(
            ROOT,
            "workspace.dependencies",
            "posts",
            "posts = { path = \"crates/posts\" }",
            &internal(),
        );
        assert!(out.contains(
            "auth = { path = \"crates/auth\" }\nposts = { path = \"crates/posts\" }\nusers = "
        ));
        let out = add_dependency(
            ROOT,
            "dependencies",
            "zebra",
            "zebra.workspace = true",
            &internal(),
        );
        assert!(out.contains("users.workspace = true\nzebra.workspace = true\n\nactix-web"));
    }

    #[test]
    fn existing_keys_are_left_alone_and_missing_sections_are_added() {
        assert_eq!(
            add_dependency(
                ROOT,
                "dependencies",
                "auth",
                "auth.workspace = true",
                &internal()
            ),
            ROOT
        );
        let out = add_dependency(
            "[package]\nname = \"x\"\n",
            "dependencies",
            "posts",
            "posts.workspace = true",
            &internal(),
        );
        assert_eq!(
            out,
            "[package]\nname = \"x\"\n\n[dependencies]\nposts.workspace = true\n"
        );
    }
}
