use serde_json::{Value, json};

use super::OpenSlot;

pub(super) fn slot_report(slots: &[OpenSlot]) -> Vec<Value> {
    slots
        .iter()
        .map(|slot| {
            json!({
                "slot": slot.spec.slot,
                "name": slot.spec.name.as_deref(),
                "lens_id": slot.spec.lens_id.as_deref().expect("A35 validated"),
                "weights_sha256": slot.spec.weights_sha256.as_deref().expect("A35 validated"),
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
