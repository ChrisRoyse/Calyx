use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde_json::Value;

use super::model::{
    AssociationHypothesis, BlockedAssociationCandidate, ConceptNode, ScanOutput,
    TypedAssociationMinerArgs, TypedPath, new_hypothesis,
};
use crate::cmd::mechanistic_direction::{
    MechanisticDirectionEvidence, MutationConsequence, TargetModulation,
    infer_observed_target_modulation, infer_required_target_modulation, modulation_name,
    mutation_consequence_name,
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
    let mut blocked_candidates = Vec::new();
    let mut scanned = 0_usize;
    let limit_reached = for_jsonl(
        &args.typed_root.join("typed_edges.jsonl"),
        args.max_input_edges,
        |row, _| {
            scanned += 1;
            absorb_edge(args, nodes, row, &mut grouped, &mut blocked_candidates)
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
        blocked_candidates,
    })
}

fn absorb_edge(
    args: &TypedAssociationMinerArgs,
    nodes: &BTreeMap<String, ConceptNode>,
    row: &Value,
    grouped: &mut BTreeMap<(String, String), AssociationHypothesis>,
    blocked: &mut Vec<BlockedAssociationCandidate>,
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
    let mechanistic = is_mechanistic_unordered_pair(source, target);
    let Some((source, target)) = oriented_pair(args, source, target, mechanistic) else {
        return Ok(());
    };
    let (source, target) = canonicalize_unscoped_pair(args, source, target, mechanistic);
    let direction = if is_mechanistic_pair(source, target) {
        let direction = direction_for_pair(row, source, target);
        if !direction_accepted(source, target, &direction) {
            blocked.push(blocked_candidate(row, source, target, direction));
            return Ok(());
        }
        Some(direction)
    } else if mechanistic {
        let direction = unsupported_mechanistic_orientation();
        blocked.push(blocked_candidate(row, source, target, direction));
        return Ok(());
    } else {
        None
    };
    let key = (source.node_id.clone(), target.node_id.clone());
    let entry = grouped
        .entry(key)
        .or_insert_with(|| new_hypothesis(source, target));
    if let Some(direction) = &direction
        && !merge_direction(entry, direction)
    {
        blocked.push(blocked_candidate(row, source, target, direction.clone()));
        return Ok(());
    }
    entry.support_count += support;
    entry.path_count += 1;
    if entry.typed_paths.len() < args.max_paths_per_pair {
        entry
            .typed_paths
            .push(path(row, support, args.max_paths_per_pair, direction));
    }
    Ok(())
}

fn oriented_pair<'a>(
    args: &TypedAssociationMinerArgs,
    source: &'a ConceptNode,
    target: &'a ConceptNode,
    mechanistic: bool,
) -> Option<(&'a ConceptNode, &'a ConceptNode)> {
    if pair_matches(args, source, target) {
        Some((source, target))
    } else if !mechanistic && pair_matches(args, target, source) {
        Some((target, source))
    } else {
        None
    }
}

fn canonicalize_unscoped_pair<'a>(
    args: &TypedAssociationMinerArgs,
    source: &'a ConceptNode,
    target: &'a ConceptNode,
    mechanistic: bool,
) -> (&'a ConceptNode, &'a ConceptNode) {
    if !mechanistic
        && args.source_type.is_none()
        && args.target_type.is_none()
        && source.node_id > target.node_id
    {
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

fn path(
    row: &Value,
    support: usize,
    max_paths: usize,
    direction: Option<MechanisticDirectionEvidence>,
) -> TypedPath {
    TypedPath {
        edge_id: str_field(row, "edge_id"),
        edge_type: str_field(row, "edge_type"),
        support_count: support,
        source_issue: u64_field(row, "source_issue"),
        source_hashes: string_array(row, "source_hash", max_paths),
        support_cx_ids: string_array(row, "support_cx_ids", max_paths),
        mechanistic_direction: direction,
    }
}

fn is_mechanistic_pair(source: &ConceptNode, target: &ConceptNode) -> bool {
    is_gene_like(source) && target.concept_type == "disease"
        || source.concept_type == "chemical" && is_gene_like(target)
}

fn is_mechanistic_unordered_pair(source: &ConceptNode, target: &ConceptNode) -> bool {
    (is_gene_like(source) && target.concept_type == "disease")
        || (source.concept_type == "disease" && is_gene_like(target))
        || (source.concept_type == "chemical" && is_gene_like(target))
        || (is_gene_like(source) && target.concept_type == "chemical")
}

fn is_gene_like(node: &ConceptNode) -> bool {
    matches!(node.concept_type.as_str(), "gene" | "gene_protein")
}

fn unsupported_mechanistic_orientation() -> MechanisticDirectionEvidence {
    MechanisticDirectionEvidence {
        status: "direction_blocked".to_string(),
        reason_codes: vec!["CALYX_MECH_UNSUPPORTED_ASSERTED_ORIENTATION".to_string()],
        ..MechanisticDirectionEvidence::default()
    }
}

fn direction_for_pair(
    row: &Value,
    source: &ConceptNode,
    target: &ConceptNode,
) -> MechanisticDirectionEvidence {
    if is_gene_like(source) && target.concept_type == "disease" {
        infer_required_target_modulation(row)
    } else if source.concept_type == "chemical" && is_gene_like(target) {
        infer_observed_target_modulation(row)
    } else {
        MechanisticDirectionEvidence {
            status: "not_mechanistic".to_string(),
            ..MechanisticDirectionEvidence::default()
        }
    }
}

fn direction_accepted(
    source: &ConceptNode,
    target: &ConceptNode,
    direction: &MechanisticDirectionEvidence,
) -> bool {
    if is_gene_like(source) && target.concept_type == "disease" {
        direction.is_required_direction_known()
    } else if source.concept_type == "chemical" && is_gene_like(target) {
        direction.is_observed_action_known()
    } else {
        true
    }
}

fn merge_direction(
    hypothesis: &mut AssociationHypothesis,
    direction: &MechanisticDirectionEvidence,
) -> bool {
    if let Some(required) = direction.required_target_modulation_name() {
        if let Some(existing) = hypothesis.required_target_modulation
            && modulation_name(existing) != Some(required.as_str())
        {
            return false;
        }
        hypothesis.required_target_modulation = Some(direction.required_target_modulation);
        hypothesis.mutation_consequence = Some(direction.mutation_consequence);
        hypothesis.mechanistic_direction_status = direction.status.clone();
    }
    if let Some(observed) = direction.observed_target_modulation_name() {
        if let Some(existing) = hypothesis.observed_target_modulation
            && modulation_name(existing) != Some(observed.as_str())
        {
            return false;
        }
        hypothesis.observed_target_modulation = Some(direction.observed_target_modulation);
        hypothesis.mechanistic_direction_status = direction.status.clone();
    }
    hypothesis
        .direction_reason_codes
        .extend(direction.reason_codes.iter().cloned());
    if direction.mutation_consequence != MutationConsequence::Unknown {
        hypothesis
            .direction_reason_codes
            .extend(mutation_consequence_name(direction.mutation_consequence).map(str::to_string));
    }
    if direction.required_target_modulation != TargetModulation::Unknown {
        hypothesis.direction_reason_codes.extend(
            modulation_name(direction.required_target_modulation)
                .map(|value| format!("required_target_modulation:{value}")),
        );
    }
    if direction.observed_target_modulation != TargetModulation::Unknown {
        hypothesis.direction_reason_codes.extend(
            modulation_name(direction.observed_target_modulation)
                .map(|value| format!("observed_target_modulation:{value}")),
        );
    }
    hypothesis.direction_reason_codes.sort();
    hypothesis.direction_reason_codes.dedup();
    true
}

fn blocked_candidate(
    row: &Value,
    source: &ConceptNode,
    target: &ConceptNode,
    direction: MechanisticDirectionEvidence,
) -> BlockedAssociationCandidate {
    BlockedAssociationCandidate {
        edge_id: str_field(row, "edge_id"),
        source_id: source.node_id.clone(),
        source_name: source.normalized_name.clone(),
        source_type: source.concept_type.clone(),
        target_id: target.node_id.clone(),
        target_name: target.normalized_name.clone(),
        target_type: target.concept_type.clone(),
        reason_codes: direction.reason_codes.clone(),
        mechanistic_direction: direction,
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
