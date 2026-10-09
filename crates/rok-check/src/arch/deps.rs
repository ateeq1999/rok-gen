//! Crate-level rules: `feature-edge` and `cycle`.

use std::collections::{BTreeMap, BTreeSet};

use crate::finding::{Finding, Rule};
use crate::workspace::Workspace;

pub fn check(
    workspace: &Workspace,
    features: &BTreeSet<String>,
    allowed: &[(String, String)],
) -> Vec<Finding> {
    let mut findings = feature_edges(workspace, features, allowed);
    findings.extend(cycles(workspace));
    findings
}

fn feature_edges(
    workspace: &Workspace,
    features: &BTreeSet<String>,
    allowed: &[(String, String)],
) -> Vec<Finding> {
    let mut findings = Vec::new();
    for member in workspace
        .members
        .iter()
        .filter(|member| features.contains(&member.name))
    {
        for (dependency, key) in &member.deps {
            let sanctioned = allowed
                .iter()
                .any(|(from, to)| *from == member.name && to == dependency);
            if features.contains(dependency) && !sanctioned {
                findings.push(Finding {
                    file: member.manifest_rel.clone(),
                    line: member.dependency_line(key),
                    column: 1,
                    rule: Rule::FeatureEdge,
                    message: format!(
                        "feature crate `{}` depends on feature crate `{dependency}`, which isn't \
                         sanctioned; add \"{} -> {dependency}\" to [architecture].allow in \
                         rok.toml if this edge is intended",
                        member.name, member.name
                    ),
                });
            }
        }
    }
    findings
}

/// Every elementary dependency cycle, each reported once (rotated to start
/// at its smallest crate name).
fn cycles(workspace: &Workspace) -> Vec<Finding> {
    let graph: BTreeMap<&str, Vec<&str>> = workspace
        .members
        .iter()
        .map(|member| {
            (
                member.name.as_str(),
                member.deps.keys().map(String::as_str).collect(),
            )
        })
        .collect();

    let mut found: BTreeSet<Vec<&str>> = BTreeSet::new();
    for &start in graph.keys() {
        let mut path = vec![start];
        walk(&graph, start, &mut path, &mut found);
    }

    found
        .into_iter()
        .filter_map(|cycle| {
            let first = workspace.member(cycle[0])?;
            let mut chain = cycle.clone();
            chain.push(cycle[0]);
            Some(Finding {
                file: first.manifest_rel.clone(),
                line: 1,
                column: 1,
                rule: Rule::Cycle,
                message: format!("dependency cycle: {}", chain.join(" -> ")),
            })
        })
        .collect()
}

/// Depth-first search for cycles back to `path[0]` that only pass through
/// crates ordered after it, so each cycle is discovered exactly once.
fn walk<'a>(
    graph: &BTreeMap<&'a str, Vec<&'a str>>,
    node: &'a str,
    path: &mut Vec<&'a str>,
    found: &mut BTreeSet<Vec<&'a str>>,
) {
    let start = path[0];
    for &next in graph.get(node).map(Vec::as_slice).unwrap_or_default() {
        if next == start {
            found.insert(path.clone());
        } else if next > start && !path.contains(&next) {
            path.push(next);
            walk(graph, next, path, found);
            path.pop();
        }
    }
}
