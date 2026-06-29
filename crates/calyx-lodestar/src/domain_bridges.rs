use calyx_core::{AnchorKind, CxId};
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::{AssocStore, KernelParams, LodestarError, Result, Scope, root_nodes_for_scope};

pub const DOMAIN_BRIDGE_SCHEMA_VERSION: u32 = 1;
const FREQUENCY_WEIGHT: f32 = 0.35;
const DEGREE_WEIGHT: f32 = 0.30;
const CENTRALITY_WEIGHT: f32 = 0.20;
const GROUNDING_WEIGHT: f32 = 0.15;

mod mining;
pub use mining::mine_domain_bridges;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct DomainPair {
    pub left: String,
    pub right: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainBridgeParams {
    pub min_gate_confidence: f32,
    pub max_per_pair: usize,
    pub max_evidence_hops: usize,
}

impl Default for DomainBridgeParams {
    fn default() -> Self {
        Self {
            min_gate_confidence: 0.25,
            max_per_pair: 32,
            max_evidence_hops: 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainBridgeScopePair {
    pub pair: DomainPair,
    pub left_scope: Scope,
    pub right_scope: Scope,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DomainBridgeMiningParams {
    pub ranking: DomainBridgeParams,
    pub kernel: KernelParams,
    pub anchor_kind: Option<AnchorKind>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainBridgeGateVerdict {
    pub passed: bool,
    pub confidence: f32,
    pub code: String,
    pub reason: String,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainBridgeInput {
    pub pair: DomainPair,
    pub cx_id: CxId,
    pub text: String,
    pub centrality_score: f32,
    pub cross_domain_distance: Option<usize>,
    pub gate: DomainBridgeGateVerdict,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainBridgeCandidate {
    pub pair: DomainPair,
    pub cx_id: CxId,
    pub text: String,
    pub frequency_weight: f32,
    pub degree: usize,
    pub degree_score: f32,
    pub centrality_score: f32,
    pub gate: DomainBridgeGateVerdict,
    pub cross_domain_distance: Option<usize>,
    pub provenance: Vec<String>,
    pub rank_score: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainBridgePairReport {
    pub pair: DomainPair,
    pub candidate_count: usize,
    pub refused_count: usize,
    pub candidates: Vec<DomainBridgeCandidate>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainBridgeReport {
    pub schema_version: u32,
    pub input_count: usize,
    pub pair_reports: Vec<DomainBridgePairReport>,
}

pub fn rank_domain_bridges(
    graph: &AssocGraph,
    inputs: &[DomainBridgeInput],
    params: &DomainBridgeParams,
) -> Result<DomainBridgeReport> {
    validate_params(params)?;
    let mut groups = BTreeMap::<DomainPair, PairAccumulator>::new();
    let max_frequency = max_frequency(graph);
    let degree_counts = degree_counts(graph);
    let max_degree = max_degree(&degree_counts);
    for input in inputs {
        validate_input(input)?;
        graph.require_node_index(input.cx_id)?;
        let entry = groups.entry(input.pair.clone()).or_default();
        if !input.gate.passed || input.gate.confidence < params.min_gate_confidence {
            entry.refused_count += 1;
            continue;
        }
        entry.candidates.push(candidate_from_input(
            graph,
            input,
            max_frequency,
            &degree_counts,
            max_degree,
        )?);
    }
    let pair_reports = groups
        .into_iter()
        .map(|(pair, mut group)| {
            sort_candidates(&mut group.candidates);
            group.candidates.truncate(params.max_per_pair);
            DomainBridgePairReport {
                pair,
                candidate_count: group.candidates.len(),
                refused_count: group.refused_count,
                candidates: group.candidates,
            }
        })
        .collect();
    Ok(DomainBridgeReport {
        schema_version: DOMAIN_BRIDGE_SCHEMA_VERSION,
        input_count: inputs.len(),
        pair_reports,
    })
}

fn candidate_from_input(
    graph: &AssocGraph,
    input: &DomainBridgeInput,
    max_frequency: f32,
    degree_counts: &BTreeMap<CxId, usize>,
    max_degree: usize,
) -> Result<DomainBridgeCandidate> {
    let frequency_weight = graph.node_weight(input.cx_id)?;
    let degree = degree_for(input.cx_id, degree_counts)?;
    let degree_score = degree as f32 / max_degree.max(1) as f32;
    let frequency_score = frequency_weight / max_frequency.max(f32::EPSILON);
    let rank_score = frequency_score * FREQUENCY_WEIGHT
        + degree_score * DEGREE_WEIGHT
        + input.centrality_score * CENTRALITY_WEIGHT
        + input.gate.confidence * GROUNDING_WEIGHT;
    Ok(DomainBridgeCandidate {
        pair: input.pair.clone(),
        cx_id: input.cx_id,
        text: input.text.clone(),
        frequency_weight,
        degree,
        degree_score,
        centrality_score: input.centrality_score,
        gate: input.gate.clone(),
        cross_domain_distance: input.cross_domain_distance,
        provenance: input.provenance.clone(),
        rank_score,
    })
}

fn sort_candidates(candidates: &mut [DomainBridgeCandidate]) {
    candidates.sort_by(|left, right| {
        right
            .rank_score
            .total_cmp(&left.rank_score)
            .then_with(|| right.frequency_weight.total_cmp(&left.frequency_weight))
            .then_with(|| right.degree.cmp(&left.degree))
            .then_with(|| left.cx_id.as_bytes().cmp(right.cx_id.as_bytes()))
    });
}

fn max_frequency(graph: &AssocGraph) -> f32 {
    graph
        .nodes()
        .iter()
        .map(|node| node.frequency_weight)
        .fold(0.0_f32, f32::max)
}

fn degree_counts(graph: &AssocGraph) -> BTreeMap<CxId, usize> {
    let mut counts = graph
        .node_ids()
        .map(|id| (id, 0_usize))
        .collect::<BTreeMap<_, _>>();
    for edge in graph.edges() {
        let (src, dst) = graph.edge_endpoints(*edge);
        *counts
            .get_mut(&src)
            .expect("edge source comes from graph node table") += 1;
        *counts
            .get_mut(&dst)
            .expect("edge destination comes from graph node table") += 1;
    }
    counts
}

fn max_degree(counts: &BTreeMap<CxId, usize>) -> usize {
    counts.values().copied().max().unwrap_or(1).max(1)
}

fn validate_params(params: &DomainBridgeParams) -> Result<()> {
    if !params.min_gate_confidence.is_finite() || !(0.0..=1.0).contains(&params.min_gate_confidence)
    {
        return invalid_params("min_gate_confidence must be finite and in [0,1]");
    }
    if params.max_per_pair == 0 {
        return invalid_params("max_per_pair must be greater than zero");
    }
    if params.max_evidence_hops == 0 {
        return invalid_params("max_evidence_hops must be greater than zero");
    }
    Ok(())
}

fn validate_pair(pair: &DomainPair) -> Result<()> {
    if pair.left.trim().is_empty() || pair.right.trim().is_empty() {
        return invalid_params("domain pair names must not be empty");
    }
    Ok(())
}

fn validate_input(input: &DomainBridgeInput) -> Result<()> {
    if input.pair.left.trim().is_empty() || input.pair.right.trim().is_empty() {
        return invalid_params("domain pair names must not be empty");
    }
    if !input.centrality_score.is_finite() || !(0.0..=1.0).contains(&input.centrality_score) {
        return invalid_params("centrality_score must be finite and in [0,1]");
    }
    if !input.gate.confidence.is_finite() || !(0.0..=1.0).contains(&input.gate.confidence) {
        return invalid_params("gate confidence must be finite and in [0,1]");
    }
    if input.gate.code.trim().is_empty() {
        return invalid_params("gate code must not be empty");
    }
    Ok(())
}

fn invalid_params<T>(detail: impl Into<String>) -> Result<T> {
    Err(LodestarError::KernelInvalidParams {
        detail: detail.into(),
    })
}

fn nonempty_roots(store: &dyn AssocStore, scope: &Scope, label: &str) -> Result<BTreeSet<CxId>> {
    let roots = root_nodes_for_scope(scope, store)?;
    if roots.is_empty() {
        return invalid_params(format!(
            "domain scope {label} has no source-of-truth root nodes"
        ));
    }
    Ok(roots)
}

fn bridge_members_by_frequency(
    graph: &AssocGraph,
    left: &[CxId],
    right: &[CxId],
) -> Result<Vec<CxId>> {
    let right_members: BTreeSet<_> = right.iter().copied().collect();
    let mut weighted = left
        .iter()
        .copied()
        .filter(|id| right_members.contains(id))
        .map(|id| Ok((id, graph.node_weight(id)?)))
        .collect::<Result<Vec<_>>>()?;
    weighted.sort_by(|(left_id, left_weight), (right_id, right_weight)| {
        right_weight
            .total_cmp(left_weight)
            .then_with(|| left_id.cmp(right_id))
    });
    Ok(weighted.into_iter().map(|(id, _)| id).collect())
}

fn distance_from_roots(
    graph: &AssocGraph,
    roots: &BTreeSet<CxId>,
    max_hops: usize,
) -> BTreeMap<CxId, usize> {
    let mut distances = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    for root in roots {
        if let Some(index) = graph.node_index(*root)
            && seen.insert(index)
        {
            distances.insert(*root, 0);
            queue.push_back((index, 0_usize));
        }
    }
    while let Some((current, hops)) = queue.pop_front() {
        if hops == max_hops {
            continue;
        }
        for edge in graph.out_edges_by_index(current) {
            if seen.insert(edge.dst) {
                let id = graph.node_id(edge.dst).expect("distance node id");
                distances.insert(id, hops + 1);
                queue.push_back((edge.dst, hops + 1));
            }
        }
    }
    distances
}

fn gate_confidence(left_groundedness: f32, right_groundedness: f32, distance: usize) -> f32 {
    let distance_score = 1.0_f32 / (1 + distance) as f32;
    left_groundedness
        .min(right_groundedness)
        .min(distance_score)
}

fn degree_for(cx_id: CxId, counts: &BTreeMap<CxId, usize>) -> Result<usize> {
    counts
        .get(&cx_id)
        .copied()
        .ok_or_else(|| LodestarError::KernelInvalidParams {
            detail: format!("bridge candidate {cx_id} is missing a precomputed degree count"),
        })
}

fn degree_score(
    cx_id: CxId,
    degree_counts: &BTreeMap<CxId, usize>,
    max_degree: usize,
) -> Result<f32> {
    let degree = degree_for(cx_id, degree_counts)?;
    Ok(degree as f32 / max_degree.max(1) as f32)
}

fn bridge_text(cx_id: CxId, metadata: &BTreeMap<String, String>) -> Result<String> {
    for key in ["term", "title", "question", "text"] {
        if let Some(value) = metadata.get(key)
            && !value.trim().is_empty()
        {
            return Ok(value.clone());
        }
    }
    let mut parts = Vec::new();
    for key in [
        "source_dataset",
        "source_id",
        "doc_id",
        "chunk_id",
        "database_name",
    ] {
        if let Some(value) = metadata.get(key)
            && !value.trim().is_empty()
        {
            parts.push(format!("{key}={value}"));
        }
    }
    if parts.is_empty() {
        for (key, value) in metadata.iter().take(4) {
            if !value.trim().is_empty() {
                parts.push(format!("{key}={value}"));
            }
        }
    }
    if parts.is_empty() {
        return invalid_params(format!(
            "bridge candidate {cx_id} has no non-empty identifying metadata"
        ));
    }
    Ok(parts.join("; "))
}

fn bridge_provenance(cx_id: CxId, metadata: &BTreeMap<String, String>) -> Vec<String> {
    let mut provenance = vec![format!("cx_id={cx_id}")];
    provenance.extend(
        metadata
            .iter()
            .take(8)
            .map(|(key, value)| format!("metadata:{key}={value}")),
    );
    provenance
}

#[derive(Clone, Debug, Default)]
struct PairAccumulator {
    refused_count: usize,
    candidates: Vec<DomainBridgeCandidate>,
}
