//! Fail-closed search performance budgets for probe-matrix (issue #1102).
//!
//! `--search-miss-budget-ms` bounds every variant whose slot-result cache
//! missed (full scoring ran); `--search-hit-budget-ms` bounds every variant
//! served from the slot-result cache. A breach fails the run with a
//! structured error after the matrix artifact is persisted, so FSV can read
//! the regressing timings from JSON.

use calyx_core::CalyxError;

use super::ProbeMatrixArgs;
use super::diagnostics::ProbeMatrixVariantDiagnostic;
use crate::error::{CliError, CliResult};

pub(super) const PERF_BUDGET_CODE: &str = "CALYX_PROBE_MATRIX_PERF_BUDGET_EXCEEDED";
const PERF_BUDGET_REMEDIATION: &str = "inspect diagnostics.variant_guard_counts[].slot_searches and search_done_elapsed_ms in the persisted matrix artifact to find the regressing slot, then fix the regression or recalibrate the budget flags";

pub(super) fn enforce_search_perf_budgets(
    guards: &[ProbeMatrixVariantDiagnostic],
    args: &ProbeMatrixArgs,
) -> CliResult {
    let miss_budget_ms = args.search_miss_budget_ms;
    let hit_budget_ms = args.search_hit_budget_ms;
    if miss_budget_ms.is_none() && hit_budget_ms.is_none() {
        return Ok(());
    }
    let mut breaches = Vec::new();
    for guard in guards {
        let is_miss = guard.search_cache_miss_count > 0;
        let budget = if is_miss {
            miss_budget_ms
        } else {
            hit_budget_ms
        };
        let Some(budget) = budget else {
            continue;
        };
        let Some(elapsed_ms) = guard.search_done_elapsed_ms else {
            return Err(perf_budget_error(format!(
                "variant {} has no search_done_elapsed_ms in diagnostics; cannot verify the search perf budget",
                guard.variant_id
            )));
        };
        if elapsed_ms > u128::from(budget) {
            breaches.push(format!(
                "variant {} ({}) search_done_elapsed_ms={elapsed_ms} > budget {budget}ms",
                guard.variant_id,
                if is_miss { "cache miss" } else { "cache hit" },
            ));
        }
    }
    if breaches.is_empty() {
        return Ok(());
    }
    Err(perf_budget_error(format!(
        "search perf budget exceeded for {} of {} variants: {}",
        breaches.len(),
        guards.len(),
        breaches.join("; ")
    )))
}

fn perf_budget_error(message: impl Into<String>) -> CliError {
    CalyxError {
        code: PERF_BUDGET_CODE,
        message: message.into(),
        remediation: PERF_BUDGET_REMEDIATION,
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::super::diagnostics::guard_zero_hit_reason;
    use super::*;

    fn budgets(miss: Option<u64>, hit: Option<u64>) -> ProbeMatrixArgs {
        ProbeMatrixArgs {
            search_miss_budget_ms: miss,
            search_hit_budget_ms: hit,
            ..ProbeMatrixArgs::default()
        }
    }

    fn guard(
        variant_id: usize,
        miss_count: usize,
        search_done_elapsed_ms: Option<u128>,
    ) -> ProbeMatrixVariantDiagnostic {
        ProbeMatrixVariantDiagnostic {
            variant_id,
            query_text_sha256: format!("sha-{variant_id}"),
            guard_prefilter_input_count: None,
            guard_prefilter_output_count: None,
            guard_prefilter_filtered_count: None,
            guard_prefilter_elapsed_ms: None,
            hit_hydration_candidate_count: None,
            hit_hydration_doc_count: None,
            hit_hydration_elapsed_ms: None,
            per_hit_hydrate_start_count: 0,
            per_hit_hydrate_done_count: 0,
            pre_guard_hit_count: None,
            post_guard_hit_count: None,
            guard_filtered_hit_count: None,
            guard_tau: None,
            guard_best_cosine_min: None,
            guard_best_cosine_max: None,
            guard_missing_cosine_count: None,
            guard_start_elapsed_ms: None,
            guard_done_elapsed_ms: None,
            search_cache_key_sha256: None,
            search_cache_lookup_count: 1,
            search_cache_hit_count: usize::from(miss_count == 0),
            search_cache_miss_count: miss_count,
            search_cache_hit_slot_count: 0,
            search_cache_miss_slot_count: 0,
            search_cache_reused_hit_count: None,
            search_cache_stored_hit_count: None,
            search_cache_store_elapsed_ms: None,
            search_done_elapsed_ms,
            last_search_phase: None,
            last_search_elapsed_ms: None,
            guard_zero_hit_reason: guard_zero_hit_reason(None, None, None, None),
            slot_searches: Vec::new(),
        }
    }

    #[test]
    fn budgets_pass_when_within_limits() {
        let guards = vec![guard(0, 13, Some(9_000)), guard(1, 0, Some(400))];
        enforce_search_perf_budgets(&guards, &budgets(Some(10_000), Some(1_000))).unwrap();
    }

    #[test]
    fn miss_budget_breach_fails_closed_with_variant_evidence() {
        let guards = vec![guard(0, 13, Some(42_951)), guard(1, 0, Some(400))];
        let err =
            enforce_search_perf_budgets(&guards, &budgets(Some(10_000), Some(1_000))).unwrap_err();
        assert_eq!(err.code(), PERF_BUDGET_CODE);
        assert!(err.message().contains("variant 0 (cache miss)"));
        assert!(err.message().contains("42951 > budget 10000ms"));
    }

    #[test]
    fn hit_budget_breach_fails_closed() {
        let guards = vec![guard(1, 0, Some(5_400))];
        let err = enforce_search_perf_budgets(&guards, &budgets(None, Some(1_000))).unwrap_err();
        assert_eq!(err.code(), PERF_BUDGET_CODE);
        assert!(err.message().contains("variant 1 (cache hit)"));
    }

    #[test]
    fn missing_timing_fails_closed_when_budget_is_set() {
        let guards = vec![guard(0, 13, None)];
        let err = enforce_search_perf_budgets(&guards, &budgets(Some(10_000), None)).unwrap_err();
        assert_eq!(err.code(), PERF_BUDGET_CODE);
        assert!(err.message().contains("cannot verify"));
    }

    #[test]
    fn no_budgets_configured_is_a_no_op() {
        let guards = vec![guard(0, 13, None)];
        enforce_search_perf_budgets(&guards, &budgets(None, None)).unwrap();
    }
}
