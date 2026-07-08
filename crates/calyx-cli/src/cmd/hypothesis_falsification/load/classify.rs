use serde_json::Value;

use super::super::model::InputHypothesis;
use super::{
    array_len, f64_field, is_drug_target_hypothesis, is_target_disease_hypothesis, str_field,
    usize_field,
};
use crate::cmd::mechanistic_direction::{
    MechanisticDirectionEvidence, infer_observed_target_modulation,
    infer_required_target_modulation, modulation_compatible, modulation_name,
};

#[derive(Clone)]
pub(super) struct SourceClass {
    pub(super) kind: &'static str,
    pub(super) reason: &'static str,
    pub(super) weight: f64,
    pub(super) summary: String,
    pub(super) mechanistic_direction: Option<MechanisticDirectionEvidence>,
}

pub(super) enum MechanismClassDecision {
    Use(Vec<SourceClass>),
    Skip {
        reason_code: String,
        summary: String,
        direction: MechanisticDirectionEvidence,
    },
}

pub(super) fn mechanism_checked_classes(
    system: &str,
    role: &str,
    row: &Value,
    hypothesis: &InputHypothesis,
    classes: &[SourceClass],
) -> MechanismClassDecision {
    if system == "open_targets" && is_target_disease_hypothesis(hypothesis) {
        return open_targets_direction_decision(row, hypothesis, classes);
    }
    if system == "dgidb"
        && role == "seed_pair_interactions"
        && is_drug_target_hypothesis(hypothesis)
    {
        return dgidb_action_direction_decision(row, hypothesis, classes);
    }
    MechanismClassDecision::Use(classes.to_vec())
}

fn open_targets_direction_decision(
    row: &Value,
    hypothesis: &InputHypothesis,
    classes: &[SourceClass],
) -> MechanismClassDecision {
    if classes
        .iter()
        .all(|class| class.reason == "open_targets_low_score_exact_pair")
    {
        return MechanismClassDecision::Use(classes.to_vec());
    }
    let direction = infer_required_target_modulation(row);
    if !direction.is_required_direction_known() {
        return MechanismClassDecision::Skip {
            reason_code: "CALYX_MECH_OPEN_TARGETS_DIRECTION_MISSING".to_string(),
            summary: "Open Targets row matched endpoints but lacked usable direction-on-target/trait fields"
                .to_string(),
            direction,
        };
    }
    let Some(required) = hypothesis.required_target_modulation else {
        return MechanismClassDecision::Skip {
            reason_code: "CALYX_MECH_HYPOTHESIS_REQUIRED_DIRECTION_MISSING".to_string(),
            summary:
                "target-disease hypothesis lacked required target modulation from the miner report"
                    .to_string(),
            direction,
        };
    };
    if !modulation_compatible(required, direction.required_target_modulation) {
        let expected = modulation_name(required).unwrap_or("unknown");
        let observed = modulation_name(direction.required_target_modulation).unwrap_or("unknown");
        return MechanismClassDecision::Use(vec![SourceClass {
            kind: "counter",
            reason: "mechanistic_required_direction_conflict",
            weight: 4.0,
            summary: format!(
                "Open Targets direction conflict: hypothesis requires {expected}, source implies {observed}"
            ),
            mechanistic_direction: Some(direction),
        }]);
    }
    let mut out = classes.to_vec();
    for class in &mut out {
        class.mechanistic_direction = Some(direction.clone());
    }
    MechanismClassDecision::Use(out)
}

fn dgidb_action_direction_decision(
    row: &Value,
    hypothesis: &InputHypothesis,
    classes: &[SourceClass],
) -> MechanismClassDecision {
    let direction = infer_observed_target_modulation(row);
    if !direction.is_observed_action_known() {
        return MechanismClassDecision::Skip {
            reason_code: "CALYX_MECH_DGIDB_ACTION_DIRECTION_MISSING".to_string(),
            summary: "DGIdb interaction matched endpoints but lacked usable action direction"
                .to_string(),
            direction,
        };
    }
    if let Some(observed) = hypothesis.observed_target_modulation
        && !modulation_compatible(observed, direction.observed_target_modulation)
    {
        let expected = modulation_name(observed).unwrap_or("unknown");
        let source = modulation_name(direction.observed_target_modulation).unwrap_or("unknown");
        return MechanismClassDecision::Use(vec![SourceClass {
            kind: "counter",
            reason: "mechanistic_drug_action_direction_conflict",
            weight: 4.0,
            summary: format!(
                "DGIdb action conflict: hypothesis action {expected}, source action {source}"
            ),
            mechanistic_direction: Some(direction),
        }]);
    }
    let mut out = classes.to_vec();
    for class in &mut out {
        class.mechanistic_direction = Some(direction.clone());
    }
    MechanismClassDecision::Use(out)
}

pub(super) fn classify_pubtator_support(row: &Value) -> Vec<SourceClass> {
    vec![SourceClass {
        kind: "support",
        reason: "pubtator_supporting_literature",
        weight: 1.0 + f64_field(row, "relation_count").unwrap_or(0.0).min(10.0) / 10.0,
        summary: format!(
            "PMID {} relation_count {} support_basis {}",
            str_field(row, "pmid"),
            usize_field(row, "relation_count").unwrap_or(0),
            str_field(row, "support_basis")
        ),
        mechanistic_direction: None,
    }]
}

pub(super) fn classify_pubtator_negative(row: &Value) -> Vec<SourceClass> {
    vec![SourceClass {
        kind: "counter",
        reason: "pubtator_negative_text_signal",
        weight: 2.5,
        summary: format!(
            "PMID {} negative signal {:?}",
            str_field(row, "pmid"),
            row.get("negative_signal_match").and_then(Value::as_str)
        ),
        mechanistic_direction: None,
    }]
}

pub(super) fn classify_trial_summary(row: &Value) -> Vec<SourceClass> {
    let mut out = Vec::new();
    let total = usize_field(row, "total_count").unwrap_or(0);
    if total > 0 {
        out.push(SourceClass {
            kind: "support",
            reason: "clinicaltrials_registry_hits",
            weight: 0.5
                + f64_field(row, "with_results_count")
                    .unwrap_or(0.0)
                    .min(10.0)
                    / 10.0,
            summary: format!(
                "ClinicalTrials.gov total_count {} results {} exact_intervention {}",
                total,
                usize_field(row, "with_results_count").unwrap_or(0),
                usize_field(row, "exact_intervention_match_count").unwrap_or(0)
            ),
            mechanistic_direction: None,
        });
    }
    let stopped = usize_field(row, "stopped_status_count").unwrap_or(0);
    if stopped > 0 {
        out.push(SourceClass {
            kind: "counter",
            reason: "clinicaltrials_stopped_status_count",
            weight: (stopped as f64 * 0.5).clamp(0.5, 3.0),
            summary: format!("ClinicalTrials.gov stopped_status_count {stopped}"),
            mechanistic_direction: None,
        });
    }
    out
}

pub(super) fn classify_trial_row(row: &Value) -> Vec<SourceClass> {
    let status = str_field(row, "overall_status");
    if matches!(status.as_str(), "TERMINATED" | "WITHDRAWN" | "SUSPENDED") {
        return vec![SourceClass {
            kind: "counter",
            reason: "clinicaltrials_stopped_trial",
            weight: 1.0,
            summary: format!(
                "{} {} why_stopped {:?}",
                str_field(row, "nct_id"),
                status,
                row.get("why_stopped").and_then(Value::as_str)
            ),
            mechanistic_direction: None,
        }];
    }
    if row
        .get("has_results")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || status == "COMPLETED"
    {
        return vec![SourceClass {
            kind: "support",
            reason: "clinicaltrials_completed_or_results_trial",
            weight: 0.5,
            summary: format!("{} {} has_results", str_field(row, "nct_id"), status),
            mechanistic_direction: None,
        }];
    }
    Vec::new()
}

pub(super) fn classify_dgidb_interaction(row: &Value) -> Vec<SourceClass> {
    vec![SourceClass {
        kind: "support",
        reason: "dgidb_exact_pair_interaction",
        weight: 1.0 + f64_field(row, "interaction_score").unwrap_or(0.0).min(2.0),
        summary: format!(
            "DGIdb {}-{} interaction_score {} source_dbs {}",
            str_field(row, "drug"),
            str_field(row, "gene"),
            f64_field(row, "interaction_score").unwrap_or(0.0),
            array_len(row, "source_dbs")
        ),
        mechanistic_direction: None,
    }]
}

pub(super) fn classify_dgidb_unmapped(row: &Value) -> Vec<SourceClass> {
    vec![SourceClass {
        kind: "counter",
        reason: "dgidb_exact_pair_no_hit",
        weight: 1.5,
        summary: format!(
            "DGIdb no-hit {}-{} reason {}",
            str_field(row, "drug"),
            str_field(row, "gene"),
            str_field(row, "reason")
        ),
        mechanistic_direction: None,
    }]
}

pub(super) fn classify_open_targets_edge(row: &Value) -> Vec<SourceClass> {
    let score = f64_field(row, "score").unwrap_or(0.0);
    if score >= 0.05 {
        vec![SourceClass {
            kind: "support",
            reason: "open_targets_association_score",
            weight: score.min(1.0),
            summary: format!(
                "Open Targets {} score {} disease {} target {}",
                str_field(row, "open_targets_data_version"),
                score,
                str_field(row, "disease_name"),
                str_field(row, "target_name")
            ),
            mechanistic_direction: None,
        }]
    } else {
        vec![SourceClass {
            kind: "counter",
            reason: "open_targets_low_score_exact_pair",
            weight: 0.5,
            summary: format!("Open Targets low score {score}"),
            mechanistic_direction: None,
        }]
    }
}
