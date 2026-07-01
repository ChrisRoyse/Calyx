use std::collections::BTreeMap;

use calyx_core::{Constellation, CxId, SlotId, SlotVector};
use calyx_sextant::Hit;

use crate::engine_trace::SearchTracer;

use super::GUARD_TAU;
use super::support::guard_cosine;

pub(super) fn apply_in_region_guard_traced(
    hits: Vec<Hit>,
    docs: &BTreeMap<CxId, Constellation>,
    query_vectors: &[(SlotId, SlotVector)],
    trace: &mut SearchTracer<'_>,
) -> Vec<Hit> {
    let mut kept = Vec::new();
    for hit in hits {
        let best = guard_cosine(&hit, docs, query_vectors);
        let accepted = best.is_some_and(|value| value >= GUARD_TAU);
        trace.emit_detail(
            "guard.in_region.candidate",
            None,
            Some(hit.rank),
            Some(format!(
                "cx_id={} tau={GUARD_TAU:.6} best_cosine={} kept={accepted}",
                hit.cx_id,
                best.map(|value| format!("{value:.6}"))
                    .unwrap_or_else(|| "missing".to_string())
            )),
        );
        if accepted {
            kept.push(hit);
        }
    }
    kept
}
