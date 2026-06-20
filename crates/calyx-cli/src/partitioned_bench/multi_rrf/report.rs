use serde_json::{Value, json};

use super::OpenSlot;

pub(super) const METRIC_CLASS: &str = "ann_correctness";
pub(super) const METRIC_SCOPE: &str = "sextant_index_layer";
pub(super) const TRUTH_REFERENCE_CLASS: &str = "vector_nearest_neighbor_reference";

pub(super) fn slot_report(slots: &[OpenSlot]) -> Vec<Value> {
    slots
        .iter()
        .map(|slot| {
            json!({
                "slot": slot.spec.slot,
                "name": slot.spec.name.as_deref(),
                "lens_id": slot.spec.lens_id.as_deref().expect("A35 validated"),
                "weights_sha256": slot.spec.weights_sha256.as_deref().expect("A35 validated"),
                "signal_kind": slot.spec.signal_kind.as_deref().expect("A35 validated"),
                "bits_about": slot.spec.bits_about.expect("A35 validated"),
                "vault": slot.spec.vault,
                "queries": slot.spec.queries,
                "corpus": slot.spec.corpus,
                "n_cx": slot.search.manifest().n_cx,
                "dim": slot.search.dim(),
                "n_regions": slot.search.manifest().n_regions,
            })
        })
        .collect()
}

pub(super) fn ann_correctness_contract() -> Value {
    json!({
        "metric_class": METRIC_CLASS,
        "metric_scope": METRIC_SCOPE,
        "truth_reference_class": TRUTH_REFERENCE_CLASS,
        "measures": "approximate/index fused RRF overlap with exact or accepted-reference vector-nearest-neighbor ranks",
        "valid_real_outcome": false,
        "grounded_intelligence_metric": false,
    })
}

pub(super) fn grounded_phase_exit_contract() -> Value {
    json!({
        "eligible": false,
        "reason": "partitioned-rrf recall compares the index against vector-nearest-neighbor references from the same frozen panel; no validity-audited real oracle outcome participates",
        "required_gate": {
            "assay": "power-calibrated I(panel;oracle) against a validity-audited real outcome",
            "lodestar": "grounding-kernel coverage >= 0.95 * full over the same valid outcome scope",
            "oracle": "oracle flakiness and validity must bound the trusted confidence",
        },
        "temporal_role": "time manipulation sidecar for walking state forward/backward/as-of; not a content lens and not a grounding substitute",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contracts_classify_partitioned_rrf_as_ann_not_grounding() {
        let ann = ann_correctness_contract();
        let gate = grounded_phase_exit_contract();

        assert_eq!(ann["metric_class"], METRIC_CLASS);
        assert_eq!(ann["valid_real_outcome"], false);
        assert_eq!(gate["eligible"], false);
        assert_eq!(
            gate["required_gate"]["assay"],
            "power-calibrated I(panel;oracle) against a validity-audited real outcome"
        );
    }
}
