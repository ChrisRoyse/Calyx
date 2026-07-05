use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde_json::Value;

use super::model::{
    AssociationHypothesis, ConceptNode, ScanOutput, TypedAssociationMinerArgs, TypedPath,
    new_hypothesis,
};
use crate::error::{CliError, CliResult};

pub(super) fn load_nodes(path: &Path) -> CliResult<BTreeMap<String, ConceptNode>> {
    let mut out = BTreeMap::new();
    for_jsonl(path, usize::MAX, |row, _line| {
        if str_field(row, "node_type") != "concept" {
            return Ok(());
        }
        let node = ConceptNode {
            node_id: str_field(row, "node_id"),
            normalized_name: str_field(row, "normalized_name"),
            concept_type: str_field(row, "concept_type").to_ascii_lowercase(),
        };
        out.insert(node.node_id.clone(), node);
        Ok(())
    })?;
    if out.is_empty() {
        return Err(CliError::runtime("typed_nodes.jsonl had no concept nodes"));
    }
    Ok(out)
}

pub(super) fn scan_edges(
    args: &TypedAssociationMinerArgs,
    nodes: &BTreeMap<String, ConceptNode>,
) -> CliResult<ScanOutput> {
    let mut grouped = BTreeMap::<(String, String), AssociationHypothesis>::new();
    let mut scanned = 0_usize;
    let limit_reached = for_jsonl(
        &args.typed_root.join("typed_edges.jsonl"),
        args.max_input_edges,
        |row, _| {
            scanned += 1;
            absorb_edge(args, nodes, row, &mut grouped)
        },
    )?;
    let max_support = grouped
        .values()
        .map(|candidate| candidate.support_count)
        .max()
        .unwrap_or(1);
    Ok(ScanOutput {
        input_edges: scanned,
        limit_reached,
        max_support,
        candidates: grouped.into_values().collect(),
    })
}

fn absorb_edge(
    args: &TypedAssociationMinerArgs,
    nodes: &BTreeMap<String, ConceptNode>,
    row: &Value,
    grouped: &mut BTreeMap<(String, String), AssociationHypothesis>,
) -> CliResult {
    if str_field(row, "edge_type") != "associated_with" {
        return Ok(());
    }
    let support = usize_field(row, "support_count").unwrap_or(0);
    if support < args.min_support {
        return Ok(());
    }
    if args
        .source_issue
        .is_some_and(|issue| u64_field(row, "source_issue") != Some(issue))
    {
        return Ok(());
    }
    let Some(source) = nodes.get(&str_field(row, "source")) else {
        return Ok(());
    };
    let Some(target) = nodes.get(&str_field(row, "target")) else {
        return Ok(());
    };
    let Some((source, target)) = oriented_pair(args, source, target) else {
        return Ok(());
    };
    let (source, target) = canonicalize_unscoped_pair(args, source, target);
    let key = (source.node_id.clone(), target.node_id.clone());
    let entry = grouped
        .entry(key)
        .or_insert_with(|| new_hypothesis(source, target));
    entry.support_count += support;
    entry.path_count += 1;
    if entry.typed_paths.len() < args.max_paths_per_pair {
        entry
            .typed_paths
            .push(path(row, support, args.max_paths_per_pair));
    }
    Ok(())
}

fn oriented_pair<'a>(
    args: &TypedAssociationMinerArgs,
    source: &'a ConceptNode,
    target: &'a ConceptNode,
) -> Option<(&'a ConceptNode, &'a ConceptNode)> {
    if pair_matches(args, source, target) {
        Some((source, target))
    } else if pair_matches(args, target, source) {
        Some((target, source))
    } else {
        None
    }
}

fn canonicalize_unscoped_pair<'a>(
    args: &TypedAssociationMinerArgs,
    source: &'a ConceptNode,
    target: &'a ConceptNode,
) -> (&'a ConceptNode, &'a ConceptNode) {
    if args.source_type.is_none() && args.target_type.is_none() && source.node_id > target.node_id {
        (target, source)
    } else {
        (source, target)
    }
}

fn pair_matches(
    args: &TypedAssociationMinerArgs,
    source: &ConceptNode,
    target: &ConceptNode,
) -> bool {
    type_match(args.source_type.as_deref(), source)
        && type_match(args.target_type.as_deref(), target)
        && name_match(args.name_contains.as_deref(), source, target)
}

fn path(row: &Value, support: usize, max_paths: usize) -> TypedPath {
    TypedPath {
        edge_id: str_field(row, "edge_id"),
        edge_type: str_field(row, "edge_type"),
        support_count: support,
        source_issue: u64_field(row, "source_issue"),
        source_hashes: string_array(row, "source_hash", max_paths),
        support_cx_ids: string_array(row, "support_cx_ids", max_paths),
    }
}

fn for_jsonl(
    path: &Path,
    max_lines: usize,
    mut f: impl FnMut(&Value, usize) -> CliResult,
) -> CliResult<bool> {
    let file = File::open(path)
        .map_err(|error| CliError::io(format!("read {}: {error}", path.display())))?;
    let mut limit_reached = false;
    for (idx, line) in BufReader::new(file).lines().enumerate() {
        if idx >= max_lines {
            limit_reached = true;
            break;
        }
        let line =
            line.map_err(|error| CliError::io(format!("read {}: {error}", path.display())))?;
        if line.trim().is_empty() {
            continue;
        }
        let row = serde_json::from_str(&line).map_err(|error| {
            CliError::runtime(format!(
                "parse {} line {}: {error}",
                path.display(),
                idx + 1
            ))
        })?;
        f(&row, idx + 1)?;
    }
    Ok(limit_reached)
}

fn type_match(filter: Option<&str>, node: &ConceptNode) -> bool {
    filter.is_none_or(|filter| node.concept_type == filter)
}

fn name_match(filter: Option<&str>, source: &ConceptNode, target: &ConceptNode) -> bool {
    filter.is_none_or(|needle| {
        source.normalized_name.to_ascii_lowercase().contains(needle)
            || target.normalized_name.to_ascii_lowercase().contains(needle)
    })
}

fn str_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn u64_field(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(|raw| {
        raw.as_u64()
            .or_else(|| raw.as_i64().and_then(|v| u64::try_from(v).ok()))
    })
}

fn usize_field(value: &Value, key: &str) -> Option<usize> {
    u64_field(value, key).and_then(|value| usize::try_from(value).ok())
}

fn string_array(value: &Value, key: &str, max: usize) -> Vec<String> {
    match value.get(key) {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .take(max)
            .map(str::to_string)
            .collect(),
        Some(Value::String(value)) => vec![value.clone()],
        _ => Vec::new(),
    }
}
