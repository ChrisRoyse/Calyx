use std::collections::{BTreeMap, BTreeSet};

use calyx_core::SlotId;
use serde::{Deserialize, Serialize};

use super::model::{
    DEFAULT_GATE_PANEL_LENSES, EnsembleConfig, EnsembleLensRole, EnsembleLensValue,
    EnsemblePairValue,
};
use crate::n_eff::stable_rank;

pub const A37_DIVERSITY_SCHEMA_VERSION: u32 = 2;
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
    #[serde(default)]
    pub content_pair_count: usize,
    #[serde(default)]
    pub expected_content_pair_count: usize,
    #[serde(default)]
    pub pair_evidence_pass: bool,
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
    config: &EnsembleConfig,
) -> A37DiversityGate {
    let mut families = BTreeMap::<String, Vec<SlotId>>::new();
    let mut temporal_sidecar_slots = Vec::new();
    for lens in lenses {
        if lens.role == EnsembleLensRole::TemporalSidecar {
            temporal_sidecar_slots.push(lens.slot);
        } else {
            let family = a37_association_family(&lens.name);
            families
                .entry(family.to_string())
                .or_default()
                .push(lens.slot);
        }
    }
    let content_slots = lenses
        .iter()
        .filter(|lens| lens.role.is_content())
        .map(|lens| lens.slot)
        .collect::<BTreeSet<_>>();
    let lens_slots_unique = lenses
        .iter()
        .map(|lens| lens.slot)
        .collect::<BTreeSet<_>>()
        .len()
        == lenses.len();
    let content_pairs = pairs
        .iter()
        .filter(|pair| content_slots.contains(&pair.slot_a) && content_slots.contains(&pair.slot_b))
        .collect::<Vec<_>>();
    let content_lens_count = content_slots.len();
    let expected_content_pair_count = content_lens_count.saturating_sub(1) * content_lens_count / 2;
    let pair_keys = content_pairs
        .iter()
        .map(|pair| ordered_pair(pair.slot_a, pair.slot_b))
        .collect::<BTreeSet<_>>();
    let pair_values_valid = content_pairs.iter().all(|pair| {
        pair.slot_a != pair.slot_b
            && pair.corr.is_finite()
            && (0.0..=1.0).contains(&pair.corr)
            && pair.nmi.is_finite()
            && (0.0..=1.0).contains(&pair.nmi)
    });
    let pair_evidence_pass = lens_slots_unique
        && expected_content_pair_count > 0
        && content_pairs.len() == expected_content_pair_count
        && pair_keys.len() == expected_content_pair_count
        && pair_values_valid;
    let association_family_count = families.len();
    let n_eff_floor = content_lens_count.max(DEFAULT_GATE_PANEL_LENSES) as f32 * 0.6;
    let family_span_pass = association_family_count >= 2;
    let n_eff = content_stable_rank(&content_slots, &content_pairs);
    let mean_pairwise_corr = mean_pairwise(&content_pairs, |pair| pair.corr).unwrap_or(0.0);
    let mean_pairwise_nmi = mean_pairwise(&content_pairs, |pair| pair.nmi).unwrap_or(0.0);
    let redundancy_bound_pass = pair_evidence_pass
        && n_eff >= n_eff_floor
        && mean_pairwise_corr <= config.max_redundancy
        && mean_pairwise_nmi <= config.max_redundancy;
    let no_collapse_pass = lenses
        .iter()
        .filter(|lens| lens.role.is_content())
        .all(|lens| lens.marginal_bits >= config.min_marginal_bits);
    let sum_unique_pid_bits = lenses
        .iter()
        .filter(|lens| lens.role.is_content())
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
        content_pair_count: pair_keys.len(),
        expected_content_pair_count,
        pair_evidence_pass,
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
            "A37 status={status}; family_span={family_span_pass}; pair_evidence={pair_evidence_pass}; redundancy_bound={redundancy_bound_pass}; no_collapse={no_collapse_pass}"
        ),
    }
}

pub fn a37_association_family(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    let tokens = lower
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if lower.contains("splade") || lower.contains("sparse") || lower.contains("lexical") {
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

fn content_stable_rank(content_slots: &BTreeSet<SlotId>, pairs: &[&EnsemblePairValue]) -> f32 {
    let positions = content_slots
        .iter()
        .enumerate()
        .map(|(index, slot)| (*slot, index))
        .collect::<BTreeMap<_, _>>();
    let mut matrix = vec![vec![0.0; positions.len()]; positions.len()];
    for (index, row) in matrix.iter_mut().enumerate() {
        row[index] = 1.0;
    }
    for pair in pairs {
        let (Some(&a), Some(&b)) = (positions.get(&pair.slot_a), positions.get(&pair.slot_b))
        else {
            continue;
        };
        if pair.corr.is_finite() && (0.0..=1.0).contains(&pair.corr) {
            matrix[a][b] = pair.corr;
            matrix[b][a] = pair.corr;
        }
    }
    stable_rank(&matrix).n_eff
}

fn ordered_pair(a: SlotId, b: SlotId) -> (SlotId, SlotId) {
    if a <= b { (a, b) } else { (b, a) }
}

fn mean_pairwise<F>(pairs: &[&EnsemblePairValue], value: F) -> Option<f32>
where
    F: Fn(&EnsemblePairValue) -> f32,
{
    if pairs.is_empty() {
        return None;
    }
    let sum = pairs.iter().map(|pair| value(pair)).sum::<f32>();
    sum.is_finite().then_some(sum / pairs.len() as f32)
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
            sidecar_lens("plain-control", 1),
        ];
        let gate = a37_diversity_gate(&lenses, &[], &EnsembleConfig::default());

        assert_eq!(
            gate.temporal_lane_role,
            "time_manipulation_walk_forward_backward_as_of_sidecar"
        );
        assert!(!gate.temporal_counts_toward_content_floor);
        assert_eq!(gate.temporal_sidecar_count, 1);
        assert_eq!(gate.content_lens_count, 1);
    }

    #[test]
    fn sidecars_cannot_rescue_collapsed_content_rank_or_dilute_content_means() {
        let mut lenses = (0..10)
            .map(|slot| {
                let name = if slot == 0 {
                    "sparse-keyword".to_string()
                } else {
                    format!("semantic-{slot}")
                };
                lens(&name, slot)
            })
            .collect::<Vec<_>>();
        lenses.extend((10..18).map(|slot| sidecar_lens(&format!("clock-{slot}"), slot)));
        let pairs = (0..lenses.len())
            .flat_map(|a| {
                let lenses = &lenses;
                ((a + 1)..lenses.len()).map(move |b| {
                    let both_content = lenses[a].role.is_content() && lenses[b].role.is_content();
                    pair(
                        &lenses[a],
                        &lenses[b],
                        if both_content { 0.55 } else { 0.0 },
                    )
                })
            })
            .collect::<Vec<_>>();

        let all_lens_n_eff = all_lens_stable_rank(&lenses, &pairs);
        let all_lens_mean = pairs.iter().map(|pair| pair.corr).sum::<f32>() / pairs.len() as f32;
        assert!(all_lens_n_eff >= 6.0, "all-lens n_eff={all_lens_n_eff}");
        assert!(all_lens_mean < 0.6, "all-lens mean={all_lens_mean}");

        let gate = a37_diversity_gate(&lenses, &pairs, &EnsembleConfig::default());

        assert_eq!(gate.status, A37_DIVERSITY_DIAGNOSTIC_ONLY);
        assert_eq!(gate.content_lens_count, 10);
        assert_eq!(gate.temporal_sidecar_count, 8);
        assert_eq!(gate.content_pair_count, 45);
        assert_eq!(gate.expected_content_pair_count, 45);
        assert!(gate.pair_evidence_pass);
        assert!(
            gate.n_eff < gate.n_eff_floor,
            "content n_eff={}",
            gate.n_eff
        );
        assert!((gate.mean_pairwise_corr - 0.55).abs() < 1.0e-6);
        assert!((gate.mean_pairwise_nmi - 0.55).abs() < 1.0e-6);
        assert!(!gate.redundancy_bound_pass);
    }

    #[test]
    fn empty_content_pair_evidence_fails_closed() {
        let lenses = (0..10)
            .map(|slot| lens(if slot == 0 { "sparse" } else { "semantic" }, slot))
            .collect::<Vec<_>>();

        let gate = a37_diversity_gate(&lenses, &[], &EnsembleConfig::default());

        assert_eq!(gate.expected_content_pair_count, 45);
        assert_eq!(gate.content_pair_count, 0);
        assert!(!gate.pair_evidence_pass);
        assert!(!gate.redundancy_bound_pass);
        assert_eq!(gate.status, A37_DIVERSITY_DIAGNOSTIC_ONLY);
    }

    #[test]
    fn duplicate_lens_slots_fail_closed_without_panicking() {
        let mut lenses = (0..10)
            .map(|slot| lens(if slot == 0 { "sparse" } else { "semantic" }, slot))
            .collect::<Vec<_>>();
        lenses[9].slot = SlotId::new(8);
        let pairs = (0..lenses.len())
            .flat_map(|a| {
                let lenses = &lenses;
                ((a + 1)..lenses.len()).map(move |b| pair(&lenses[a], &lenses[b], 0.1))
            })
            .collect::<Vec<_>>();

        let gate = a37_diversity_gate(&lenses, &pairs, &EnsembleConfig::default());

        assert!(!gate.pair_evidence_pass);
        assert!(!gate.redundancy_bound_pass);
        assert_eq!(gate.status, A37_DIVERSITY_DIAGNOSTIC_ONLY);
    }

    #[test]
    fn names_containing_time_remain_content_without_a_sidecar_role() {
        let lenses = vec![
            lens("realtime-semantic", 0),
            lens("datetime-content", 1),
            sidecar_lens("opaque-control", 2),
        ];

        let gate = a37_diversity_gate(&lenses, &[], &EnsembleConfig::default());

        assert_eq!(gate.content_lens_count, 2);
        assert_eq!(gate.temporal_sidecar_slots, vec![SlotId::new(2)]);
        assert_ne!(
            a37_association_family("realtime-semantic"),
            "temporal_sidecar"
        );
    }

    fn lens(name: &str, slot: u16) -> EnsembleLensValue {
        EnsembleLensValue {
            name: name.to_string(),
            slot: SlotId::new(slot),
            role: EnsembleLensRole::Content,
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

    fn sidecar_lens(name: &str, slot: u16) -> EnsembleLensValue {
        EnsembleLensValue {
            role: EnsembleLensRole::TemporalSidecar,
            ..lens(name, slot)
        }
    }

    fn pair(a: &EnsembleLensValue, b: &EnsembleLensValue, redundancy: f32) -> EnsemblePairValue {
        EnsemblePairValue {
            a: a.name.clone(),
            b: b.name.clone(),
            slot_a: a.slot,
            slot_b: b.slot,
            corr: redundancy,
            nmi: redundancy,
            pair_bits: 0.2,
            pair_ci: [0.1, 0.3],
            synergy_gain_bits: 0.0,
        }
    }

    fn all_lens_stable_rank(lenses: &[EnsembleLensValue], pairs: &[EnsemblePairValue]) -> f32 {
        let mut matrix = vec![vec![0.0; lenses.len()]; lenses.len()];
        for (index, row) in matrix.iter_mut().enumerate() {
            row[index] = 1.0;
        }
        for pair in pairs {
            let a = lenses
                .iter()
                .position(|lens| lens.slot == pair.slot_a)
                .unwrap();
            let b = lenses
                .iter()
                .position(|lens| lens.slot == pair.slot_b)
                .unwrap();
            matrix[a][b] = pair.corr;
            matrix[b][a] = pair.corr;
        }
        stable_rank(&matrix).n_eff
    }
}
