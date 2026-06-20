use std::collections::BTreeMap;

use calyx_core::SlotId;
use serde::{Deserialize, Serialize};

use super::model::{
    DEFAULT_GATE_PANEL_LENSES, EnsembleConfig, EnsembleLensValue, EnsemblePairValue,
};

pub const A37_DIVERSITY_SCHEMA_VERSION: u32 = 1;
pub const A37_DIVERSITY_GATE_PASSED: &str = "gate_passed";
pub const A37_DIVERSITY_DIAGNOSTIC_ONLY: &str = "diagnostic_only";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct A37DiversityGate {
    pub schema_version: u32,
    pub role: String,
    pub status: String,
    pub content_lens_count: usize,
    pub temporal_sidecar_count: usize,
    pub temporal_counts_toward_content_floor: bool,
    pub temporal_lane_role: String,
    pub association_family_count: usize,
    pub association_families: BTreeMap<String, Vec<SlotId>>,
    pub temporal_sidecar_slots: Vec<SlotId>,
    pub family_span_pass: bool,
    pub redundancy_bound_pass: bool,
    pub no_collapse_pass: bool,
    pub n_eff: f32,
    pub n_eff_floor: f32,
    pub mean_pairwise_corr: f32,
    pub mean_pairwise_nmi: f32,
    pub max_redundancy: f32,
    pub sum_unique_pid_bits: f32,
    pub min_marginal_bits: f32,
    pub verdict: String,
}

pub fn a37_diversity_gate(
    lenses: &[EnsembleLensValue],
    pairs: &[EnsemblePairValue],
    n_eff: f32,
    config: &EnsembleConfig,
) -> A37DiversityGate {
    let mut families = BTreeMap::<String, Vec<SlotId>>::new();
    let mut temporal_sidecar_slots = Vec::new();
    for lens in lenses {
        let family = a37_association_family(&lens.name);
        if family == "temporal_sidecar" {
            temporal_sidecar_slots.push(lens.slot);
        } else {
            families
                .entry(family.to_string())
                .or_default()
                .push(lens.slot);
        }
    }
    let content_lens_count = lenses.len().saturating_sub(temporal_sidecar_slots.len());
    let association_family_count = families.len();
    let n_eff_floor = content_lens_count.max(DEFAULT_GATE_PANEL_LENSES) as f32 * 0.6;
    let family_span_pass = association_family_count >= 2;
    let mean_pairwise_corr = mean_pairwise(pairs, |pair| pair.corr);
    let mean_pairwise_nmi = mean_pairwise(pairs, |pair| pair.nmi);
    let redundancy_bound_pass = n_eff >= n_eff_floor
        && mean_pairwise_corr <= config.max_redundancy
        && mean_pairwise_nmi <= config.max_redundancy;
    let no_collapse_pass = lenses
        .iter()
        .filter(|lens| !temporal_sidecar_slots.contains(&lens.slot))
        .all(|lens| lens.marginal_bits >= config.min_marginal_bits);
    let sum_unique_pid_bits = lenses
        .iter()
        .filter(|lens| !temporal_sidecar_slots.contains(&lens.slot))
        .map(|lens| lens.pid.unique_bits)
        .sum::<f32>();
    let status = if family_span_pass && redundancy_bound_pass && no_collapse_pass {
        A37_DIVERSITY_GATE_PASSED
    } else {
        A37_DIVERSITY_DIAGNOSTIC_ONLY
    };
    A37DiversityGate {
        schema_version: A37_DIVERSITY_SCHEMA_VERSION,
        role: "a37_associational_diversity_gate".to_string(),
        status: status.to_string(),
        content_lens_count,
        temporal_sidecar_count: temporal_sidecar_slots.len(),
        temporal_counts_toward_content_floor: false,
        temporal_lane_role: "time_manipulation_walk_forward_backward_as_of_sidecar".to_string(),
        association_family_count,
        association_families: families,
        temporal_sidecar_slots,
        family_span_pass,
        redundancy_bound_pass,
        no_collapse_pass,
        n_eff,
        n_eff_floor,
        mean_pairwise_corr,
        mean_pairwise_nmi,
        max_redundancy: config.max_redundancy,
        sum_unique_pid_bits,
        min_marginal_bits: config.min_marginal_bits,
        verdict: format!(
            "A37 status={status}; family_span={family_span_pass}; redundancy_bound={redundancy_bound_pass}; no_collapse={no_collapse_pass}"
        ),
    }
}

pub fn a37_association_family(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    let tokens = lower
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if lower.contains("temporal") || lower.contains("time") || lower.contains("as-of") {
        "temporal_sidecar"
    } else if lower.contains("splade") || lower.contains("sparse") || lower.contains("lexical") {
        "lexical_sparse"
    } else if lower.contains("late")
        || lower.contains("colbert")
        || lower.contains("token")
        || lower.contains("interaction")
    {
        "late_interaction_token"
    } else if lower.contains("entity")
        || lower.contains("cameo")
        || lower.contains("graph")
        || lower.contains("actor")
        || lower.contains("geo")
    {
        "entity_cameo_graph"
    } else if lower.contains("byte") || lower.contains("char") {
        "byte_char"
    } else if tokens.iter().any(|token| matches!(*token, "ast" | "cfg"))
        || lower.contains("structural")
        || lower.contains("dataflow")
    {
        "structural"
    } else if lower.contains("rerank") || lower.contains("cross-encoder") {
        "reranker_asymmetric"
    } else if tokens.iter().any(|token| {
        matches!(
            *token,
            "domain" | "legal" | "clinical" | "medical" | "financial" | "scientific" | "scibert"
        )
    }) {
        "dense_semantic_domain"
    } else {
        "dense_semantic_general"
    }
}

fn mean_pairwise<F>(pairs: &[EnsemblePairValue], value: F) -> f32
where
    F: Fn(&EnsemblePairValue) -> f32,
{
    pairs.iter().map(value).sum::<f32>() / pairs.len().max(1) as f32
}

#[cfg(test)]
mod tests {
    use calyx_core::SlotId;

    use super::*;
    use crate::ensemble::model::{EnsembleDecision, PidBits};

    #[test]
    fn a37_records_temporal_lane_as_time_manipulation_sidecar() {
        let lenses = vec![
            lens("semantic-general", 0),
            lens("temporal-as-of-sidecar", 1),
        ];
        let gate = a37_diversity_gate(&lenses, &[], 1.0, &EnsembleConfig::default());

        assert_eq!(
            gate.temporal_lane_role,
            "time_manipulation_walk_forward_backward_as_of_sidecar"
        );
        assert!(!gate.temporal_counts_toward_content_floor);
        assert_eq!(gate.temporal_sidecar_count, 1);
        assert_eq!(gate.content_lens_count, 1);
    }

    fn lens(name: &str, slot: u16) -> EnsembleLensValue {
        EnsembleLensValue {
            name: name.to_string(),
            slot: SlotId::new(slot),
            solo_bits: 0.2,
            solo_ci: [0.1, 0.3],
            panel_without_bits: 0.15,
            marginal_bits: 0.05,
            marginal_ci: [0.02, 0.08],
            pid: PidBits {
                unique_bits: 0.05,
                redundant_bits: 0.1,
                synergistic_bits: 0.01,
            },
            max_pairwise_corr: 0.1,
            max_pairwise_nmi: 0.1,
            decision: EnsembleDecision::Keep,
            decision_reason: "unit".to_string(),
        }
    }
}
