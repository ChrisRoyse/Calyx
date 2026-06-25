use std::collections::BTreeMap;

use calyx_core::CxId;
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};

use crate::{LodestarError, Result};

pub const DOMAIN_BRIDGE_SCHEMA_VERSION: u32 = 1;
const FREQUENCY_WEIGHT: f32 = 0.35;
const DEGREE_WEIGHT: f32 = 0.30;
const CENTRALITY_WEIGHT: f32 = 0.20;
const GROUNDING_WEIGHT: f32 = 0.15;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct DomainPair {
    pub left: String,
    pub right: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainBridgeParams {
    pub min_gate_confidence: f32,
    pub max_per_pair: usize,
}

impl Default for DomainBridgeParams {
    fn default() -> Self {
        Self {
            min_gate_confidence: 0.25,
            max_per_pair: 32,
        }
    }
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
    let max_degree = max_degree(graph)?;
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
    max_degree: usize,
) -> Result<DomainBridgeCandidate> {
    let frequency_weight = graph.node_weight(input.cx_id)?;
    let degree = graph.in_degree(input.cx_id)? + graph.out_degree(input.cx_id)?;
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

fn max_degree(graph: &AssocGraph) -> Result<usize> {
    let mut max_value = 1_usize;
    for id in graph.node_ids() {
        max_value = max_value.max(graph.in_degree(id)? + graph.out_degree(id)?);
    }
    Ok(max_value)
}

fn validate_params(params: &DomainBridgeParams) -> Result<()> {
    if !params.min_gate_confidence.is_finite() || !(0.0..=1.0).contains(&params.min_gate_confidence)
    {
        return invalid_params("min_gate_confidence must be finite and in [0,1]");
    }
    if params.max_per_pair == 0 {
        return invalid_params("max_per_pair must be greater than zero");
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

#[derive(Clone, Debug, Default)]
struct PairAccumulator {
    refused_count: usize,
    candidates: Vec<DomainBridgeCandidate>,
}
