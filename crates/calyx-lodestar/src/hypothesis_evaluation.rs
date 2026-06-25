use std::collections::BTreeSet;

use calyx_core::CxId;
use serde::{Deserialize, Serialize};

use crate::{LodestarError, Result};

pub const HYPOTHESIS_EVALUATION_SCHEMA_VERSION: u32 = 1;
const PLAUSIBILITY_WEIGHT: f32 = 0.35;
const NOVELTY_WEIGHT: f32 = 0.25;
const TESTABILITY_WEIGHT: f32 = 0.25;
const FALSIFIABILITY_WEIGHT: f32 = 0.15;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HypothesisEvaluationParams {
    pub min_runs_per_hypothesis: usize,
    pub min_prompt_variants: usize,
    pub min_temperature_variants: usize,
    pub min_retrieved_evidence: usize,
    pub retain_score_floor: f32,
    pub max_ranked: usize,
}

impl Default for HypothesisEvaluationParams {
    fn default() -> Self {
        Self {
            min_runs_per_hypothesis: 2,
            min_prompt_variants: 2,
            min_temperature_variants: 2,
            min_retrieved_evidence: 1,
            retain_score_floor: 0.65,
            max_ranked: 64,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetrievedEvidence {
    pub evidence_id: String,
    pub source_cx_id: CxId,
    pub title: String,
    pub abstract_text: String,
    pub grounding_confidence: f32,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvaluatorRun {
    pub prompt_id: String,
    pub temperature_x100: u16,
    pub plausible_score: f32,
    pub novelty_score: f32,
    pub testability_score: f32,
    pub falsifiability_score: f32,
    pub justification: String,
    pub falsification_test: String,
    pub cited_evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HypothesisEvaluationInput {
    pub hypothesis_id: String,
    pub a: CxId,
    pub b: CxId,
    pub c: CxId,
    pub claim: String,
    pub grounded_confidence: f32,
    pub chain_provenance: Vec<String>,
    pub retrieved_evidence: Vec<RetrievedEvidence>,
    pub evaluator_runs: Vec<EvaluatorRun>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisEvaluationVerdict {
    RetainForRanking,
    NeedsMoreEvidence,
    RejectLowScore,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HypothesisEvaluation {
    pub hypothesis_id: String,
    pub a: CxId,
    pub b: CxId,
    pub c: CxId,
    pub claim: String,
    pub run_count: usize,
    pub prompt_variant_count: usize,
    pub temperature_variant_count: usize,
    pub evidence_count: usize,
    pub plausible_mean: f32,
    pub novelty_mean: f32,
    pub testability_mean: f32,
    pub falsifiability_mean: f32,
    pub grounded_confidence: f32,
    pub aggregate_score: f32,
    pub verdict: HypothesisEvaluationVerdict,
    pub justifications: Vec<String>,
    pub falsification_tests: Vec<String>,
    pub cited_evidence: Vec<RetrievedEvidence>,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HypothesisEvaluationReport {
    pub schema_version: u32,
    pub input_count: usize,
    pub retained_count: usize,
    pub needs_more_evidence_count: usize,
    pub rejected_count: usize,
    pub evaluations: Vec<HypothesisEvaluation>,
}

pub fn aggregate_hypothesis_evaluations(
    inputs: &[HypothesisEvaluationInput],
    params: &HypothesisEvaluationParams,
) -> Result<HypothesisEvaluationReport> {
    validate_params(params)?;
    if inputs.is_empty() {
        return invalid_params("at least one hypothesis evaluation input is required");
    }
    let mut evaluations = Vec::with_capacity(inputs.len());
    for input in inputs {
        evaluations.push(evaluate_one(input, params)?);
    }
    sort_evaluations(&mut evaluations);
    evaluations.truncate(params.max_ranked);
    let retained_count = evaluations
        .iter()
        .filter(|row| row.verdict == HypothesisEvaluationVerdict::RetainForRanking)
        .count();
    let needs_more_evidence_count = evaluations
        .iter()
        .filter(|row| row.verdict == HypothesisEvaluationVerdict::NeedsMoreEvidence)
        .count();
    let rejected_count = evaluations
        .iter()
        .filter(|row| row.verdict == HypothesisEvaluationVerdict::RejectLowScore)
        .count();
    Ok(HypothesisEvaluationReport {
        schema_version: HYPOTHESIS_EVALUATION_SCHEMA_VERSION,
        input_count: inputs.len(),
        retained_count,
        needs_more_evidence_count,
        rejected_count,
        evaluations,
    })
}

fn evaluate_one(
    input: &HypothesisEvaluationInput,
    params: &HypothesisEvaluationParams,
) -> Result<HypothesisEvaluation> {
    validate_input(input, params)?;
    let cited_evidence = cited_evidence(input)?;
    let plausible_mean = mean(input.evaluator_runs.iter().map(|run| run.plausible_score));
    let novelty_mean = mean(input.evaluator_runs.iter().map(|run| run.novelty_score));
    let testability_mean = mean(input.evaluator_runs.iter().map(|run| run.testability_score));
    let falsifiability_mean = mean(
        input
            .evaluator_runs
            .iter()
            .map(|run| run.falsifiability_score),
    );
    let aggregate_score = plausible_mean * PLAUSIBILITY_WEIGHT
        + novelty_mean * NOVELTY_WEIGHT
        + testability_mean * TESTABILITY_WEIGHT
        + falsifiability_mean * FALSIFIABILITY_WEIGHT;
    let verdict = if input.retrieved_evidence.len() < params.min_retrieved_evidence {
        HypothesisEvaluationVerdict::NeedsMoreEvidence
    } else if aggregate_score >= params.retain_score_floor {
        HypothesisEvaluationVerdict::RetainForRanking
    } else {
        HypothesisEvaluationVerdict::RejectLowScore
    };
    Ok(HypothesisEvaluation {
        hypothesis_id: input.hypothesis_id.clone(),
        a: input.a,
        b: input.b,
        c: input.c,
        claim: input.claim.clone(),
        run_count: input.evaluator_runs.len(),
        prompt_variant_count: prompt_variant_count(&input.evaluator_runs),
        temperature_variant_count: temperature_variant_count(&input.evaluator_runs),
        evidence_count: input.retrieved_evidence.len(),
        plausible_mean,
        novelty_mean,
        testability_mean,
        falsifiability_mean,
        grounded_confidence: input.grounded_confidence,
        aggregate_score,
        verdict,
        justifications: input
            .evaluator_runs
            .iter()
            .map(|run| run.justification.clone())
            .collect(),
        falsification_tests: input
            .evaluator_runs
            .iter()
            .map(|run| run.falsification_test.clone())
            .collect(),
        cited_evidence,
        provenance: input.chain_provenance.clone(),
    })
}

fn validate_input(
    input: &HypothesisEvaluationInput,
    params: &HypothesisEvaluationParams,
) -> Result<()> {
    if input.hypothesis_id.trim().is_empty() || input.claim.trim().is_empty() {
        return invalid_params("hypothesis_id and claim must not be empty");
    }
    if !score_is_valid(input.grounded_confidence) {
        return invalid_params("grounded_confidence must be finite and in [0,1]");
    }
    if input.evaluator_runs.len() < params.min_runs_per_hypothesis {
        return invalid_params("not enough evaluator runs for hypothesis");
    }
    if prompt_variant_count(&input.evaluator_runs) < params.min_prompt_variants {
        return invalid_params("not enough prompt variants for hypothesis");
    }
    if temperature_variant_count(&input.evaluator_runs) < params.min_temperature_variants {
        return invalid_params("not enough temperature variants for hypothesis");
    }
    validate_evidence(&input.retrieved_evidence)?;
    validate_runs(&input.evaluator_runs)?;
    validate_citations(input)?;
    Ok(())
}

fn validate_evidence(evidence: &[RetrievedEvidence]) -> Result<()> {
    let mut ids = BTreeSet::new();
    for row in evidence {
        if row.evidence_id.trim().is_empty()
            || row.title.trim().is_empty()
            || row.abstract_text.trim().is_empty()
        {
            return invalid_params("retrieved evidence id/title/abstract must not be empty");
        }
        if !ids.insert(row.evidence_id.as_str()) {
            return invalid_params("retrieved evidence ids must be unique");
        }
        if !score_is_valid(row.grounding_confidence) {
            return invalid_params("evidence grounding_confidence must be finite and in [0,1]");
        }
    }
    Ok(())
}

fn validate_runs(runs: &[EvaluatorRun]) -> Result<()> {
    for run in runs {
        if run.prompt_id.trim().is_empty()
            || run.justification.trim().is_empty()
            || run.falsification_test.trim().is_empty()
        {
            return invalid_params(
                "evaluator runs require prompt, justification, and falsification",
            );
        }
        if run.cited_evidence_ids.is_empty() {
            return invalid_params("evaluator runs must cite retrieved evidence");
        }
        for score in [
            run.plausible_score,
            run.novelty_score,
            run.testability_score,
            run.falsifiability_score,
        ] {
            if !score_is_valid(score) {
                return invalid_params("evaluator scores must be finite and in [0,1]");
            }
        }
    }
    Ok(())
}

fn validate_citations(input: &HypothesisEvaluationInput) -> Result<()> {
    let evidence_ids = input
        .retrieved_evidence
        .iter()
        .map(|row| row.evidence_id.as_str())
        .collect::<BTreeSet<_>>();
    for run in &input.evaluator_runs {
        for cited in &run.cited_evidence_ids {
            if !evidence_ids.contains(cited.as_str()) {
                return invalid_params("evaluator run cites evidence not present in retrieval set");
            }
        }
    }
    Ok(())
}

fn cited_evidence(input: &HypothesisEvaluationInput) -> Result<Vec<RetrievedEvidence>> {
    let cited_ids = input
        .evaluator_runs
        .iter()
        .flat_map(|run| run.cited_evidence_ids.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    Ok(input
        .retrieved_evidence
        .iter()
        .filter(|row| cited_ids.contains(row.evidence_id.as_str()))
        .cloned()
        .collect())
}

fn prompt_variant_count(runs: &[EvaluatorRun]) -> usize {
    runs.iter()
        .map(|run| run.prompt_id.as_str())
        .collect::<BTreeSet<_>>()
        .len()
}

fn temperature_variant_count(runs: &[EvaluatorRun]) -> usize {
    runs.iter()
        .map(|run| run.temperature_x100)
        .collect::<BTreeSet<_>>()
        .len()
}

fn mean(values: impl Iterator<Item = f32>) -> f32 {
    let mut sum = 0.0_f32;
    let mut count = 0_usize;
    for value in values {
        sum += value;
        count += 1;
    }
    sum / count.max(1) as f32
}

fn sort_evaluations(evaluations: &mut [HypothesisEvaluation]) {
    evaluations.sort_by(|left, right| {
        right
            .aggregate_score
            .total_cmp(&left.aggregate_score)
            .then_with(|| {
                right
                    .grounded_confidence
                    .total_cmp(&left.grounded_confidence)
            })
            .then_with(|| left.hypothesis_id.cmp(&right.hypothesis_id))
    });
}

fn validate_params(params: &HypothesisEvaluationParams) -> Result<()> {
    if params.min_runs_per_hypothesis == 0
        || params.min_prompt_variants == 0
        || params.min_temperature_variants == 0
        || params.max_ranked == 0
    {
        return invalid_params("run, variant, and rank limits must be greater than zero");
    }
    if !score_is_valid(params.retain_score_floor) {
        return invalid_params("retain_score_floor must be finite and in [0,1]");
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
