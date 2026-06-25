use calyx_core::CxId;
use serde::{Deserialize, Serialize};

use crate::{LodestarError, Result};

pub const RANKED_HYPOTHESIS_SCHEMA_VERSION: u32 = 1;
const NOVELTY_WEIGHT: f32 = 0.25;
const GROUNDING_WEIGHT: f32 = 0.30;
const DISTANCE_WEIGHT: f32 = 0.20;
const EVALUATOR_WEIGHT: f32 = 0.25;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RankedHypothesisParams {
    pub max_ranked: usize,
    pub review_top_n: usize,
    pub min_review_score: f32,
}

impl Default for RankedHypothesisParams {
    fn default() -> Self {
        Self {
            max_ranked: 64,
            review_top_n: 10,
            min_review_score: 0.65,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceableHypothesisInput {
    pub hypothesis_id: String,
    pub a: CxId,
    pub b: CxId,
    pub c: CxId,
    pub claim: String,
    pub novelty_score: f32,
    pub grounded_confidence: f32,
    pub cross_domain_distance: usize,
    pub evaluator_plausibility_score: f32,
    pub evaluator_aggregate_score: f32,
    pub sufficiency_proof: String,
    pub provenance: Vec<String>,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RankedHypothesis {
    pub rank: usize,
    pub hypothesis_id: String,
    pub a: CxId,
    pub b: CxId,
    pub c: CxId,
    pub claim: String,
    pub novelty_score: f32,
    pub grounded_confidence: f32,
    pub cross_domain_distance: usize,
    pub distance_score: f32,
    pub evaluator_plausibility_score: f32,
    pub evaluator_aggregate_score: f32,
    pub rank_score: f32,
    pub human_review_flag: bool,
    pub sufficiency_proof: String,
    pub provenance: Vec<String>,
    pub evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RankedHypothesisReport {
    pub schema_version: u32,
    pub input_count: usize,
    pub ranked_count: usize,
    pub human_review_count: usize,
    pub hypotheses: Vec<RankedHypothesis>,
}

pub fn rank_traceable_hypotheses(
    inputs: &[TraceableHypothesisInput],
    params: &RankedHypothesisParams,
) -> Result<RankedHypothesisReport> {
    validate_params(params)?;
    if inputs.is_empty() {
        return invalid_params("at least one hypothesis is required");
    }
    for input in inputs {
        validate_input(input)?;
    }
    let max_distance = inputs
        .iter()
        .map(|input| input.cross_domain_distance)
        .max()
        .unwrap_or(1)
        .max(1);
    let mut hypotheses = inputs
        .iter()
        .map(|input| ranked_from_input(input, max_distance))
        .collect::<Vec<_>>();
    sort_hypotheses(&mut hypotheses);
    hypotheses.truncate(params.max_ranked);
    for (index, hypothesis) in hypotheses.iter_mut().enumerate() {
        hypothesis.rank = index + 1;
        hypothesis.human_review_flag =
            index < params.review_top_n && hypothesis.rank_score >= params.min_review_score;
    }
    let human_review_count = hypotheses
        .iter()
        .filter(|hypothesis| hypothesis.human_review_flag)
        .count();
    Ok(RankedHypothesisReport {
        schema_version: RANKED_HYPOTHESIS_SCHEMA_VERSION,
        input_count: inputs.len(),
        ranked_count: hypotheses.len(),
        human_review_count,
        hypotheses,
    })
}

fn ranked_from_input(input: &TraceableHypothesisInput, max_distance: usize) -> RankedHypothesis {
    let distance_score = input.cross_domain_distance as f32 / max_distance as f32;
    let rank_score = input.novelty_score * NOVELTY_WEIGHT
        + input.grounded_confidence * GROUNDING_WEIGHT
        + distance_score * DISTANCE_WEIGHT
        + input.evaluator_plausibility_score * EVALUATOR_WEIGHT;
    RankedHypothesis {
        rank: 0,
        hypothesis_id: input.hypothesis_id.clone(),
        a: input.a,
        b: input.b,
        c: input.c,
        claim: input.claim.clone(),
        novelty_score: input.novelty_score,
        grounded_confidence: input.grounded_confidence,
        cross_domain_distance: input.cross_domain_distance,
        distance_score,
        evaluator_plausibility_score: input.evaluator_plausibility_score,
        evaluator_aggregate_score: input.evaluator_aggregate_score,
        rank_score,
        human_review_flag: false,
        sufficiency_proof: input.sufficiency_proof.clone(),
        provenance: input.provenance.clone(),
        evidence_ids: input.evidence_ids.clone(),
    }
}

fn sort_hypotheses(hypotheses: &mut [RankedHypothesis]) {
    hypotheses.sort_by(|left, right| {
        right
            .rank_score
            .total_cmp(&left.rank_score)
            .then_with(|| {
                right
                    .grounded_confidence
                    .total_cmp(&left.grounded_confidence)
            })
            .then_with(|| right.cross_domain_distance.cmp(&left.cross_domain_distance))
            .then_with(|| left.hypothesis_id.cmp(&right.hypothesis_id))
    });
}

fn validate_params(params: &RankedHypothesisParams) -> Result<()> {
    if params.max_ranked == 0 {
        return invalid_params("max_ranked must be greater than zero");
    }
    if !score_is_valid(params.min_review_score) {
        return invalid_params("min_review_score must be finite and in [0,1]");
    }
    Ok(())
}

fn validate_input(input: &TraceableHypothesisInput) -> Result<()> {
    if input.hypothesis_id.trim().is_empty()
        || input.claim.trim().is_empty()
        || input.sufficiency_proof.trim().is_empty()
    {
        return invalid_params("hypothesis id, claim, and sufficiency proof must not be empty");
    }
    if input.cross_domain_distance == 0 {
        return invalid_params("cross_domain_distance must be greater than zero");
    }
    for score in [
        input.novelty_score,
        input.grounded_confidence,
        input.evaluator_plausibility_score,
        input.evaluator_aggregate_score,
    ] {
        if !score_is_valid(score) {
            return invalid_params("rank scores must be finite and in [0,1]");
        }
    }
    if input.provenance.is_empty() || input.evidence_ids.is_empty() {
        return invalid_params("ranked hypotheses require provenance and evidence ids");
    }
    Ok(())
}

fn score_is_valid(score: f32) -> bool {
    score.is_finite() && (0.0..=1.0).contains(&score)
}

fn invalid_params<T>(detail: impl Into<String>) -> Result<T> {
    Err(LodestarError::KernelInvalidParams {
        detail: detail.into(),
    })
}
