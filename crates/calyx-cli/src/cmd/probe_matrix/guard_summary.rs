use std::collections::BTreeSet;

use super::diagnostics::ProbeMatrixVariantDiagnostic;

/// Aggregate evidence that the in-region guard filtered every candidate the
/// search path actually retrieved, across the completed variants. Used to fail
/// closed with a specific diagnosis instead of a generic empty-benchmark error
/// (issue #1088).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct GuardFilteredAllSummary {
    pub variant_count: usize,
    pub retrieved_candidate_count: usize,
    pub filtered_candidate_count: usize,
    pub observed_best_cosine_min: Option<String>,
    pub observed_best_cosine_max: Option<String>,
    pub tau: Option<String>,
    pub reasons: BTreeSet<String>,
}

/// Returns `Some` only when at least one completed variant retrieved candidates
/// AND every variant that retrieved candidates had all of them filtered by the
/// in-region guard (prefilter or full cosine). `None` means the guard is not the
/// reason the benchmark lacks accepted hits.
pub(super) fn guard_filtered_all_summary(
    guards: &[ProbeMatrixVariantDiagnostic],
) -> Option<GuardFilteredAllSummary> {
    let mut retrieved_total = 0usize;
    let mut filtered_total = 0usize;
    let mut variants_with_candidates = 0usize;
    let mut reasons = BTreeSet::new();
    let mut cosine_mins = Vec::new();
    let mut cosine_maxes = Vec::new();
    let mut tau = None;
    for guard in guards {
        // Candidates the search path actually retrieved for this variant, before
        // the guard: the prefilter input count is the ground truth.
        let retrieved = guard.guard_prefilter_input_count.unwrap_or(0);
        if retrieved == 0 {
            // No candidates retrieved: this variant's emptiness is not a guard
            // artifact, so it neither triggers nor blocks the diagnosis.
            continue;
        }
        variants_with_candidates += 1;
        // Survivors after the full in-region guard (post-guard hit count),
        // falling back to the prefilter output when the guard stage did not run.
        let survivors = guard
            .post_guard_hit_count
            .or(guard.guard_prefilter_output_count)
            .unwrap_or(0);
        if survivors > 0 {
            // At least one variant kept in-region candidates: the guard is not
            // filtering everything, so this is not the guard-filtered-all case.
            return None;
        }
        retrieved_total += retrieved;
        filtered_total += guard
            .guard_prefilter_filtered_count
            .unwrap_or(retrieved)
            .max(guard.guard_filtered_hit_count.unwrap_or(0));
        if let Some(reason) = &guard.guard_zero_hit_reason {
            reasons.insert(reason.clone());
        }
        if tau.is_none() {
            tau = guard.guard_tau.clone();
        }
        if let Some(min) = &guard.guard_best_cosine_min {
            cosine_mins.push(min.clone());
        }
        if let Some(max) = &guard.guard_best_cosine_max {
            cosine_maxes.push(max.clone());
        }
    }
    if variants_with_candidates == 0 {
        return None;
    }
    Some(GuardFilteredAllSummary {
        variant_count: variants_with_candidates,
        retrieved_candidate_count: retrieved_total,
        filtered_candidate_count: filtered_total,
        observed_best_cosine_min: cosine_mins.into_iter().min(),
        observed_best_cosine_max: cosine_maxes.into_iter().max(),
        tau,
        reasons,
    })
}

#[cfg(test)]
mod tests {
    use super::super::diagnostics::{ProbeMatrixVariantDiagnostic, guard_zero_hit_reason};
    use super::*;

    fn variant(
        variant_id: usize,
        prefilter_in: Option<usize>,
        prefilter_out: Option<usize>,
        pre_guard: Option<usize>,
        post_guard: Option<usize>,
    ) -> ProbeMatrixVariantDiagnostic {
        ProbeMatrixVariantDiagnostic {
            variant_id,
            query_text_sha256: format!("sha-{variant_id}"),
            guard_prefilter_input_count: prefilter_in,
            guard_prefilter_output_count: prefilter_out,
            guard_prefilter_filtered_count: match (prefilter_in, prefilter_out) {
                (Some(a), Some(b)) => Some(a.saturating_sub(b)),
                _ => None,
            },
            guard_prefilter_elapsed_ms: None,
            hit_hydration_candidate_count: None,
            hit_hydration_doc_count: None,
            hit_hydration_elapsed_ms: None,
            per_hit_hydrate_start_count: 0,
            per_hit_hydrate_done_count: 0,
            pre_guard_hit_count: pre_guard,
            post_guard_hit_count: post_guard,
            guard_filtered_hit_count: match (pre_guard, post_guard) {
                (Some(a), Some(b)) => Some(a.saturating_sub(b)),
                _ => None,
            },
            guard_tau: Some("0.999000".to_string()),
            guard_best_cosine_min: Some("0.401000".to_string()),
            guard_best_cosine_max: Some("0.788000".to_string()),
            guard_missing_cosine_count: Some(0),
            guard_start_elapsed_ms: None,
            guard_done_elapsed_ms: None,
            search_cache_key_sha256: None,
            search_cache_lookup_count: 0,
            search_cache_hit_count: 0,
            search_cache_miss_count: 0,
            search_cache_hit_slot_count: 0,
            search_cache_miss_slot_count: 0,
            search_cache_reused_hit_count: None,
            search_cache_stored_hit_count: None,
            search_cache_store_elapsed_ms: None,
            search_done_elapsed_ms: None,
            last_search_phase: None,
            last_search_elapsed_ms: None,
            guard_zero_hit_reason: guard_zero_hit_reason(
                prefilter_in,
                prefilter_out,
                pre_guard,
                post_guard,
            ),
            slot_searches: Vec::new(),
        }
    }

    #[test]
    fn guard_filtered_all_reports_when_every_variant_rejected_retrieved_candidates() {
        let guards = vec![
            variant(0, Some(64), Some(0), Some(0), Some(0)),
            variant(1, Some(32), Some(0), Some(0), Some(0)),
        ];
        let summary = guard_filtered_all_summary(&guards).expect("guard filtered all");
        assert_eq!(summary.variant_count, 2);
        assert_eq!(summary.retrieved_candidate_count, 96);
        assert_eq!(summary.tau.as_deref(), Some("0.999000"));
        assert_eq!(
            summary.observed_best_cosine_max.as_deref(),
            Some("0.788000")
        );
        assert!(
            summary
                .reasons
                .contains("in_region_guard_prefilter_rejected_all_candidates")
        );
    }

    #[test]
    fn guard_filtered_all_none_when_a_variant_kept_candidates() {
        let guards = vec![
            variant(0, Some(64), Some(0), Some(0), Some(0)),
            variant(1, Some(32), Some(4), Some(4), Some(2)),
        ];
        assert_eq!(guard_filtered_all_summary(&guards), None);
    }

    #[test]
    fn guard_filtered_all_none_when_no_candidates_retrieved() {
        // Guard off, or search retrieved nothing: not a guard artifact.
        let guards = vec![variant(0, None, None, Some(0), Some(0))];
        assert_eq!(guard_filtered_all_summary(&guards), None);
    }
}
