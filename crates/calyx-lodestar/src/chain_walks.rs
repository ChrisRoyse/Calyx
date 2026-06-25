use std::collections::BTreeSet;

use calyx_core::CxId;
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};

use crate::{
    DiscoveryCandidateLog, DiscoveryChainLog, DiscoveryChainParams, LodestarError, Result,
    run_grounded_discovery_chain,
};

pub const CHAIN_WALK_SCHEMA_VERSION: u32 = 1;
const SCORE_PATH_WEIGHT: f32 = 0.45;
const SCORE_GROUNDING_WEIGHT: f32 = 0.35;
const SCORE_NOVELTY_WEIGHT: f32 = 0.20;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainWalkSeedKind {
    StaticCandidate,
    OperatorQuestion,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChainWalkSeed {
    pub seed_id: String,
    pub kind: ChainWalkSeedKind,
    pub start: CxId,
    pub question: Option<String>,
    pub rationale: String,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChainWalkParams {
    pub chain: DiscoveryChainParams,
    pub max_hypotheses_per_seed: usize,
    pub min_terminal_confidence: f32,
}

impl Default for ChainWalkParams {
    fn default() -> Self {
        Self {
            chain: DiscoveryChainParams::default(),
            max_hypotheses_per_seed: 8,
            min_terminal_confidence: 0.25,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AbcHypothesis {
    pub seed_id: String,
    pub seed_kind: ChainWalkSeedKind,
    pub a: CxId,
    pub b: CxId,
    pub c: CxId,
    pub terminal_path: Vec<CxId>,
    pub cross_domain_distance: usize,
    pub terminal_confidence: f32,
    pub novelty_score: f32,
    pub path_score: f32,
    pub rank_score: f32,
    pub testable_claim: String,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChainWalkResult {
    pub seed: ChainWalkSeed,
    pub log: DiscoveryChainLog,
    pub hypotheses: Vec<AbcHypothesis>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChainWalkReport {
    pub schema_version: u32,
    pub seed_count: usize,
    pub completed_chain_count: usize,
    pub hypothesis_count: usize,
    pub results: Vec<ChainWalkResult>,
}

pub fn run_grounded_chain_walks(
    graph: &AssocGraph,
    seeds: &[ChainWalkSeed],
    anchors: &[CxId],
    params: &ChainWalkParams,
) -> Result<ChainWalkReport> {
    validate_inputs(graph, seeds, params)?;
    let mut results = Vec::with_capacity(seeds.len());
    for seed in seeds {
        graph.require_node_index(seed.start)?;
        let log = run_grounded_discovery_chain(graph, &[seed.start], anchors, &params.chain)?;
        let hypotheses = hypotheses_from_log(graph, seed, &log, params)?;
        results.push(ChainWalkResult {
            seed: seed.clone(),
            log,
            hypotheses,
        });
    }
    let completed_chain_count = results
        .iter()
        .filter(|result| !result.hypotheses.is_empty())
        .count();
    let hypothesis_count = results
        .iter()
        .map(|result| result.hypotheses.len())
        .sum::<usize>();
    Ok(ChainWalkReport {
        schema_version: CHAIN_WALK_SCHEMA_VERSION,
        seed_count: seeds.len(),
        completed_chain_count,
        hypothesis_count,
        results,
    })
}

fn hypotheses_from_log(
    graph: &AssocGraph,
    seed: &ChainWalkSeed,
    log: &DiscoveryChainLog,
    params: &ChainWalkParams,
) -> Result<Vec<AbcHypothesis>> {
    let mut hypotheses = Vec::new();
    for hop in &log.accepted_hops {
        if hop.path.len() < 3 || hop.gate_confidence < params.min_terminal_confidence {
            continue;
        }
        let a = hop.path[0];
        let b = hop.path[hop.path.len() - 2];
        let c = hop.path[hop.path.len() - 1];
        let novelty_score = novelty_score(graph, c)?;
        let rank_score = hop.candidate_score * SCORE_PATH_WEIGHT
            + hop.gate_confidence * SCORE_GROUNDING_WEIGHT
            + novelty_score * SCORE_NOVELTY_WEIGHT;
        hypotheses.push(AbcHypothesis {
            seed_id: seed.seed_id.clone(),
            seed_kind: seed.kind,
            a,
            b,
            c,
            terminal_path: hop.path.clone(),
            cross_domain_distance: hop.path.len() - 1,
            terminal_confidence: hop.gate_confidence,
            novelty_score,
            path_score: hop.candidate_score,
            rank_score,
            testable_claim: format!("{}: {} -- {} -- {}", seed.seed_id, a, b, c),
            provenance: hypothesis_provenance(seed, log, hop.hop, b, c),
        });
    }
    sort_hypotheses(&mut hypotheses);
    hypotheses.truncate(params.max_hypotheses_per_seed);
    Ok(hypotheses)
}

fn hypothesis_provenance(
    seed: &ChainWalkSeed,
    log: &DiscoveryChainLog,
    hop: usize,
    b: CxId,
    c: CxId,
) -> Vec<String> {
    let mut provenance = vec![
        format!("seed_id={}", seed.seed_id),
        format!("seed_kind={:?}", seed.kind),
    ];
    provenance.extend(seed.provenance.iter().cloned());
    for row in selected_rows(log).filter(|row| row.candidate.hop <= hop) {
        provenance.extend(row.candidate.provenance.iter().cloned());
        provenance.extend(row.gate.evidence.iter().cloned());
    }
    provenance.push(format!("terminal_b={b}"));
    provenance.push(format!("terminal_c={c}"));
    provenance
}

fn selected_rows(log: &DiscoveryChainLog) -> impl Iterator<Item = &DiscoveryCandidateLog> {
    log.candidates.iter().filter(|row| row.selected)
}

fn sort_hypotheses(hypotheses: &mut [AbcHypothesis]) {
    hypotheses.sort_by(|left, right| {
        right
            .rank_score
            .total_cmp(&left.rank_score)
            .then_with(|| {
                right
                    .terminal_confidence
                    .total_cmp(&left.terminal_confidence)
            })
            .then_with(|| right.cross_domain_distance.cmp(&left.cross_domain_distance))
            .then_with(|| left.c.as_bytes().cmp(right.c.as_bytes()))
    });
}

fn validate_inputs(
    graph: &AssocGraph,
    seeds: &[ChainWalkSeed],
    params: &ChainWalkParams,
) -> Result<()> {
    if graph.is_empty() {
        return Err(LodestarError::KernelEmptyGraph);
    }
    if seeds.is_empty() {
        return invalid_params("at least one chain-walk seed is required");
    }
    if params.max_hypotheses_per_seed == 0 {
        return invalid_params("max_hypotheses_per_seed must be greater than zero");
    }
    if !params.min_terminal_confidence.is_finite()
        || !(0.0..=1.0).contains(&params.min_terminal_confidence)
    {
        return invalid_params("min_terminal_confidence must be finite and in [0,1]");
    }
    let mut seen = BTreeSet::new();
    for seed in seeds {
        if seed.seed_id.trim().is_empty() {
            return invalid_params("seed_id must not be empty");
        }
        if !seen.insert(seed.seed_id.as_str()) {
            return invalid_params("seed_id values must be unique");
        }
        if matches!(seed.kind, ChainWalkSeedKind::OperatorQuestion)
            && seed.question.as_deref().unwrap_or("").trim().is_empty()
        {
            return invalid_params("operator-question seeds require question text");
        }
    }
    Ok(())
}

fn novelty_score(graph: &AssocGraph, id: CxId) -> Result<f32> {
    let frequency = graph.node_weight(id)?;
    Ok((1.0 / (1.0 + frequency)).clamp(0.0, 1.0))
}

fn invalid_params<T>(detail: impl Into<String>) -> Result<T> {
    Err(LodestarError::KernelInvalidParams {
        detail: detail.into(),
    })
}
