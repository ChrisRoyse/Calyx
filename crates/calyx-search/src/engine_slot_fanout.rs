//! Concurrent per-lens retrieval with deterministic trace and result ordering.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use calyx_core::{CalyxError, CxId, SlotId, SlotVector};
use calyx_sextant::IndexSearchHit;

use crate::engine_measure::slot_vector_shape;
use crate::engine_trace::SearchTracer;
use crate::error::CliResult;
use crate::persisted::PersistedSearchIndexes;

const MAX_MULTI_STAGE1_CANDIDATES: usize = 256;

pub(crate) type SlotHitsWithLatency = (
    BTreeMap<SlotId, Vec<IndexSearchHit>>,
    BTreeMap<SlotId, u128>,
);

pub(crate) fn search_slots_uncached(
    indexes: &PersistedSearchIndexes,
    query_vectors: &[(SlotId, SlotVector)],
    k: usize,
    filter_candidates: Option<&BTreeSet<CxId>>,
    trace: &mut SearchTracer<'_>,
) -> CliResult<SlotHitsWithLatency> {
    let (multi_jobs, stage1_jobs): (Vec<_>, Vec<_>) = query_vectors
        .iter()
        // Use the core shape classifier instead of naming one representation
        // here. `multi_dim` exhaustively covers both native late-interaction
        // forms (Multi and SegmentedMulti), so a future form cannot silently
        // fall into the full-corpus stage-one path.
        .partition(|(_, query)| query.multi_dim().is_some());
    trace.emit_detail(
        "search_slots.stage1.start",
        None,
        Some(stage1_jobs.len()),
        Some(format!("multi_slots={}", multi_jobs.len())),
    );
    let (mut out, mut elapsed_by_slot) =
        run_parallel_slot_searches(indexes, &stage1_jobs, k, filter_candidates, "stage1", trace)?;
    trace.emit("search_slots.stage1.done", None, Some(stage1_jobs.len()));

    let (multi_candidates, candidate_source) = if multi_jobs.is_empty() {
        (None, "none")
    } else if filter_candidates
        .is_some_and(|candidates| candidates.len() <= k.min(MAX_MULTI_STAGE1_CANDIDATES))
    {
        (filter_candidates.cloned(), "filter")
    } else {
        let candidates = stage1_candidates(&out, k.min(MAX_MULTI_STAGE1_CANDIDATES));
        if candidates.is_empty() {
            (filter_candidates.cloned(), "empty_stage1")
        } else {
            (Some(candidates), "fused_stage1")
        }
    };
    trace.emit_detail(
        "search_slots.multi_candidates",
        None,
        multi_candidates.as_ref().map(BTreeSet::len),
        Some(format!(
            "source={candidate_source} limit={}",
            k.min(MAX_MULTI_STAGE1_CANDIDATES)
        )),
    );
    let (multi_hits, multi_elapsed) = run_parallel_slot_searches(
        indexes,
        &multi_jobs,
        k,
        multi_candidates.as_ref().or(filter_candidates),
        "multi_rerank",
        trace,
    )?;
    out.extend(multi_hits);
    elapsed_by_slot.extend(multi_elapsed);
    Ok((out, elapsed_by_slot))
}

fn run_parallel_slot_searches(
    indexes: &PersistedSearchIndexes,
    jobs: &[&(SlotId, SlotVector)],
    k: usize,
    candidates: Option<&BTreeSet<CxId>>,
    stage: &'static str,
    trace: &mut SearchTracer<'_>,
) -> CliResult<SlotHitsWithLatency> {
    for (slot, query) in jobs {
        trace.emit_detail(
            "search_slot.start",
            Some(*slot),
            Some(k),
            Some(format!("stage={stage} {}", slot_vector_shape(query))),
        );
    }
    let outcomes = std::thread::scope(|scope| {
        jobs.iter()
            .map(|job| {
                scope.spawn(move || {
                    let slot = job.0;
                    let query = &job.1;
                    clear_maxsim_cuda_telemetry();
                    let _ = take_slot_serving_detail();
                    let started = Instant::now();
                    let hits = if let Some(candidates) = candidates {
                        indexes.search_filtered(slot, query, k, candidates)
                    } else {
                        indexes.search(slot, query, k)
                    };
                    let maxsim_cuda_detail = take_maxsim_cuda_detail();
                    let serving_detail = take_slot_serving_detail();
                    (
                        slot,
                        started.elapsed().as_millis(),
                        hits,
                        maxsim_cuda_detail,
                        serving_detail,
                    )
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join())
            .collect::<Vec<_>>()
    });
    let mut out = BTreeMap::new();
    let mut elapsed_by_slot = BTreeMap::new();
    for (job, outcome) in jobs.iter().zip(outcomes) {
        let (slot, slot_elapsed_ms, hits, maxsim_cuda_detail, serving_detail) =
            outcome.map_err(|_| {
                let slot = job.0;
                CalyxError::stale_derived(format!(
                    "parallel persisted search thread panicked for slot {slot} stage {stage}"
                ))
            })?;
        let hits = hits?;
        elapsed_by_slot.insert(slot, slot_elapsed_ms);
        let mut detail = format!("stage={stage} slot_elapsed_ms={slot_elapsed_ms}");
        if let Some(maxsim_cuda_detail) = maxsim_cuda_detail {
            detail.push(' ');
            detail.push_str(&maxsim_cuda_detail);
        }
        if let Some(serving_detail) = serving_detail {
            detail.push(' ');
            detail.push_str(&serving_detail);
        }
        trace.emit_detail(
            "search_slot.done",
            Some(slot),
            Some(hits.len()),
            Some(detail),
        );
        if !hits.is_empty() {
            out.insert(slot, hits);
        }
    }
    Ok((out, elapsed_by_slot))
}

thread_local! {
    static LAST_SERVING_DETAIL: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub(crate) fn record_slot_serving_detail(detail: String) {
    LAST_SERVING_DETAIL.with(|cell| cell.replace(Some(detail)));
}

fn take_slot_serving_detail() -> Option<String> {
    LAST_SERVING_DETAIL.with(|cell| cell.borrow_mut().take())
}

#[cfg(feature = "cuda")]
fn clear_maxsim_cuda_telemetry() {
    let _ = crate::persisted::take_maxsim_cuda_detail();
}

#[cfg(not(feature = "cuda"))]
fn clear_maxsim_cuda_telemetry() {}

#[cfg(feature = "cuda")]
fn take_maxsim_cuda_detail() -> Option<String> {
    crate::persisted::take_maxsim_cuda_detail()
}

#[cfg(not(feature = "cuda"))]
fn take_maxsim_cuda_detail() -> Option<String> {
    None
}

fn stage1_candidates(
    per_slot: &BTreeMap<SlotId, Vec<IndexSearchHit>>,
    limit: usize,
) -> BTreeSet<CxId> {
    let mut scores = BTreeMap::<CxId, f64>::new();
    for hits in per_slot.values() {
        for hit in hits {
            *scores.entry(hit.cx_id).or_default() += 1.0 / (hit.rank as f64 + 60.0);
        }
    }
    let mut ranked = scores.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    ranked
        .into_iter()
        .take(limit)
        .map(|(cx_id, _)| cx_id)
        .collect()
}
