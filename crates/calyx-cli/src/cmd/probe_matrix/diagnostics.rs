use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::path::Path;
use std::time::Instant;

use calyx_core::{SlotId, SlotVector};
use calyx_registry::VaultPanelState;
use calyx_search::SearchTraceEvent;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::ProbeMatrixLog;
use super::resident;
use super::support::{accepted_hit_count, hex_lower};
use crate::error::CliResult;
use crate::fsv_grounding::GroundingAudit;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProbeMatrixArtifactStatus {
    Ok,
    Refused,
    Incomplete,
}

impl ProbeMatrixArtifactStatus {
    pub(super) fn from_log(log: &ProbeMatrixLog) -> Self {
        if accepted_hit_count(log) > 0 && !log.productive.is_empty() {
            Self::Ok
        } else {
            Self::Refused
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ProbeMatrixDiagnostics {
    pub query_measurements: Vec<ProbeMatrixQueryMeasurement>,
    #[serde(default)]
    pub search_result_cache: calyx_search::SearchSlotCacheDiagnostic,
    pub variant_guard_counts: Vec<ProbeMatrixVariantDiagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grounding_preflight: Option<GroundingAudit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ProbeMatrixQueryMeasurement {
    pub query_text_sha256: String,
    pub measured_slot_count: usize,
    pub measure_call_count: usize,
    pub variant_use_count: usize,
    pub elapsed_ms: u128,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ProbeMatrixVariantDiagnostic {
    pub variant_id: usize,
    pub query_text_sha256: String,
    pub guard_prefilter_input_count: Option<usize>,
    pub guard_prefilter_output_count: Option<usize>,
    pub guard_prefilter_filtered_count: Option<usize>,
    pub guard_prefilter_elapsed_ms: Option<u128>,
    pub hit_hydration_candidate_count: Option<usize>,
    pub hit_hydration_doc_count: Option<usize>,
    pub hit_hydration_elapsed_ms: Option<u128>,
    pub per_hit_hydrate_start_count: usize,
    pub per_hit_hydrate_done_count: usize,
    pub pre_guard_hit_count: Option<usize>,
    pub post_guard_hit_count: Option<usize>,
    pub guard_filtered_hit_count: Option<usize>,
    pub guard_tau: Option<String>,
    pub guard_best_cosine_min: Option<String>,
    pub guard_best_cosine_max: Option<String>,
    pub guard_missing_cosine_count: Option<usize>,
    pub guard_start_elapsed_ms: Option<u128>,
    pub guard_done_elapsed_ms: Option<u128>,
    pub search_cache_key_sha256: Option<String>,
    pub search_cache_lookup_count: usize,
    pub search_cache_hit_count: usize,
    pub search_cache_miss_count: usize,
    pub search_cache_hit_slot_count: usize,
    pub search_cache_miss_slot_count: usize,
    pub search_cache_reused_hit_count: Option<usize>,
    pub search_cache_stored_hit_count: Option<usize>,
    pub search_cache_store_elapsed_ms: Option<u128>,
    pub search_done_elapsed_ms: Option<u128>,
    pub last_search_phase: Option<String>,
    pub last_search_elapsed_ms: Option<u128>,
    pub guard_zero_hit_reason: Option<String>,
    #[serde(default)]
    pub slot_searches: Vec<super::slot_timings::ProbeMatrixSlotSearchDiagnostic>,
}

pub(super) struct QueryVectorCache {
    allowed_slots: BTreeSet<SlotId>,
    entries: BTreeMap<String, CachedQueryVectors>,
}

struct CachedQueryVectors {
    query_text_sha256: String,
    vectors: Vec<(SlotId, SlotVector)>,
    elapsed_ms: u128,
    variant_use_count: usize,
}

impl QueryVectorCache {
    pub(super) fn new(allowed_slots: BTreeSet<SlotId>) -> Self {
        Self {
            allowed_slots,
            entries: BTreeMap::new(),
        }
    }

    pub(super) fn query_vectors<'a>(
        &'a mut self,
        state: &VaultPanelState,
        vault_dir: &Path,
        query: &str,
        resident_addr: Option<SocketAddr>,
    ) -> CliResult<(String, &'a [(SlotId, SlotVector)])> {
        if !self.entries.contains_key(query) {
            let started = Instant::now();
            let query_text_sha256 = sha256_text(query);
            eprintln!(
                "probe-matrix: query measurement cache_miss query_sha256={} selected_slots={} resident_addr={:?}",
                query_text_sha256,
                self.allowed_slots.len(),
                resident_addr
            );
            let vectors = match resident_addr {
                Some(addr) => resident::measure_query_vectors_via_resident(
                    state,
                    vault_dir,
                    query,
                    &self.allowed_slots,
                    addr,
                )?,
                None => calyx_search::engine::measure_query_vectors_with_slots(
                    state,
                    query,
                    Some(&self.allowed_slots),
                )?,
            };
            let elapsed_ms = started.elapsed().as_millis();
            eprintln!(
                "probe-matrix: query measurement cached query_sha256={} measured_slots={} elapsed_ms={}",
                query_text_sha256,
                vectors.len(),
                elapsed_ms
            );
            self.entries.insert(
                query.to_string(),
                CachedQueryVectors {
                    query_text_sha256,
                    vectors,
                    elapsed_ms,
                    variant_use_count: 0,
                },
            );
        }
        let entry = self
            .entries
            .get_mut(query)
            .expect("query vector cache entry inserted before readback");
        entry.variant_use_count += 1;
        eprintln!(
            "probe-matrix: query measurement cache_hit query_sha256={} use_count={} measured_slots={}",
            entry.query_text_sha256,
            entry.variant_use_count,
            entry.vectors.len()
        );
        Ok((entry.query_text_sha256.clone(), entry.vectors.as_slice()))
    }

    pub(super) fn diagnostics(&self) -> Vec<ProbeMatrixQueryMeasurement> {
        self.entries
            .values()
            .map(|entry| ProbeMatrixQueryMeasurement {
                query_text_sha256: entry.query_text_sha256.clone(),
                measured_slot_count: entry.vectors.len(),
                measure_call_count: 1,
                variant_use_count: entry.variant_use_count,
                elapsed_ms: entry.elapsed_ms,
            })
            .collect()
    }
}

pub(super) fn variant_guard_diagnostic(
    variant_id: usize,
    query_text_sha256: String,
    events: &[SearchTraceEvent],
) -> ProbeMatrixVariantDiagnostic {
    let prefilter_in = count_for_phase(events, "guard.prefilter.start");
    let prefilter_out = count_for_phase(events, "guard.prefilter.done");
    let pre = count_for_phase(events, "guard.in_region.start");
    let post = count_for_phase(events, "guard.in_region.done");
    let summary = guard_candidate_summary(events);
    let last_event = events.last();
    ProbeMatrixVariantDiagnostic {
        variant_id,
        query_text_sha256,
        guard_prefilter_input_count: prefilter_in,
        guard_prefilter_output_count: prefilter_out,
        guard_prefilter_filtered_count: match (prefilter_in, prefilter_out) {
            (Some(before), Some(after)) => Some(before.saturating_sub(after)),
            _ => None,
        },
        guard_prefilter_elapsed_ms: elapsed_for_phase(events, "guard.prefilter.done"),
        hit_hydration_candidate_count: count_for_phase(events, "hit_docs.hydrate.start"),
        hit_hydration_doc_count: count_for_phase(events, "hit_docs.hydrate.done"),
        hit_hydration_elapsed_ms: elapsed_for_phase(events, "hit_docs.hydrate.done"),
        per_hit_hydrate_start_count: event_count_for_phase(events, "hit_doc.hydrate.start"),
        per_hit_hydrate_done_count: event_count_for_phase(events, "hit_doc.hydrate.done"),
        pre_guard_hit_count: pre,
        post_guard_hit_count: post,
        guard_filtered_hit_count: match (pre, post) {
            (Some(before), Some(after)) => Some(before.saturating_sub(after)),
            _ => None,
        },
        guard_tau: summary.tau,
        guard_best_cosine_min: summary.min,
        guard_best_cosine_max: summary.max,
        guard_missing_cosine_count: summary.missing,
        guard_start_elapsed_ms: elapsed_for_phase(events, "guard.in_region.start"),
        guard_done_elapsed_ms: elapsed_for_phase(events, "guard.in_region.done"),
        search_cache_key_sha256: last_cache_key(events),
        search_cache_lookup_count: event_count_for_phase(events, "search_slots.cache.lookup"),
        search_cache_hit_count: event_count_for_phase(events, "search_slots.cache.hit"),
        search_cache_miss_count: event_count_for_phase(events, "search_slots.cache.miss"),
        search_cache_hit_slot_count: count_for_phase(events, "search_slots.cache.hit").unwrap_or(0),
        search_cache_miss_slot_count: count_for_phase(events, "search_slots.cache.miss")
            .unwrap_or(0),
        search_cache_reused_hit_count: detail_usize(events, "search_slots.cache.hit", "hit_count"),
        search_cache_stored_hit_count: detail_usize(
            events,
            "search_slots.cache.store",
            "hit_count",
        ),
        search_cache_store_elapsed_ms: detail_u128(
            events,
            "search_slots.cache.store",
            "search_elapsed_ms",
        ),
        search_done_elapsed_ms: elapsed_for_phase(events, "search.done"),
        last_search_phase: last_event.map(|event| event.phase.to_string()),
        last_search_elapsed_ms: last_event.map(|event| event.elapsed_ms),
        guard_zero_hit_reason: guard_zero_hit_reason(prefilter_in, prefilter_out, pre, post),
        slot_searches: super::slot_timings::slot_search_diagnostics(events),
    }
}

fn count_for_phase(events: &[SearchTraceEvent], phase: &str) -> Option<usize> {
    events
        .iter()
        .rev()
        .find(|event| event.phase == phase)
        .and_then(|event| event.count)
}

fn elapsed_for_phase(events: &[SearchTraceEvent], phase: &str) -> Option<u128> {
    events
        .iter()
        .rev()
        .find(|event| event.phase == phase)
        .map(|event| event.elapsed_ms)
}

fn event_count_for_phase(events: &[SearchTraceEvent], phase: &str) -> usize {
    events.iter().filter(|event| event.phase == phase).count()
}

fn detail_usize(events: &[SearchTraceEvent], phase: &str, field: &str) -> Option<usize> {
    detail_value(events, phase, field).and_then(|value| value.parse::<usize>().ok())
}

fn detail_u128(events: &[SearchTraceEvent], phase: &str, field: &str) -> Option<u128> {
    detail_value(events, phase, field).and_then(|value| value.parse::<u128>().ok())
}

fn detail_value<'a>(events: &'a [SearchTraceEvent], phase: &str, field: &str) -> Option<&'a str> {
    events
        .iter()
        .rev()
        .filter(|event| event.phase == phase)
        .filter_map(|event| event.detail.as_deref())
        .find_map(|detail| detail_field(detail, field))
}

fn last_cache_key(events: &[SearchTraceEvent]) -> Option<String> {
    [
        "search_slots.cache.hit",
        "search_slots.cache.miss",
        "search_slots.cache.store",
        "search_slots.cache.lookup",
    ]
    .iter()
    .find_map(|phase| detail_value(events, phase, "key_sha256"))
    .map(str::to_string)
}

fn sha256_text(query: &str) -> String {
    hex_lower(&Sha256::digest(query.as_bytes()))
}

#[derive(Default)]
struct GuardCandidateSummary {
    tau: Option<String>,
    min: Option<String>,
    max: Option<String>,
    missing: Option<usize>,
}

fn guard_candidate_summary(events: &[SearchTraceEvent]) -> GuardCandidateSummary {
    // The exact guard's recomputed cosines are the primary evidence; when the
    // prefilter already rejected every candidate the exact guard never ran, so
    // fall back to the prefilter's dense index scores (the same cosine space)
    // rather than reporting no measurable cosine at all — the observed range is
    // the operator's calibration evidence (#1088).
    let exact = phase_candidate_summary(events, "guard.in_region.candidate", "best_cosine");
    if exact.tau.is_some() || exact.min.is_some() || exact.missing.is_some() {
        return exact;
    }
    phase_candidate_summary(events, "guard.prefilter.candidate", "best_index_score")
}

fn phase_candidate_summary(
    events: &[SearchTraceEvent],
    phase: &str,
    score_field: &str,
) -> GuardCandidateSummary {
    let mut scores = Vec::new();
    let mut missing = 0usize;
    let mut tau = None;
    for detail in events
        .iter()
        .filter(|event| event.phase == phase)
        .filter_map(|event| event.detail.as_deref())
    {
        if tau.is_none() {
            tau = detail_field(detail, "tau").map(str::to_string);
        }
        match detail_field(detail, score_field) {
            Some("missing") | None => missing += 1,
            Some(value) => {
                if let Ok(score) = value.parse::<f32>() {
                    scores.push(score);
                }
            }
        }
    }
    scores.sort_by(f32::total_cmp);
    GuardCandidateSummary {
        tau,
        min: scores.first().map(|value| format!("{value:.6}")),
        max: scores.last().map(|value| format!("{value:.6}")),
        missing: (!scores.is_empty() || missing > 0).then_some(missing),
    }
}

fn detail_field<'a>(detail: &'a str, field: &str) -> Option<&'a str> {
    detail
        .split_whitespace()
        .find_map(|part| part.strip_prefix(field)?.strip_prefix('='))
}

pub(super) fn guard_zero_hit_reason(
    prefilter_in: Option<usize>,
    prefilter_out: Option<usize>,
    pre: Option<usize>,
    post: Option<usize>,
) -> Option<String> {
    match (prefilter_in, prefilter_out, pre, post) {
        (Some(before), Some(0), _, _) if before > 0 => {
            Some("in_region_guard_prefilter_rejected_all_candidates".to_string())
        }
        (_, _, Some(before), Some(0)) if before > 0 => {
            Some("in_region_guard_filtered_all_candidates".to_string())
        }
        (_, _, Some(0), Some(0)) => Some("no_pre_guard_hits".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn prefilter_only_rejection_still_reports_tau_and_cosine_range() {
        // #1088 readback: when the prefilter rejects every candidate the exact
        // guard never emits events, but the persisted diagnostics must still
        // carry the applied tau and the observed dense-score range — that range
        // is the operator's calibration evidence.
        let event =
            |phase: &'static str, count: Option<usize>, detail: Option<&str>| SearchTraceEvent {
                phase,
                slot: None,
                elapsed_ms: 1,
                count,
                detail: detail.map(str::to_string),
            };
        let events = vec![
            event("guard.prefilter.start", Some(2), None),
            event(
                "guard.prefilter.candidate",
                Some(1),
                Some("cx_id=a tau=1.000000 best_index_score=0.401000 kept=false"),
            ),
            event(
                "guard.prefilter.candidate",
                Some(2),
                Some("cx_id=b tau=1.000000 best_index_score=0.788000 kept=false"),
            ),
            event(
                "guard.prefilter.done",
                Some(0),
                Some("filtered=2 tau=1.000000"),
            ),
        ];
        let row = variant_guard_diagnostic(7, "sha-7".to_string(), &events);
        assert_eq!(row.guard_tau.as_deref(), Some("1.000000"));
        assert_eq!(row.guard_best_cosine_min.as_deref(), Some("0.401000"));
        assert_eq!(row.guard_best_cosine_max.as_deref(), Some("0.788000"));
        assert_eq!(row.guard_missing_cosine_count, Some(0));
        assert_eq!(
            row.guard_zero_hit_reason.as_deref(),
            Some("in_region_guard_prefilter_rejected_all_candidates")
        );
    }

    #[test]
    fn zero_hit_reason_prefers_prefilter_rejection() {
        assert_eq!(
            guard_zero_hit_reason(Some(64), Some(0), Some(0), Some(0)).as_deref(),
            Some("in_region_guard_prefilter_rejected_all_candidates")
        );
    }

    #[test]
    fn zero_hit_reason_reports_exact_guard_filtering() {
        assert_eq!(
            guard_zero_hit_reason(Some(64), Some(4), Some(4), Some(0)).as_deref(),
            Some("in_region_guard_filtered_all_candidates")
        );
    }

    #[test]
    fn zero_hit_reason_reports_no_pre_guard_hits() {
        assert_eq!(
            guard_zero_hit_reason(None, None, Some(0), Some(0)).as_deref(),
            Some("no_pre_guard_hits")
        );
    }
}
