use std::collections::BTreeSet;

use calyx_core::CxId;
use serde::{Deserialize, Serialize};

use crate::{LodestarError, ProbeMatrixLog, ProbeRecord, Result};

pub const REFUSAL_EXPANSION_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RefusalExpansionParams {
    pub min_deficit_bits: f32,
    pub max_actions: usize,
}

impl Default for RefusalExpansionParams {
    fn default() -> Self {
        Self {
            min_deficit_bits: 0.0,
            max_actions: 16,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalExpansionActionKind {
    AddEvidence,
    AddLens,
    Reground,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RefusalExpansionAction {
    pub id: usize,
    pub variant_id: usize,
    pub frontier: String,
    pub code: String,
    pub reason: String,
    pub deficit_bits: f32,
    pub kind: RefusalExpansionActionKind,
    pub evidence_query: String,
    pub lens_hint: String,
    pub priority_score: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RefusalExpansionPlan {
    pub schema_version: u32,
    pub frontier: String,
    pub total_deficit_bits: f32,
    pub actions: Vec<RefusalExpansionAction>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RefusalExpansionVerification {
    pub schema_version: u32,
    pub frontier: String,
    pub before_refusal_count: usize,
    pub after_refusal_count: usize,
    pub closed_refusal_count: usize,
    pub before_grounded_count: usize,
    pub after_grounded_count: usize,
    pub new_grounded_hits: Vec<CxId>,
    pub closed: bool,
}

pub fn plan_refusal_expansion(
    log: &ProbeMatrixLog,
    params: &RefusalExpansionParams,
) -> Result<RefusalExpansionPlan> {
    validate_params(params)?;
    let mut actions = Vec::new();
    let mut total_deficit_bits = 0.0_f32;
    for record in &log.records {
        for refusal in &record.refusals {
            let deficit_bits = refusal.deficit_bits.unwrap_or(0.0);
            total_deficit_bits += deficit_bits;
            if deficit_bits < params.min_deficit_bits {
                continue;
            }
            actions.push(action_from_refusal(
                actions.len(),
                &log.spec.frontier,
                record,
                &refusal.code,
                &refusal.reason,
                deficit_bits,
            ));
        }
    }
    actions.sort_by(|left, right| {
        right
            .priority_score
            .total_cmp(&left.priority_score)
            .then_with(|| left.variant_id.cmp(&right.variant_id))
            .then_with(|| left.code.cmp(&right.code))
    });
    actions.truncate(params.max_actions);
    for (index, action) in actions.iter_mut().enumerate() {
        action.id = index;
    }
    Ok(RefusalExpansionPlan {
        schema_version: REFUSAL_EXPANSION_SCHEMA_VERSION,
        frontier: log.spec.frontier.clone(),
        total_deficit_bits,
        actions,
    })
}

pub fn verify_refusal_expansion(
    before: &ProbeMatrixLog,
    after: &ProbeMatrixLog,
) -> Result<RefusalExpansionVerification> {
    if before.spec.frontier != after.spec.frontier {
        return invalid_params("before and after logs must use the same frontier");
    }
    let before_refusal_count = refusal_count(before);
    let after_refusal_count = refusal_count(after);
    let before_grounded = grounded_hits(before);
    let after_grounded = grounded_hits(after);
    let new_grounded_hits: Vec<_> = after_grounded
        .difference(&before_grounded)
        .copied()
        .collect();
    let closed_refusal_count = before_refusal_count.saturating_sub(after_refusal_count);
    Ok(RefusalExpansionVerification {
        schema_version: REFUSAL_EXPANSION_SCHEMA_VERSION,
        frontier: before.spec.frontier.clone(),
        before_refusal_count,
        after_refusal_count,
        closed_refusal_count,
        before_grounded_count: before_grounded.len(),
        after_grounded_count: after_grounded.len(),
        new_grounded_hits,
        closed: closed_refusal_count > 0 && after_grounded.len() > before_grounded.len(),
    })
}

fn action_from_refusal(
    id: usize,
    frontier: &str,
    record: &ProbeRecord,
    code: &str,
    reason: &str,
    deficit_bits: f32,
) -> RefusalExpansionAction {
    let kind = classify_action(code, reason);
    RefusalExpansionAction {
        id,
        variant_id: record.variant.id,
        frontier: frontier.to_string(),
        code: code.to_string(),
        reason: reason.to_string(),
        deficit_bits,
        evidence_query: evidence_query(frontier, &kind, code),
        lens_hint: format!("{:?}", record.variant.lens_emphasis),
        priority_score: priority_score(deficit_bits, &kind),
        kind,
    }
}

fn classify_action(code: &str, reason: &str) -> RefusalExpansionActionKind {
    let lower = format!("{} {}", code.to_lowercase(), reason.to_lowercase());
    if lower.contains("lens") || lower.contains("sensor") {
        RefusalExpansionActionKind::AddLens
    } else if lower.contains("ground") || lower.contains("anchor") {
        RefusalExpansionActionKind::Reground
    } else {
        RefusalExpansionActionKind::AddEvidence
    }
}

fn evidence_query(frontier: &str, kind: &RefusalExpansionActionKind, code: &str) -> String {
    match kind {
        RefusalExpansionActionKind::AddEvidence => {
            format!("add corroborating evidence for {frontier} after {code}")
        }
        RefusalExpansionActionKind::AddLens => {
            format!("add or emphasize missing lens evidence for {frontier} after {code}")
        }
        RefusalExpansionActionKind::Reground => {
            format!("add anchor or outcome grounding for {frontier} after {code}")
        }
    }
}

fn priority_score(deficit_bits: f32, kind: &RefusalExpansionActionKind) -> f32 {
    let kind_bonus = match kind {
        RefusalExpansionActionKind::AddEvidence => 0.10,
        RefusalExpansionActionKind::AddLens => 0.20,
        RefusalExpansionActionKind::Reground => 0.15,
    };
    deficit_bits + kind_bonus
}

fn refusal_count(log: &ProbeMatrixLog) -> usize {
    log.records.iter().map(|record| record.refusals.len()).sum()
}

fn grounded_hits(log: &ProbeMatrixLog) -> BTreeSet<CxId> {
    log.records
        .iter()
        .flat_map(|record| record.hits.iter())
        .filter(|hit| hit.grounded)
        .map(|hit| hit.cx_id)
        .collect()
}

fn validate_params(params: &RefusalExpansionParams) -> Result<()> {
    if !params.min_deficit_bits.is_finite() || params.min_deficit_bits < 0.0 {
        return invalid_params("min_deficit_bits must be finite and non-negative");
    }
    if params.max_actions == 0 {
        return invalid_params("max_actions must be greater than zero");
    }
    Ok(())
}

fn invalid_params<T>(detail: impl Into<String>) -> Result<T> {
    Err(LodestarError::KernelInvalidParams {
        detail: detail.into(),
    })
}
