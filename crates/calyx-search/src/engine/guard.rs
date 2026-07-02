use std::collections::BTreeMap;

use calyx_core::{Constellation, CxId, SlotId, SlotVector};
use calyx_sextant::Hit;

use crate::engine_trace::SearchTracer;

#[cfg(test)]
use super::GUARD_TAU;
use super::support::guard_cosine;

pub(super) fn prefilter_in_region_candidates_traced(
    hits: Vec<Hit>,
    query_vectors: &[(SlotId, SlotVector)],
    tau: f32,
    trace: &mut SearchTracer<'_>,
) -> Vec<Hit> {
    let mut kept = Vec::new();
    for hit in hits {
        let best = prefilter_best_score(&hit, query_vectors);
        let accepted = best.is_some_and(|value| value >= tau);
        trace.emit_detail(
            "guard.prefilter.candidate",
            None,
            Some(hit.rank),
            Some(format!(
                "cx_id={} tau={tau:.6} best_index_score={} kept={accepted}",
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

#[cfg(test)]
pub(super) fn prefilter_in_region_candidates(
    hits: Vec<Hit>,
    query_vectors: &[(SlotId, SlotVector)],
) -> Vec<Hit> {
    hits.into_iter()
        .filter(|hit| {
            prefilter_best_score(hit, query_vectors).is_some_and(|score| score >= GUARD_TAU)
        })
        .collect()
}

pub(super) fn apply_in_region_guard_traced(
    hits: Vec<Hit>,
    docs: &BTreeMap<CxId, Constellation>,
    query_vectors: &[(SlotId, SlotVector)],
    tau: f32,
    trace: &mut SearchTracer<'_>,
) -> Vec<Hit> {
    let mut kept = Vec::new();
    for hit in hits {
        let best = guard_cosine(&hit, docs, query_vectors);
        let accepted = best.is_some_and(|value| value >= tau);
        trace.emit_detail(
            "guard.in_region.candidate",
            None,
            Some(hit.rank),
            Some(format!(
                "cx_id={} tau={tau:.6} best_cosine={} kept={accepted}",
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

fn prefilter_best_score(hit: &Hit, query_vectors: &[(SlotId, SlotVector)]) -> Option<f32> {
    hit.per_lens
        .iter()
        .filter_map(|item| {
            let has_dense_query = query_vectors
                .iter()
                .any(|(slot, vector)| *slot == item.slot && vector.as_dense().is_some());
            (has_dense_query && item.raw_score.is_finite()).then_some(item.raw_score)
        })
        .max_by(f32::total_cmp)
}
