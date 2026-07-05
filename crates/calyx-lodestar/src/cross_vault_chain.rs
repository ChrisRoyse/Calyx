use std::collections::{BTreeMap, BTreeSet};

use calyx_core::CxId;
use calyx_paths::{AssocGraph, Edge, attenuate};
use serde::{Deserialize, Serialize};

use crate::Result;

mod validation;

pub const CROSS_VAULT_CHAIN_SCHEMA_VERSION: u32 = 1;
const SCORE_PATH_WEIGHT: f32 = 0.45;
const SCORE_TERMINAL_GATE_WEIGHT: f32 = 0.35;
const SCORE_BRIDGE_WEIGHT: f32 = 0.20;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MolecularKernelState {
    Grounded,
    Missing,
    Ungrounded,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossVaultChainParams {
    pub max_molecular_hops: usize,
    pub max_candidates: usize,
    pub min_endpoint_bits: f32,
    pub min_bridge_confidence: f32,
    pub min_molecular_gate_confidence: f32,
}

impl Default for CrossVaultChainParams {
    fn default() -> Self {
        Self {
            max_molecular_hops: 2,
            max_candidates: 32,
            min_endpoint_bits: 0.05,
            min_bridge_confidence: 0.25,
            min_molecular_gate_confidence: 0.25,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClinicalFrontier {
    pub seed_id: String,
    pub clinical_vault_id: String,
    pub clinical_cx_id: CxId,
    pub normalized_entity_id: String,
    pub grounded_confidence: f32,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MolecularEndpoint {
    pub molecular_vault_id: String,
    pub molecular_cx_id: CxId,
    pub normalized_entity_id: String,
    pub evidence_id: String,
    pub grounded_bits: f32,
    pub grounded_confidence: f32,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossVaultMolecularCandidate {
    pub seed_id: String,
    pub normalized_entity_id: String,
    pub evidence_id: String,
    pub from: CxId,
    pub to: CxId,
    pub hop: usize,
    pub edge_weight: f32,
    pub raw_path_score: f32,
    pub attenuated_path_score: f32,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossVaultMolecularGateVerdict {
    pub passed: bool,
    pub confidence: f32,
    pub code: String,
    pub reason: String,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossVaultChainCandidate {
    pub seed_id: String,
    pub clinical_vault_id: String,
    pub molecular_vault_id: String,
    pub clinical_cx_id: CxId,
    pub molecular_entry_cx_id: CxId,
    pub terminal_molecular_cx_id: CxId,
    pub normalized_entity_id: String,
    pub molecular_evidence_id: String,
    pub molecular_path: Vec<CxId>,
    pub molecular_hop_count: usize,
    pub path_score: f32,
    pub bridge_confidence: f32,
    pub terminal_confidence: f32,
    pub gate_code: String,
    pub rank_score: f32,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossVaultDeficit {
    pub seed_id: String,
    pub normalized_entity_id: String,
    pub molecular_cx_id: Option<CxId>,
    pub code: String,
    pub reason: String,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossVaultChainReport {
    pub schema_version: u32,
    pub clinical_seed_count: usize,
    pub molecular_endpoint_count: usize,
    pub candidate_count: usize,
    pub deficit_count: usize,
    pub candidates: Vec<CrossVaultChainCandidate>,
    pub deficits: Vec<CrossVaultDeficit>,
}

pub fn run_cross_vault_grounded_chain<G>(
    clinical_frontiers: &[ClinicalFrontier],
    molecular_graph: &AssocGraph,
    molecular_kernel_state: MolecularKernelState,
    endpoints: &[MolecularEndpoint],
    params: &CrossVaultChainParams,
    mut molecular_gate: G,
) -> Result<CrossVaultChainReport>
where
    G: FnMut(&CrossVaultMolecularCandidate) -> CrossVaultMolecularGateVerdict,
{
    validation::validate_kernel_state(molecular_kernel_state)?;
    validation::validate_inputs(clinical_frontiers, molecular_graph, endpoints, params)?;
    let endpoints_by_entity = endpoints_by_entity(endpoints);
    let mut candidates = Vec::new();
    let mut deficits = Vec::new();
    for seed in clinical_frontiers {
        let Some(matches) = endpoints_by_entity.get(seed.normalized_entity_id.as_str()) else {
            deficits.push(missing_endpoint_deficit(seed));
            continue;
        };
        for endpoint in matches {
            if !endpoint_clears_gate(seed, endpoint, params, &mut deficits) {
                continue;
            }
            let bridge_confidence = seed.grounded_confidence.min(endpoint.grounded_confidence);
            push_bridge_candidate(seed, endpoint, bridge_confidence, &mut candidates);
            traverse_molecular_graph(
                TraversalContext {
                    seed,
                    endpoint,
                    bridge_confidence,
                    graph: molecular_graph,
                    params,
                },
                &mut molecular_gate,
                &mut candidates,
                &mut deficits,
            )?;
        }
    }
    sort_candidates(&mut candidates);
    candidates.truncate(params.max_candidates);
    Ok(CrossVaultChainReport {
        schema_version: CROSS_VAULT_CHAIN_SCHEMA_VERSION,
        clinical_seed_count: clinical_frontiers.len(),
        molecular_endpoint_count: endpoints.len(),
        candidate_count: candidates.len(),
        deficit_count: deficits.len(),
        candidates,
        deficits,
    })
}

fn traverse_molecular_graph<G>(
    context: TraversalContext<'_>,
    molecular_gate: &mut G,
    candidates: &mut Vec<CrossVaultChainCandidate>,
    deficits: &mut Vec<CrossVaultDeficit>,
) -> Result<()>
where
    G: FnMut(&CrossVaultMolecularCandidate) -> CrossVaultMolecularGateVerdict,
{
    let TraversalContext {
        seed,
        endpoint,
        bridge_confidence,
        graph,
        params,
    } = context;
    let mut visited = BTreeSet::from([endpoint.molecular_cx_id]);
    let mut frontier = vec![TraversalNode {
        id: endpoint.molecular_cx_id,
        raw_path_score: bridge_confidence,
        path: vec![endpoint.molecular_cx_id],
    }];
    for hop in 1..=params.max_molecular_hops {
        let mut next_frontier = Vec::new();
        for node in &frontier {
            for edge in ranked_outgoing(graph, node.id)? {
                let to = graph.node_id(edge.dst).expect("edge destination id");
                if visited.contains(&to) {
                    continue;
                }
                let raw_path_score = node.raw_path_score * edge.weight;
                let mut path = node.path.clone();
                path.push(to);
                let candidate = molecular_candidate(
                    seed,
                    endpoint,
                    node.id,
                    to,
                    hop,
                    edge.weight,
                    raw_path_score,
                );
                let verdict = molecular_gate(&candidate);
                if verdict.passed && verdict.confidence >= params.min_molecular_gate_confidence {
                    visited.insert(to);
                    candidates.push(chain_candidate(ChainCandidateInput {
                        seed,
                        endpoint,
                        molecular_path: path.clone(),
                        molecular_hop_count: hop,
                        path_score: raw_path_score,
                        bridge_confidence,
                        terminal_confidence: verdict.confidence,
                        gate_code: &verdict.code,
                        gate_evidence: &verdict.evidence,
                    }));
                    next_frontier.push(TraversalNode {
                        id: to,
                        raw_path_score,
                        path,
                    });
                } else {
                    deficits.push(molecular_gate_deficit(seed, endpoint, to, &verdict));
                }
            }
        }
        if next_frontier.is_empty() {
            break;
        }
        frontier = next_frontier;
    }
    Ok(())
}

fn push_bridge_candidate(
    seed: &ClinicalFrontier,
    endpoint: &MolecularEndpoint,
    bridge_confidence: f32,
    candidates: &mut Vec<CrossVaultChainCandidate>,
) {
    let evidence = vec![
        format!("endpoint_bits={:.6}", endpoint.grounded_bits),
        format!("endpoint_confidence={:.6}", endpoint.grounded_confidence),
    ];
    candidates.push(chain_candidate(ChainCandidateInput {
        seed,
        endpoint,
        molecular_path: vec![endpoint.molecular_cx_id],
        molecular_hop_count: 0,
        path_score: bridge_confidence,
        bridge_confidence,
        terminal_confidence: bridge_confidence,
        gate_code: "CALYX_CROSS_VAULT_BRIDGE_PASS",
        gate_evidence: &evidence,
    }));
}

fn chain_candidate(input: ChainCandidateInput<'_>) -> CrossVaultChainCandidate {
    let ChainCandidateInput {
        seed,
        endpoint,
        molecular_path,
        molecular_hop_count,
        path_score,
        bridge_confidence,
        terminal_confidence,
        gate_code,
        gate_evidence,
    } = input;
    let terminal_molecular_cx_id = *molecular_path.last().expect("nonempty molecular path");
    let rank_score = path_score * SCORE_PATH_WEIGHT
        + terminal_confidence * SCORE_TERMINAL_GATE_WEIGHT
        + bridge_confidence * SCORE_BRIDGE_WEIGHT;
    CrossVaultChainCandidate {
        seed_id: seed.seed_id.clone(),
        clinical_vault_id: seed.clinical_vault_id.clone(),
        molecular_vault_id: endpoint.molecular_vault_id.clone(),
        clinical_cx_id: seed.clinical_cx_id,
        molecular_entry_cx_id: endpoint.molecular_cx_id,
        terminal_molecular_cx_id,
        normalized_entity_id: seed.normalized_entity_id.clone(),
        molecular_evidence_id: endpoint.evidence_id.clone(),
        molecular_path,
        molecular_hop_count,
        path_score,
        bridge_confidence,
        terminal_confidence,
        gate_code: gate_code.to_string(),
        rank_score,
        provenance: candidate_provenance(seed, endpoint, terminal_molecular_cx_id, gate_evidence),
    }
}

fn endpoint_clears_gate(
    seed: &ClinicalFrontier,
    endpoint: &MolecularEndpoint,
    params: &CrossVaultChainParams,
    deficits: &mut Vec<CrossVaultDeficit>,
) -> bool {
    if endpoint.grounded_bits < params.min_endpoint_bits
        || endpoint.grounded_confidence < params.min_bridge_confidence
    {
        deficits.push(CrossVaultDeficit {
            seed_id: seed.seed_id.clone(),
            normalized_entity_id: seed.normalized_entity_id.clone(),
            molecular_cx_id: Some(endpoint.molecular_cx_id),
            code: "CALYX_CROSS_VAULT_MOLECULAR_BITS_GATE_FAILED".to_string(),
            reason:
                "shared normalized entity endpoint did not clear molecular bits/confidence gate"
                    .to_string(),
            evidence: vec![
                format!("endpoint_bits={:.6}", endpoint.grounded_bits),
                format!("min_endpoint_bits={:.6}", params.min_endpoint_bits),
                format!("endpoint_confidence={:.6}", endpoint.grounded_confidence),
                format!("min_bridge_confidence={:.6}", params.min_bridge_confidence),
            ],
        });
        return false;
    }
    true
}

fn molecular_gate_deficit(
    seed: &ClinicalFrontier,
    endpoint: &MolecularEndpoint,
    to: CxId,
    verdict: &CrossVaultMolecularGateVerdict,
) -> CrossVaultDeficit {
    CrossVaultDeficit {
        seed_id: seed.seed_id.clone(),
        normalized_entity_id: seed.normalized_entity_id.clone(),
        molecular_cx_id: Some(to),
        code: verdict.code.clone(),
        reason: verdict.reason.clone(),
        evidence: {
            let mut evidence = endpoint.provenance.clone();
            evidence.extend(verdict.evidence.iter().cloned());
            evidence
        },
    }
}

fn missing_endpoint_deficit(seed: &ClinicalFrontier) -> CrossVaultDeficit {
    CrossVaultDeficit {
        seed_id: seed.seed_id.clone(),
        normalized_entity_id: seed.normalized_entity_id.clone(),
        molecular_cx_id: None,
        code: "CALYX_CROSS_VAULT_MOLECULAR_ENDPOINT_MISSING".to_string(),
        reason: "no molecular endpoint shares the clinical normalized entity id".to_string(),
        evidence: seed.provenance.clone(),
    }
}

fn molecular_candidate(
    seed: &ClinicalFrontier,
    endpoint: &MolecularEndpoint,
    from: CxId,
    to: CxId,
    hop: usize,
    edge_weight: f32,
    raw_path_score: f32,
) -> CrossVaultMolecularCandidate {
    CrossVaultMolecularCandidate {
        seed_id: seed.seed_id.clone(),
        normalized_entity_id: seed.normalized_entity_id.clone(),
        evidence_id: endpoint.evidence_id.clone(),
        from,
        to,
        hop,
        edge_weight,
        raw_path_score,
        attenuated_path_score: attenuate(raw_path_score, hop as u32),
        provenance: vec![
            format!("molecular_edge={from}->{to}"),
            format!("molecular_evidence_id={}", endpoint.evidence_id),
        ],
    }
}

fn candidate_provenance(
    seed: &ClinicalFrontier,
    endpoint: &MolecularEndpoint,
    terminal_molecular_cx_id: CxId,
    gate_evidence: &[String],
) -> Vec<String> {
    let mut provenance = vec![
        format!("clinical_vault_id={}", seed.clinical_vault_id),
        format!("molecular_vault_id={}", endpoint.molecular_vault_id),
        format!("normalized_entity_id={}", seed.normalized_entity_id),
        format!("molecular_evidence_id={}", endpoint.evidence_id),
        format!("molecular_entry_cx_id={}", endpoint.molecular_cx_id),
        format!("terminal_molecular_cx_id={terminal_molecular_cx_id}"),
    ];
    provenance.extend(seed.provenance.iter().cloned());
    provenance.extend(endpoint.provenance.iter().cloned());
    provenance.extend(gate_evidence.iter().cloned());
    provenance
}

fn endpoints_by_entity(endpoints: &[MolecularEndpoint]) -> BTreeMap<&str, Vec<&MolecularEndpoint>> {
    let mut by_entity = BTreeMap::<&str, Vec<&MolecularEndpoint>>::new();
    for endpoint in endpoints {
        by_entity
            .entry(endpoint.normalized_entity_id.as_str())
            .or_default()
            .push(endpoint);
    }
    by_entity
}

fn ranked_outgoing(graph: &AssocGraph, from: CxId) -> Result<Vec<Edge>> {
    let mut edges = graph.out_neighbors(from)?.to_vec();
    edges.sort_by(|left, right| {
        right
            .weight
            .total_cmp(&left.weight)
            .then_with(|| left.dst.cmp(&right.dst))
    });
    Ok(edges)
}

fn sort_candidates(candidates: &mut [CrossVaultChainCandidate]) {
    candidates.sort_by(|left, right| {
        right
            .rank_score
            .total_cmp(&left.rank_score)
            .then_with(|| {
                right
                    .terminal_confidence
                    .total_cmp(&left.terminal_confidence)
            })
            .then_with(|| right.path_score.total_cmp(&left.path_score))
            .then_with(|| {
                left.terminal_molecular_cx_id
                    .cmp(&right.terminal_molecular_cx_id)
            })
    });
}

#[derive(Clone, Debug)]
struct TraversalNode {
    id: CxId,
    raw_path_score: f32,
    path: Vec<CxId>,
}

struct TraversalContext<'a> {
    seed: &'a ClinicalFrontier,
    endpoint: &'a MolecularEndpoint,
    bridge_confidence: f32,
    graph: &'a AssocGraph,
    params: &'a CrossVaultChainParams,
}

struct ChainCandidateInput<'a> {
    seed: &'a ClinicalFrontier,
    endpoint: &'a MolecularEndpoint,
    molecular_path: Vec<CxId>,
    molecular_hop_count: usize,
    path_score: f32,
    bridge_confidence: f32,
    terminal_confidence: f32,
    gate_code: &'a str,
    gate_evidence: &'a [String],
}
