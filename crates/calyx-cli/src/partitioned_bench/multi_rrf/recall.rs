use std::collections::{BTreeMap, BTreeSet};

use calyx_core::SlotId;
use serde_json::{Value, json};

use super::ground_truth::PrecomputedTruth;
use super::slot_truth::SlotTruth;
use super::timeline;
use super::{OpenSlot, fuse, fused_hit_ids, report, row_for_metric, slot_id, to_index_hits};
use crate::partitioned_bench::brute_force::brute_force_topk_vecfile_ranked;

pub(super) struct Request<'a> {
    pub(super) slots: &'a [OpenSlot],
    pub(super) truth_n: usize,
    pub(super) truth_depth: usize,
    pub(super) k: usize,
    pub(super) fused_hits: &'a [Vec<u64>],
    pub(super) single_hits: &'a BTreeMap<SlotId, Vec<Vec<u64>>>,
    pub(super) timeline: Option<&'a timeline::Timeline>,
    pub(super) precomputed_truth: Option<&'a PrecomputedTruth>,
    pub(super) slot_truth: Option<&'a SlotTruth>,
}

#[derive(Default)]
pub(super) struct RecallReadback {
    pub(super) fused_recall: Option<f32>,
    pub(super) per_slot_recall: Vec<Value>,
    pub(super) best_single: Option<f32>,
    pub(super) sample_readback: Vec<Value>,
    pub(super) per_query_recall: Vec<f32>,
    pub(super) exact_fused_rows: Vec<Vec<u64>>,
    pub(super) ground_truth_source: Option<Value>,
}

pub(super) fn readback(req: Request<'_>) -> RecallReadback {
    let mut single_found: BTreeMap<SlotId, usize> = req
        .slots
        .iter()
        .map(|slot| (slot_id(slot.spec.slot), 0))
        .collect();
    let mut fused_found = 0usize;
    let mut total = 0usize;
    let mut sample_readback = Vec::new();
    let mut per_query_recall = Vec::with_capacity(req.truth_n);
    let mut exact_fused_rows = Vec::with_capacity(req.truth_n);
    for query_idx in 0..req.truth_n {
        let (exact_ids, exact_slot_rows) = exact_truth_for_query(&req, query_idx);
        let truth = exact_ids.iter().copied().collect::<BTreeSet<_>>();
        if sample_readback.len() < 3 {
            sample_readback.push(sample_row(&req, query_idx, &exact_ids, exact_slot_rows));
        }
        let truth_len = truth.len();
        let query_found = req.fused_hits[query_idx]
            .iter()
            .filter(|id| truth.contains(id))
            .count();
        total += truth_len;
        fused_found += query_found;
        per_query_recall.push(query_found as f32 / truth_len.max(1) as f32);
        for (slot, rows) in req.single_hits {
            let found = rows[query_idx]
                .iter()
                .filter(|id| truth.contains(id))
                .count();
            *single_found.get_mut(slot).expect("slot seeded") += found;
        }
        exact_fused_rows.push(exact_ids);
    }
    let denom = total.max(1) as f32;
    let per_slot = per_slot_recall(single_found, denom);
    let best = per_slot
        .iter()
        .filter_map(|row| row["recall_at_k"].as_f64().map(|value| value as f32))
        .max_by(f32::total_cmp);
    RecallReadback {
        fused_recall: Some(fused_found as f32 / denom),
        per_slot_recall: per_slot,
        best_single: best,
        sample_readback,
        per_query_recall,
        exact_fused_rows,
        ground_truth_source: Some(ground_truth_source(&req)),
    }
}

fn exact_truth_for_query(req: &Request<'_>, query_idx: usize) -> (Vec<u64>, Vec<Value>) {
    if let Some(precomputed) = req.precomputed_truth {
        return (
            precomputed.row_ids(query_idx).to_vec(),
            Vec::from([json!({"source": "precomputed_fused_rrf_i32bin"})]),
        );
    }
    if let Some(slot_truth) = req.slot_truth {
        return fused_from_slot_truth(req, query_idx, slot_truth);
    }
    let mut exact_per_slot = BTreeMap::new();
    let mut exact_slot_rows = Vec::new();
    for slot in req.slots {
        let query = row_for_metric(&slot.queries, query_idx as u64, slot.distance_metric);
        let exact = brute_force_topk_vecfile_ranked(
            &slot.corpus,
            &[query],
            req.truth_depth,
            slot.distance_metric,
        )
        .pop()
        .expect("one query");
        exact_slot_rows.push(json!({
            "slot": slot.spec.slot,
            "exact_top_k": exact.iter().take(req.k).map(|(id, _)| *id).collect::<Vec<_>>(),
        }));
        exact_per_slot.insert(slot_id(slot.spec.slot), to_index_hits(exact));
    }
    let exact_fused = fuse(&exact_per_slot, req.k);
    (fused_hit_ids(&exact_fused, req.k), exact_slot_rows)
}

fn fused_from_slot_truth(
    req: &Request<'_>,
    query_idx: usize,
    slot_truth: &SlotTruth,
) -> (Vec<u64>, Vec<Value>) {
    let mut exact_per_slot = BTreeMap::new();
    let mut exact_slot_rows = Vec::new();
    for slot in req.slots {
        let slot_id = slot_id(slot.spec.slot);
        let rows = slot_truth.row_ids(slot_id, query_idx).to_vec();
        exact_slot_rows.push(json!({
            "slot": slot.spec.slot,
            "source": "precomputed_slot_rrf_i32bin",
            "exact_top_k": rows.iter().take(req.k).copied().collect::<Vec<_>>(),
            "truth_depth": rows.len(),
        }));
        let ranked = rows
            .into_iter()
            .enumerate()
            .map(|(idx, row_id)| (row_id, idx as f32))
            .collect();
        exact_per_slot.insert(slot_id, to_index_hits(ranked));
    }
    let exact_fused = fuse(&exact_per_slot, req.k);
    (fused_hit_ids(&exact_fused, req.k), exact_slot_rows)
}

fn sample_row(
    req: &Request<'_>,
    query_idx: usize,
    exact_ids: &[u64],
    exact_slot_rows: Vec<Value>,
) -> Value {
    let mut row = json!({
        "query_idx": query_idx,
        "partitioned_fused_top_k": req.fused_hits[query_idx],
        "exact_fused_top_k": exact_ids,
        "per_slot_exact_top_k": exact_slot_rows,
    });
    if let Some(timeline) = req.timeline {
        row["query_timeline"] = timeline.row_value(query_idx);
        row["partitioned_fused_timeline"] = timeline.rows_value(&req.fused_hits[query_idx]);
        row["exact_fused_timeline"] = timeline.rows_value(exact_ids);
        row["time_walk"] = timeline.time_walk(query_idx);
    }
    row
}

fn per_slot_recall(single_found: BTreeMap<SlotId, usize>, denom: f32) -> Vec<Value> {
    single_found
        .into_iter()
        .map(|(slot, found)| {
            json!({
                "slot": slot.get(),
                "recall_at_k": found as f32 / denom,
            })
        })
        .collect()
}

fn ground_truth_source(req: &Request<'_>) -> Value {
    req.precomputed_truth
        .map(PrecomputedTruth::source)
        .or_else(|| req.slot_truth.map(SlotTruth::source))
        .unwrap_or_else(|| {
            json!({
                "mode": "cpu_bruteforce_slot_corpora",
                "metric_class": report::METRIC_CLASS,
                "metric_scope": report::METRIC_SCOPE,
                "truth_reference_class": report::TRUTH_REFERENCE_CLASS,
                "valid_real_outcome": false,
                "grounded_phase_exit_eligible": false,
                "scale_suitable": false,
                "query_count": req.truth_n,
                "truth_depth": req.truth_depth,
                "note": "diagnostic exact path only; use precomputed fused truth for scale gates",
            })
        })
}
