use std::str::FromStr;

use calyx_core::CxId;
use calyx_lodestar::{EvaluatorRun, HypothesisEvaluationInput, RetrievedEvidence};
use sha2::{Digest, Sha256};

use super::types::{
    AssociationHypothesis, CLINICAL_BOUNDARY, EvidenceRow, HypothesisFlag, SourceArtifact,
    bridge_error,
};
use crate::error::CliResult;

pub(super) fn evaluation_input(
    hypothesis: &AssociationHypothesis,
    flag: &HypothesisFlag,
    evidence_rows: &[&EvidenceRow],
    miner: &SourceArtifact,
    falsification: &SourceArtifact,
) -> CliResult<HypothesisEvaluationInput> {
    validate_hypothesis(hypothesis)?;
    let mut evidence = Vec::with_capacity(evidence_rows.len());
    for (index, row) in evidence_rows.iter().enumerate() {
        evidence.push(retrieved_evidence(hypothesis, row, index)?);
    }
    let evidence_ids = evidence
        .iter()
        .map(|row| row.evidence_id.clone())
        .collect::<Vec<_>>();
    let novelty = clamp_score(hypothesis.novelty_score, "novelty_score")?;
    let grounded = grounded_confidence(hypothesis, flag)?;
    let claim = format!(
        "Association research lead: {} ({}) -> {} ({}); status {}; support {}; counter {}. {}",
        hypothesis.source_name,
        hypothesis.source_type,
        hypothesis.target_name,
        hypothesis.target_type,
        flag.sweep_status,
        flag.support_evidence_count,
        flag.counter_evidence_count,
        CLINICAL_BOUNDARY
    );
    let provenance = bridge_provenance(hypothesis, flag, miner, falsification, evidence.len())?;
    let mut runs = deterministic_runs(flag, grounded, novelty, &evidence_ids);
    for run in &mut runs {
        run.justification.push_str(&format!(
            " Sources: miner={}, falsification={}.",
            miner.sha256, falsification.sha256
        ));
    }
    Ok(HypothesisEvaluationInput {
        hypothesis_id: hypothesis.hypothesis_id.clone(),
        a: cx_id_from_domain_id("source", &hypothesis.source_id)?,
        b: evidence[0].source_cx_id,
        c: cx_id_from_domain_id("target", &hypothesis.target_id)?,
        claim,
        grounded_confidence: grounded,
        chain_provenance: provenance,
        retrieved_evidence: evidence,
        evaluator_runs: runs,
    })
}

pub(super) fn rank_input(
    evaluation: &calyx_lodestar::HypothesisEvaluation,
) -> CliResult<calyx_lodestar::TraceableHypothesisInput> {
    let cross_domain_distance = provenance_usize(&evaluation.provenance, "cross_domain_distance")?;
    let sufficiency_proof = provenance_string(&evaluation.provenance, "sufficiency_proof")?;
    let evidence_ids = evaluation
        .cited_evidence
        .iter()
        .map(|row| row.evidence_id.clone())
        .collect::<Vec<_>>();
    if evidence_ids.is_empty() {
        return Err(bridge_error(format!(
            "evaluation {} has no cited evidence ids",
            evaluation.hypothesis_id
        )));
    }
    Ok(calyx_lodestar::TraceableHypothesisInput {
        hypothesis_id: evaluation.hypothesis_id.clone(),
        a: evaluation.a,
        b: evaluation.b,
        c: evaluation.c,
        claim: evaluation.claim.clone(),
        novelty_score: evaluation.novelty_mean,
        grounded_confidence: evaluation.grounded_confidence,
        cross_domain_distance,
        evaluator_plausibility_score: evaluation.plausible_mean,
        evaluator_aggregate_score: evaluation.aggregate_score,
        sufficiency_proof,
        provenance: evaluation.provenance.clone(),
        evidence_ids,
    })
}

fn retrieved_evidence(
    hypothesis: &AssociationHypothesis,
    row: &EvidenceRow,
    index: usize,
) -> CliResult<RetrievedEvidence> {
    if row.summary.trim().is_empty() {
        return Err(bridge_error(format!(
            "evidence row {} for {} has empty summary",
            row.source_row_index, hypothesis.hypothesis_id
        )));
    }
    Ok(RetrievedEvidence {
        evidence_id: format!(
            "{}::bridge-evidence::{:02}",
            hypothesis.hypothesis_id,
            index + 1
        ),
        source_cx_id: cx_id_from_evidence(row),
        title: format!(
            "{} {} {}",
            row.source_system, row.evidence_kind, row.reason_code
        ),
        abstract_text: row.summary.clone(),
        grounding_confidence: evidence_confidence(row.weight)?,
        provenance: vec![
            format!("source_system={}", row.source_system),
            format!("source_path={}", row.source_path),
            format!("source_sha256={}", row.source_sha256),
            format!("source_row_index={}", row.source_row_index),
            format!("reason_code={}", row.reason_code),
            "research_lead_only=true".to_string(),
            format!("clinical_boundary={CLINICAL_BOUNDARY}"),
        ],
    })
}

fn bridge_provenance(
    hypothesis: &AssociationHypothesis,
    flag: &HypothesisFlag,
    miner: &SourceArtifact,
    falsification: &SourceArtifact,
    evidence_count: usize,
) -> CliResult<Vec<String>> {
    let cross_domain_distance = hypothesis.path_count.max(1);
    Ok(vec![
        "bridge_schema_version=1".to_string(),
        "bridge_kind=bridge-falsification-evaluate".to_string(),
        format!("miner_report_sha256={}", miner.sha256),
        format!("falsification_report_sha256={}", falsification.sha256),
        format!("source_hypothesis_id={}", hypothesis.hypothesis_id),
        format!(
            "novelty_score={}",
            clamp_score(hypothesis.novelty_score, "novelty_score")?
        ),
        format!("cross_domain_distance={cross_domain_distance}"),
        format!(
            "sufficiency_proof=typed_support_count:{};support_evidence:{};counter_evidence:{};falsification_score:{:.6};boundary:research_lead_only",
            hypothesis.support_count,
            flag.support_evidence_count,
            flag.counter_evidence_count,
            flag.falsification_score
        ),
        format!("retrieved_evidence_count={evidence_count}"),
        "b_role=primary_falsification_evidence".to_string(),
        "research_lead_only=true".to_string(),
        format!("clinical_boundary={CLINICAL_BOUNDARY}"),
    ])
}

fn deterministic_runs(
    flag: &HypothesisFlag,
    grounded: f32,
    novelty: f32,
    evidence_ids: &[String],
) -> Vec<EvaluatorRun> {
    let total = (flag.support_evidence_count + flag.counter_evidence_count).max(1) as f32;
    let counter_ratio = flag.counter_evidence_count as f32 / total;
    let conservative_plausibility = (grounded * (1.0 - 0.35 * counter_ratio)).clamp(0.0, 1.0);
    vec![
        EvaluatorRun {
            prompt_id: "deterministic_support_balance_v1".to_string(),
            temperature_x100: 0,
            plausible_score: grounded,
            novelty_score: novelty,
            testability_score: 1.0,
            falsifiability_score: 1.0,
            justification: format!(
                "Deterministic bridge score from support_weight={}, counter_weight={}, falsification_score={}.",
                flag.support_weight, flag.counter_weight, flag.falsification_score
            ),
            falsification_test: "Check independent support/counter evidence rows and reproduce the falsification sweep before any claim escalation.".to_string(),
            cited_evidence_ids: evidence_ids.to_vec(),
        },
        EvaluatorRun {
            prompt_id: "deterministic_counter_pressure_v1".to_string(),
            temperature_x100: 100,
            plausible_score: conservative_plausibility,
            novelty_score: novelty,
            testability_score: 1.0,
            falsifiability_score: 1.0,
            justification: format!(
                "Conservative bridge score applies counter-evidence ratio {:.6}; research lead only.",
                counter_ratio
            ),
            falsification_test: "Escalate only after external outcome, safety, dose, and human-review gates; association evidence alone is insufficient.".to_string(),
            cited_evidence_ids: evidence_ids.to_vec(),
        },
    ]
}

fn validate_hypothesis(hypothesis: &AssociationHypothesis) -> CliResult {
    for (field, value) in [
        ("hypothesis_id", &hypothesis.hypothesis_id),
        ("source_id", &hypothesis.source_id),
        ("source_name", &hypothesis.source_name),
        ("target_id", &hypothesis.target_id),
        ("target_name", &hypothesis.target_name),
    ] {
        if value.trim().is_empty() {
            return Err(bridge_error(format!("{field} must not be empty")));
        }
    }
    clamp_score(hypothesis.score, "score")?;
    clamp_score(hypothesis.novelty_score, "novelty_score")?;
    Ok(())
}

fn grounded_confidence(
    hypothesis: &AssociationHypothesis,
    flag: &HypothesisFlag,
) -> CliResult<f32> {
    let support = clamp_score(hypothesis.score, "score")?;
    let counter = clamp_score(flag.falsification_score, "falsification_score")?;
    Ok((support * (1.0 - counter)).clamp(0.0, 1.0))
}

fn clamp_score(value: f64, field: &str) -> CliResult<f32> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(bridge_error(format!("{field} must be finite and in [0,1]")));
    }
    Ok(value as f32)
}

fn evidence_confidence(weight: f64) -> CliResult<f32> {
    if !weight.is_finite() || weight < 0.0 {
        return Err(bridge_error(
            "evidence weight must be finite and non-negative",
        ));
    }
    Ok((weight / (1.0 + weight)) as f32)
}

fn provenance_string(provenance: &[String], key: &str) -> CliResult<String> {
    provenance
        .iter()
        .find_map(|row| {
            row.strip_prefix(&format!("{key}="))
                .map(ToString::to_string)
        })
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| bridge_error(format!("missing provenance field {key}")))
}

fn provenance_usize(provenance: &[String], key: &str) -> CliResult<usize> {
    let raw = provenance_string(provenance, key)?;
    raw.parse::<usize>()
        .map_err(|error| bridge_error(format!("parse provenance field {key}: {error}")))
}

fn cx_id_from_domain_id(role: &str, value: &str) -> CliResult<CxId> {
    if value.trim().is_empty() {
        return Err(bridge_error(format!("{role} id must not be empty")));
    }
    CxId::from_str(value).or_else(|_| Ok(synthetic_cx_id(&[role, value])))
}

fn cx_id_from_evidence(row: &EvidenceRow) -> CxId {
    synthetic_cx_id(&[
        "evidence",
        &row.source_system,
        &row.source_sha256,
        &row.source_row_index.to_string(),
    ])
}

fn synthetic_cx_id(parts: &[&str]) -> CxId {
    let mut hasher = Sha256::new();
    hasher.update(b"calyx-discovery-bridge-cxid-v1\0");
    for part in parts {
        hasher.update(part.len().to_be_bytes());
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    CxId::from_bytes(bytes)
}
