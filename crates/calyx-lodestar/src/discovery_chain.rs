use std::collections::BTreeSet;

use calyx_core::CxId;
use calyx_paths::{AssocGraph, Edge, attenuate};
use serde::{Deserialize, Serialize};

use crate::{LodestarError, Result, groundedness_distance};

pub const DISCOVERY_CHAIN_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryChainParams {
    pub max_hops: usize,
    pub branch_width: usize,
    pub probe_width: usize,
    pub max_groundedness_distance: usize,
    pub min_gate_confidence: f32,
    pub novelty_weight: f32,
}

impl Default for DiscoveryChainParams {
    fn default() -> Self {
        Self {
            max_hops: 100,
            branch_width: 3,
            probe_width: 16,
            max_groundedness_distance: 3,
            min_gate_confidence: 0.25,
            novelty_weight: 0.35,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryCandidate {
    pub hop: usize,
    pub branch_id: usize,
    pub from: CxId,
    pub to: CxId,
    pub edge_weight: f32,
    pub raw_path_score: f32,
    pub attenuated_path_score: f32,
    pub novelty_score: f32,
    pub candidate_score: f32,
    pub groundedness_distance: Option<usize>,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryGateVerdict {
    pub passed: bool,
    pub confidence: f32,
    pub code: String,
    pub reason: String,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryCandidateLog {
    pub candidate: DiscoveryCandidate,
    pub gate: DiscoveryGateVerdict,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryAcceptedHop {
    pub hop: usize,
    pub branch_id: usize,
    pub from: CxId,
    pub to: CxId,
    pub candidate_score: f32,
    pub gate_confidence: f32,
    pub path: Vec<CxId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryTermination {
    MaxHops,
    FrontierExhausted,
    NoGatePass,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryChainLog {
    pub schema_version: u32,
    pub starts: Vec<CxId>,
    pub anchors: Vec<CxId>,
    pub params: DiscoveryChainParams,
    pub candidates: Vec<DiscoveryCandidateLog>,
    pub accepted_hops: Vec<DiscoveryAcceptedHop>,
    pub gate_pass_count: usize,
    pub refused_count: usize,
    pub terminated: DiscoveryTermination,
}

pub fn run_grounded_discovery_chain(
    graph: &AssocGraph,
    starts: &[CxId],
    anchors: &[CxId],
    params: &DiscoveryChainParams,
) -> Result<DiscoveryChainLog> {
    run_discovery_chain_with_gate(graph, starts, anchors, params, |candidate| {
        grounded_gate(candidate, params)
    })
}

pub fn run_discovery_chain_with_gate<G>(
    graph: &AssocGraph,
    starts: &[CxId],
    anchors: &[CxId],
    params: &DiscoveryChainParams,
    mut gate: G,
) -> Result<DiscoveryChainLog>
where
    G: FnMut(&DiscoveryCandidate) -> DiscoveryGateVerdict,
{
    validate_inputs(graph, starts, anchors, params)?;

    let mut log = DiscoveryChainLog {
        schema_version: DISCOVERY_CHAIN_SCHEMA_VERSION,
        starts: starts.to_vec(),
        anchors: anchors.to_vec(),
        params: params.clone(),
        candidates: Vec::new(),
        accepted_hops: Vec::new(),
        gate_pass_count: 0,
        refused_count: 0,
        terminated: DiscoveryTermination::MaxHops,
    };
    let mut visited: BTreeSet<CxId> = starts.iter().copied().collect();
    let mut frontier: Vec<_> = starts
        .iter()
        .copied()
        .enumerate()
        .map(|(branch_id, id)| FrontierNode {
            id,
            branch_id,
            raw_path_score: 1.0,
            path: vec![id],
        })
        .collect();

    for hop in 1..=params.max_hops {
        let mut passed = Vec::new();
        let mut evaluated = 0_usize;
        for node in &frontier {
            for ranked in ranked_outgoing(graph, node.id, params)? {
                evaluated += 1;
                let raw_path_score = node.raw_path_score * ranked.edge.weight;
                let mut path = node.path.clone();
                path.push(ranked.to);
                let candidate = DiscoveryCandidate {
                    hop,
                    branch_id: node.branch_id,
                    from: node.id,
                    to: ranked.to,
                    edge_weight: ranked.edge.weight,
                    raw_path_score,
                    attenuated_path_score: attenuate(raw_path_score, hop as u32),
                    novelty_score: ranked.novelty_score,
                    candidate_score: ranked.candidate_score,
                    groundedness_distance: groundedness_distance(
                        graph,
                        ranked.to,
                        anchors,
                        params.max_groundedness_distance,
                    )?,
                    provenance: candidate_provenance(node.id, ranked.to, &path),
                };
                let verdict = if visited.contains(&ranked.to) {
                    visited_gate(&candidate)
                } else {
                    gate(&candidate)
                };
                let log_index = log.candidates.len();
                if verdict.passed {
                    log.gate_pass_count += 1;
                    passed.push(PassedCandidate {
                        log_index,
                        candidate: candidate.clone(),
                        gate: verdict.clone(),
                        path,
                    });
                } else {
                    log.refused_count += 1;
                }
                log.candidates.push(DiscoveryCandidateLog {
                    candidate,
                    gate: verdict,
                    selected: false,
                });
            }
        }
        if passed.is_empty() {
            log.terminated = if evaluated == 0 {
                DiscoveryTermination::FrontierExhausted
            } else {
                DiscoveryTermination::NoGatePass
            };
            return Ok(log);
        }

        sort_passed(&mut passed);
        let mut next_frontier = Vec::new();
        for pass in passed.into_iter().take(params.branch_width) {
            visited.insert(pass.candidate.to);
            log.candidates[pass.log_index].selected = true;
            log.accepted_hops.push(DiscoveryAcceptedHop {
                hop: pass.candidate.hop,
                branch_id: pass.candidate.branch_id,
                from: pass.candidate.from,
                to: pass.candidate.to,
                candidate_score: pass.candidate.candidate_score,
                gate_confidence: pass.gate.confidence,
                path: pass.path.clone(),
            });
            next_frontier.push(FrontierNode {
                id: pass.candidate.to,
                branch_id: pass.candidate.branch_id,
                raw_path_score: pass.candidate.raw_path_score,
                path: pass.path,
            });
        }
        frontier = next_frontier;
    }
    Ok(log)
}

fn grounded_gate(
    candidate: &DiscoveryCandidate,
    params: &DiscoveryChainParams,
) -> DiscoveryGateVerdict {
    match candidate.groundedness_distance {
        Some(distance) => {
            let confidence = grounded_confidence(distance, params.max_groundedness_distance);
            if confidence >= params.min_gate_confidence {
                DiscoveryGateVerdict {
                    passed: true,
                    confidence,
                    code: "CALYX_DISCOVERY_GATE_PASS".to_string(),
                    reason: "candidate reaches an anchored node within the groundedness radius"
                        .to_string(),
                    evidence: vec![format!("groundedness_distance={distance}")],
                }
            } else {
                DiscoveryGateVerdict {
                    passed: false,
                    confidence,
                    code: "CALYX_DISCOVERY_LOW_CONFIDENCE".to_string(),
                    reason: "candidate is grounded but below the configured confidence floor"
                        .to_string(),
                    evidence: vec![format!(
                        "confidence={confidence:.6} min={:.6}",
                        params.min_gate_confidence
                    )],
                }
            }
        }
        None => DiscoveryGateVerdict {
            passed: false,
            confidence: 0.0,
            code: "CALYX_DISCOVERY_UNGROUNDED".to_string(),
            reason: "candidate has no reachable anchor inside the groundedness radius".to_string(),
            evidence: vec!["groundedness_distance=null".to_string()],
        },
    }
}

fn visited_gate(candidate: &DiscoveryCandidate) -> DiscoveryGateVerdict {
    DiscoveryGateVerdict {
        passed: false,
        confidence: 0.0,
        code: "CALYX_DISCOVERY_VISITED_LOOP".to_string(),
        reason: "candidate would revisit an already accepted chain node".to_string(),
        evidence: vec![format!("to={}", candidate.to)],
    }
}

fn validate_inputs(
    graph: &AssocGraph,
    starts: &[CxId],
    anchors: &[CxId],
    params: &DiscoveryChainParams,
) -> Result<()> {
    if graph.is_empty() {
        return Err(LodestarError::KernelEmptyGraph);
    }
    if starts.is_empty() {
        return invalid_params("at least one start node is required");
    }
    if params.max_hops == 0 {
        return invalid_params("max_hops must be greater than zero");
    }
    if params.branch_width == 0 {
        return invalid_params("branch_width must be greater than zero");
    }
    if params.probe_width == 0 {
        return invalid_params("probe_width must be greater than zero");
    }
    if !params.min_gate_confidence.is_finite() || !(0.0..=1.0).contains(&params.min_gate_confidence)
    {
        return invalid_params("min_gate_confidence must be finite and in [0,1]");
    }
    if !params.novelty_weight.is_finite() || !(0.0..=1.0).contains(&params.novelty_weight) {
        return invalid_params("novelty_weight must be finite and in [0,1]");
    }
    for start in starts {
        graph.require_node_index(*start)?;
    }
    for anchor in anchors {
        graph.require_node_index(*anchor)?;
    }
    Ok(())
}

fn invalid_params<T>(detail: impl Into<String>) -> Result<T> {
    Err(LodestarError::KernelInvalidParams {
        detail: detail.into(),
    })
}

fn ranked_outgoing(
    graph: &AssocGraph,
    from: CxId,
    params: &DiscoveryChainParams,
) -> Result<Vec<RankedEdge>> {
    let mut ranked = Vec::new();
    for edge in graph.out_neighbors(from)? {
        let to = graph.node_id(edge.dst).expect("edge destination id");
        let novelty_score = novelty_score(graph, to)?;
        ranked.push(RankedEdge {
            edge: *edge,
            to,
            novelty_score,
            candidate_score: score_candidate(*edge, novelty_score, params.novelty_weight),
        });
    }
    ranked.sort_by(|left, right| {
        right
            .candidate_score
            .total_cmp(&left.candidate_score)
            .then_with(|| left.to.as_bytes().cmp(right.to.as_bytes()))
    });
    ranked.truncate(params.probe_width);
    Ok(ranked)
}

fn score_candidate(edge: Edge, novelty_score: f32, novelty_weight: f32) -> f32 {
    edge.weight * ((1.0 - novelty_weight) + novelty_weight * novelty_score)
}

fn novelty_score(graph: &AssocGraph, id: CxId) -> Result<f32> {
    let frequency = graph.node_weight(id)?;
    Ok((1.0 / (1.0 + frequency)).clamp(0.0, 1.0))
}

fn grounded_confidence(distance: usize, max_distance: usize) -> f32 {
    if distance == 0 {
        return 1.0;
    }
    if max_distance == 0 {
        return 0.0;
    }
    1.0 - (distance as f32 / (max_distance + 1) as f32)
}

fn candidate_provenance(from: CxId, to: CxId, path: &[CxId]) -> Vec<String> {
    vec![
        format!("assoc_edge={from}->{to}"),
        format!("path_len={}", path.len()),
    ]
}

fn sort_passed(passed: &mut [PassedCandidate]) {
    passed.sort_by(|left, right| {
        right
            .candidate
            .candidate_score
            .total_cmp(&left.candidate.candidate_score)
            .then_with(|| right.gate.confidence.total_cmp(&left.gate.confidence))
            .then_with(|| {
                left.candidate
                    .to
                    .as_bytes()
                    .cmp(right.candidate.to.as_bytes())
            })
    });
}

#[derive(Clone, Debug)]
struct FrontierNode {
    id: CxId,
    branch_id: usize,
    raw_path_score: f32,
    path: Vec<CxId>,
}

#[derive(Clone, Debug)]
struct RankedEdge {
    edge: Edge,
    to: CxId,
    novelty_score: f32,
    candidate_score: f32,
}

#[derive(Clone, Debug)]
struct PassedCandidate {
    log_index: usize,
    candidate: DiscoveryCandidate,
    gate: DiscoveryGateVerdict,
    path: Vec<CxId>,
}
