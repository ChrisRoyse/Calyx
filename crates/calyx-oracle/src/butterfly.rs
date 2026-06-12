//! Hop-attenuated Oracle butterfly tree traversal.

mod context;

use std::collections::{BTreeMap, BTreeSet};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::recurrence::read_series;
use calyx_aster::vault::{AsterVault, encode};
use calyx_core::{AnchorValue, Clock, Constellation, LedgerRef, VaultStore, content_address};
use calyx_ledger::{ActorId, EntryKind, SubjectId};
use serde::Serialize;

use context::ExpansionContext;

use crate::{
    Consequence, ConsequenceTree, DEFAULT_CONSEQUENCE_TREE_MAX_DEPTH, DomainId,
    ORACLE_ACTION_METADATA_KEY, ORACLE_DOMAIN_METADATA_KEY, ORACLE_FALLBACK_DOMAIN_METADATA_KEY,
    OracleError,
};

pub const MAX_DEPTH: u8 = DEFAULT_CONSEQUENCE_TREE_MAX_DEPTH;
pub const HOP_ATTENUATION: f32 = 0.7;
pub const MIN_CONFIDENCE_THRESHOLD: f32 = 0.05;

const ORACLE_FALLBACK_ACTION_METADATA_KEY: &str = "action";
const LEDGER_ACTOR: &str = "calyx-oracle";
const LEDGER_TAG: &str = "oracle_expand_v1";
const PROVISIONAL_SEQ: u64 = u64::MAX;

pub fn expand<C>(
    vault: &AsterVault<C>,
    consequence: &Consequence,
    clock: &dyn Clock,
) -> Result<Vec<Consequence>, OracleError>
where
    C: Clock,
{
    let tree = build_tree_internal(vault, consequence.clone(), clock)?;
    Ok(flatten_descendants(&tree))
}

pub fn build_tree<C>(
    vault: &AsterVault<C>,
    root: Consequence,
    clock: &dyn Clock,
) -> Result<ConsequenceTree, OracleError>
where
    C: Clock,
{
    build_tree_internal(vault, root, clock)
}

pub fn select<'a>(
    tree: &'a ConsequenceTree,
    desired_outcome: &AnchorValue,
) -> Option<&'a ConsequenceTree> {
    let mut best = None;
    select_terminal(tree, desired_outcome, &mut best);
    best.map(|(node, _)| node)
}

pub fn provisional_ledger_ref() -> LedgerRef {
    LedgerRef {
        seq: PROVISIONAL_SEQ,
        hash: [0; 32],
    }
}

pub fn is_provisional_ledger_ref(value: &LedgerRef) -> bool {
    value.seq == PROVISIONAL_SEQ && value.hash == [0; 32]
}

fn build_tree_internal<C>(
    vault: &AsterVault<C>,
    root: Consequence,
    clock: &dyn Clock,
) -> Result<ConsequenceTree, OracleError>
where
    C: Clock,
{
    let mut tree = ConsequenceTree {
        root,
        children: Vec::new(),
        max_depth: MAX_DEPTH,
    };
    let mut visited = BTreeSet::new();
    visited.insert(NodeKey::from_consequence(&tree.root));
    let mut stats = ExpansionStats::default();
    expand_node(vault, &mut tree, &mut visited, &mut stats)?;
    let ledger_ref = write_expansion_ledger(vault, &tree.root, &stats, clock)?;
    apply_grounded_provenance(&mut tree, &ledger_ref);
    Ok(tree)
}

fn expand_node<C>(
    vault: &AsterVault<C>,
    node: &mut ConsequenceTree,
    visited: &mut BTreeSet<NodeKey>,
    stats: &mut ExpansionStats,
) -> Result<(), OracleError>
where
    C: Clock,
{
    stats.nodes_visited += 1;
    if node.root.hop >= MAX_DEPTH {
        stats.depth_prunes += 1;
        return Ok(());
    }
    let child_confidence = attenuate(node.root.confidence);
    if child_confidence < MIN_CONFIDENCE_THRESHOLD {
        stats.threshold_prunes += 1;
        return Ok(());
    }

    stats.expand_calls += 1;
    let candidates = outgoing_candidates(vault, &node.root, stats)?;
    for candidate in candidates {
        let key = NodeKey::new(&candidate.domain, &candidate.action_or_event);
        if visited.contains(&key) {
            stats.cycle_skips += 1;
            continue;
        }
        let mut child = ConsequenceTree {
            root: Consequence {
                action_or_event: candidate.action_or_event,
                domain: candidate.domain,
                outcome: candidate.outcome,
                confidence: child_confidence,
                hop: node.root.hop.saturating_add(1),
                provenance: if candidate.grounded {
                    pending_ledger_ref()
                } else {
                    provisional_ledger_ref()
                },
            },
            children: Vec::new(),
            max_depth: MAX_DEPTH,
        };
        stats.children_emitted += 1;
        if candidate.grounded {
            visited.insert(key.clone());
            expand_node(vault, &mut child, visited, stats)?;
            visited.remove(&key);
        } else {
            stats.provisional_edges += 1;
        }
        node.children.push(child);
    }
    Ok(())
}

fn outgoing_candidates<C>(
    vault: &AsterVault<C>,
    parent: &Consequence,
    stats: &mut ExpansionStats,
) -> Result<Vec<ChildCandidate>, OracleError>
where
    C: Clock,
{
    let mut out = BTreeMap::<ChildKey, ChildCandidate>::new();
    let rows = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
        .map_err(|_| OracleError::NoRecurrence {
            domain: parent.domain.clone(),
        })?;
    stats.base_rows_scanned += rows.len() as u64;
    for (_, bytes) in rows {
        let cx =
            encode::decode_constellation_base(&bytes).map_err(|_| OracleError::NoRecurrence {
                domain: parent.domain.clone(),
            })?;
        if !matches_domain(&cx, &parent.domain) {
            continue;
        }
        collect_candidates(vault, &cx, parent, stats, &mut out)?;
    }
    Ok(out.into_values().collect())
}

fn collect_candidates<C>(
    vault: &AsterVault<C>,
    cx: &Constellation,
    parent: &Consequence,
    stats: &mut ExpansionStats,
    out: &mut BTreeMap<ChildKey, ChildCandidate>,
) -> Result<(), OracleError>
where
    C: Clock,
{
    let base_action_match = matches_action(cx, &parent.action_or_event);
    let series = read_series(vault, cx.cx_id).map_err(|_| OracleError::NoRecurrence {
        domain: parent.domain.clone(),
    })?;
    stats.recurrence_rows_scanned += series.occurrences.len() as u64;
    for occurrence in &series.occurrences {
        if occurrence.context.bytes.is_empty() {
            continue;
        }
        let parsed: ExpansionContext =
            serde_json::from_slice(&occurrence.context.bytes).map_err(|_| {
                OracleError::NoRecurrence {
                    domain: parent.domain.clone(),
                }
            })?;
        if !parsed.matches_action(&parent.action_or_event, base_action_match) {
            continue;
        }
        for seed in parsed.consequences() {
            let outcome_label =
                outcome_label(&seed.outcome).map_err(|_| OracleError::NoRecurrence {
                    domain: parent.domain.clone(),
                })?;
            let key = ChildKey {
                domain: seed.domain.as_str().to_string(),
                action_or_event: seed.action_or_event.clone(),
                outcome_label,
            };
            out.entry(key).or_insert(seed);
        }
    }
    Ok(())
}

fn write_expansion_ledger<C>(
    vault: &AsterVault<C>,
    root: &Consequence,
    stats: &ExpansionStats,
    clock: &dyn Clock,
) -> Result<LedgerRef, OracleError>
where
    C: Clock,
{
    let payload = ExpansionLedgerPayload {
        tag: LEDGER_TAG,
        root_domain_id: hex_bytes(&domain_digest(&root.domain)),
        root_action_digest: hex_bytes(&content_address([root.action_or_event.as_bytes()])),
        root_outcome_digest: hex_bytes(&content_address([outcome_label(&root.outcome)
            .map_err(|_| OracleError::LedgerWriteFailure)?
            .as_bytes()])),
        root_hop: root.hop,
        root_confidence: unit(root.confidence),
        max_depth: MAX_DEPTH,
        hop_attenuation: HOP_ATTENUATION,
        min_confidence_threshold: MIN_CONFIDENCE_THRESHOLD,
        expand_calls: stats.expand_calls,
        nodes_visited: stats.nodes_visited,
        children_emitted: stats.children_emitted,
        provisional_edges: stats.provisional_edges,
        cycle_skips: stats.cycle_skips,
        depth_prunes: stats.depth_prunes,
        threshold_prunes: stats.threshold_prunes,
        base_rows_scanned: stats.base_rows_scanned,
        recurrence_rows_scanned: stats.recurrence_rows_scanned,
        ts: clock.now(),
    };
    let bytes = serde_json::to_vec(&payload).map_err(|_| OracleError::LedgerWriteFailure)?;
    vault
        .append_ledger_entry(
            EntryKind::Answer,
            SubjectId::Query(expansion_digest(root).to_vec()),
            bytes,
            ActorId::Service(LEDGER_ACTOR.to_string()),
        )
        .map_err(|_| OracleError::LedgerWriteFailure)
}

fn apply_grounded_provenance(tree: &mut ConsequenceTree, ledger_ref: &LedgerRef) {
    for child in &mut tree.children {
        if !is_provisional_ledger_ref(&child.root.provenance) {
            child.root.provenance = ledger_ref.clone();
        }
        apply_grounded_provenance(child, ledger_ref);
    }
}

fn pending_ledger_ref() -> LedgerRef {
    LedgerRef {
        seq: 0,
        hash: [0; 32],
    }
}

fn flatten_descendants(tree: &ConsequenceTree) -> Vec<Consequence> {
    let mut out = Vec::new();
    for child in &tree.children {
        out.push(child.root.clone());
        out.extend(flatten_descendants(child));
    }
    out
}

fn select_terminal<'a>(
    node: &'a ConsequenceTree,
    desired: &AnchorValue,
    best: &mut Option<(&'a ConsequenceTree, f32)>,
) {
    if node.children.is_empty() {
        if let Some(score) = anchor_score(&node.root.outcome, desired) {
            let replace = best.as_ref().is_none_or(|(_, current)| score > *current);
            if replace {
                *best = Some((node, score));
            }
        }
        return;
    }
    for child in &node.children {
        select_terminal(child, desired, best);
    }
}

fn anchor_score(actual: &AnchorValue, desired: &AnchorValue) -> Option<f32> {
    match (actual, desired) {
        (AnchorValue::Bool(left), AnchorValue::Bool(right)) => exact_score(left == right),
        (AnchorValue::Enum(left), AnchorValue::Enum(right))
        | (AnchorValue::Text(left), AnchorValue::Text(right)) => exact_score(left == right),
        (AnchorValue::Number(left), AnchorValue::Number(right))
            if left.is_finite() && right.is_finite() =>
        {
            Some((1.0 / (1.0 + (left - right).abs())) as f32)
        }
        (AnchorValue::OneHot(left), AnchorValue::OneHot(right)) => jaccard_score(left, right),
        (AnchorValue::Vector(left), AnchorValue::Vector(right)) => cosine_score(left, right),
        _ => None,
    }
}

fn exact_score(matches: bool) -> Option<f32> {
    matches.then_some(1.0)
}

fn jaccard_score(left: &[String], right: &[String]) -> Option<f32> {
    let left = left.iter().collect::<BTreeSet<_>>();
    let right = right.iter().collect::<BTreeSet<_>>();
    let union = left.union(&right).count();
    if union == 0 {
        return None;
    }
    let score = left.intersection(&right).count() as f32 / union as f32;
    (score > 0.0).then_some(score)
}

fn cosine_score(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.len() != right.len() || left.is_empty() {
        return None;
    }
    let mut dot = 0.0_f32;
    let mut left_norm = 0.0_f32;
    let mut right_norm = 0.0_f32;
    for (l, r) in left.iter().zip(right) {
        if !l.is_finite() || !r.is_finite() {
            return None;
        }
        dot += l * r;
        left_norm += l * l;
        right_norm += r * r;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        return None;
    }
    Some(dot / left_norm.sqrt() / right_norm.sqrt())
}

fn attenuate(confidence: f32) -> f32 {
    (unit(confidence) * HOP_ATTENUATION).clamp(0.0, 1.0)
}

fn unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn matches_domain(cx: &Constellation, domain: &DomainId) -> bool {
    cx.metadata_value(ORACLE_DOMAIN_METADATA_KEY) == Some(domain.as_str())
        || cx.metadata_value(ORACLE_FALLBACK_DOMAIN_METADATA_KEY) == Some(domain.as_str())
}

fn matches_action(cx: &Constellation, action_id: &str) -> bool {
    cx.metadata_value(ORACLE_ACTION_METADATA_KEY) == Some(action_id)
        || cx.metadata_value(ORACLE_FALLBACK_ACTION_METADATA_KEY) == Some(action_id)
}

fn outcome_label(value: &AnchorValue) -> Result<String, serde_json::Error> {
    serde_json::to_string(value)
}

fn expansion_digest(root: &Consequence) -> [u8; 16] {
    content_address([
        root.domain.as_str().as_bytes(),
        root.action_or_event.as_bytes(),
        &root.hop.to_be_bytes(),
    ])
}

fn domain_digest(domain: &DomainId) -> [u8; 16] {
    content_address([domain.as_str().as_bytes()])
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct NodeKey {
    domain: String,
    action_or_event: String,
}

impl NodeKey {
    fn new(domain: &DomainId, action_or_event: &str) -> Self {
        Self {
            domain: domain.as_str().to_string(),
            action_or_event: action_or_event.to_string(),
        }
    }

    fn from_consequence(value: &Consequence) -> Self {
        Self::new(&value.domain, &value.action_or_event)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ChildKey {
    domain: String,
    action_or_event: String,
    outcome_label: String,
}

#[derive(Clone, Debug, PartialEq)]
struct ChildCandidate {
    action_or_event: String,
    domain: DomainId,
    outcome: AnchorValue,
    grounded: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
struct ExpansionStats {
    expand_calls: u64,
    nodes_visited: u64,
    children_emitted: u64,
    provisional_edges: u64,
    cycle_skips: u64,
    depth_prunes: u64,
    threshold_prunes: u64,
    base_rows_scanned: u64,
    recurrence_rows_scanned: u64,
}

#[derive(Clone, Debug, Serialize)]
struct ExpansionLedgerPayload {
    tag: &'static str,
    root_domain_id: String,
    root_action_digest: String,
    root_outcome_digest: String,
    root_hop: u8,
    root_confidence: f32,
    max_depth: u8,
    hop_attenuation: f32,
    min_confidence_threshold: f32,
    expand_calls: u64,
    nodes_visited: u64,
    children_emitted: u64,
    provisional_edges: u64,
    cycle_skips: u64,
    depth_prunes: u64,
    threshold_prunes: u64,
    base_rows_scanned: u64,
    recurrence_rows_scanned: u64,
    ts: u64,
}

#[cfg(test)]
#[path = "butterfly_tests.rs"]
mod tests;
