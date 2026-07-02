//! The shared search query path: measure the query through the active text
//! lenses, recall per slot from the persisted indexes, fuse (RRF / weighted /
//! pipeline / single-lens), attach stored provenance, optionally apply the
//! in-region guard, then rank+truncate. Extracted from the CLI (#573) so the
//! CLI and `calyx-web-api` run the IDENTICAL path. Takes an already-opened
//! vault + panel state (the caller owns vault lifecycle); never resolves a CLI
//! home and never prints — failures are structured errors.

use std::collections::BTreeSet;
use std::path::Path;

use calyx_aster::vault::AsterVault;
use calyx_core::{SlotId, SlotVector};
use calyx_sextant::FusionContext;
use calyx_sextant::fusion;

use crate::engine_fusion::{stage1_slots, weights_for};
pub use crate::engine_measure::{measure_query_vectors, measure_query_vectors_with_slots};
use crate::engine_measure::{
    measure_query_vectors_with_slots_traced, no_indexable_query_vectors,
    no_indexable_stored_vectors,
};
use crate::engine_slot_cache::search_slots_with_cache;
pub use crate::engine_slot_cache::{SearchSlotCache, SearchSlotCacheDiagnostic};
pub use crate::engine_trace::SearchTraceEvent;
use crate::engine_trace::SearchTracer;
use crate::error::CliResult;
use crate::persisted::PersistedSearchIndexes;
use crate::provenance::attach_verified_provenance;

mod budget;
mod guard;
mod hydration;
mod support;
mod types;
pub use budget::SearchBudget;
#[cfg(test)]
use guard::prefilter_in_region_candidates;
use guard::{apply_in_region_guard_traced, prefilter_in_region_candidates_traced};
use hydration::hydrate_hit_docs_with_bounded_readbacks;
use support::{SearchReadSnapshot, is_stale_derived, renumber_and_truncate, vault_base_count_at};
#[cfg(test)]
use support::{apply_in_region_guard, cosine};
pub use types::{FusionChoice, GuardChoice, SearchFreshness, SearchOutcome};

/// In-region guard cosine threshold used when the caller does not supply a
/// calibrated tau. This is a deliberately strict near-duplicate threshold and
/// is NOT calibrated to any particular corpus; a caller running `in-region`
/// against real vault content should calibrate tau to the corpus's actual
/// in-region cosine distribution (issue #1088).
pub const DEFAULT_IN_REGION_GUARD_TAU: f32 = 0.999;

/// Test-only alias for the default tau, used by guard/support unit tests that
/// assert behavior at the default threshold.
#[cfg(test)]
const GUARD_TAU: f32 = DEFAULT_IN_REGION_GUARD_TAU;

/// Bounded MVCC reader lease for a whole search readback pass.
const SEARCH_READER_LEASE_MS: u64 = 300_000;

/// Resolve the in-region guard tau to apply. `guard = Off` ignores any tau
/// (returns the default, unused). For `in-region`, an operator-supplied tau
/// must be finite and in `(0.0, 1.0]` — cosine similarity is bounded by 1.0 and
/// a non-positive threshold accepts everything, defeating the guard. A missing
/// tau resolves to [`DEFAULT_IN_REGION_GUARD_TAU`]. Fail closed on bad input;
/// never silently clamp (issue #1088).
fn resolve_guard_tau(guard: GuardChoice, guard_tau: Option<f32>) -> CliResult<f32> {
    match guard_tau {
        None => Ok(DEFAULT_IN_REGION_GUARD_TAU),
        Some(_) if guard != GuardChoice::InRegion => Err(crate::error::SearchError::usage(
            "guard tau was supplied but guard mode is not in-region; pass --guard in-region or omit the tau",
        )),
        Some(tau) if !tau.is_finite() || tau <= 0.0 || tau > 1.0 => {
            Err(crate::error::SearchError::usage(format!(
                "in-region guard tau {tau} is out of range; supply a finite cosine threshold in (0.0, 1.0]"
            )))
        }
        Some(tau) => Ok(tau),
    }
}

/// Run the real search over `vault` (already opened) using its persisted
/// indexes at `vault_dir`. `state` is the loaded panel state (the query is
/// measured through its active text lenses). Returns ranked hits with stored
/// provenance. An empty/uningested vault yields an empty outcome (not an error);
/// a query with no indexable lens vectors, or stored vectors that don't match
/// the active query lenses, is a structured error (no silent empty result).
#[allow(clippy::too_many_arguments)]
pub fn search_outcome(
    vault: &AsterVault,
    state: &calyx_registry::VaultPanelState,
    vault_dir: &Path,
    query: &str,
    k: usize,
    fusion: FusionChoice,
    guard: GuardChoice,
    filter: Option<&str>,
    explain: bool,
) -> CliResult<SearchOutcome> {
    search_outcome_with_freshness(
        vault,
        state,
        vault_dir,
        query,
        k,
        fusion,
        guard,
        filter,
        explain,
        SearchFreshness::Fresh,
    )
}

/// Run search with an explicit freshness policy. `Fresh` refuses stale derived
/// indexes; `StaleOk` permits lag only while tagging every hit with the index
/// build seq and current Base snapshot seq.
#[allow(clippy::too_many_arguments)]
pub fn search_outcome_with_freshness(
    vault: &AsterVault,
    state: &calyx_registry::VaultPanelState,
    vault_dir: &Path,
    query: &str,
    k: usize,
    fusion: FusionChoice,
    guard: GuardChoice,
    filter: Option<&str>,
    explain: bool,
    freshness: SearchFreshness,
) -> CliResult<SearchOutcome> {
    search_outcome_with_slots(
        vault, state, vault_dir, query, k, fusion, guard, filter, explain, None, freshness,
    )
}

/// Slot-scoped variant of [`search_outcome`]. Normal search measures every
/// active text lens, but matrix/probe callers sometimes need a physically exact
/// subset: only those slots may be measured, searched, fused, and guarded.
#[allow(clippy::too_many_arguments)]
pub fn search_outcome_with_slots(
    vault: &AsterVault,
    state: &calyx_registry::VaultPanelState,
    vault_dir: &Path,
    query: &str,
    k: usize,
    fusion: FusionChoice,
    guard: GuardChoice,
    filter: Option<&str>,
    explain: bool,
    allowed_slots: Option<&BTreeSet<SlotId>>,
    freshness: SearchFreshness,
) -> CliResult<SearchOutcome> {
    search_outcome_with_slots_traced(
        vault,
        state,
        vault_dir,
        query,
        k,
        fusion,
        guard,
        filter,
        explain,
        allowed_slots,
        freshness,
        None,
    )
}

/// Slot-scoped search with optional structured phase events.
#[allow(clippy::too_many_arguments)]
pub fn search_outcome_with_slots_traced(
    vault: &AsterVault,
    state: &calyx_registry::VaultPanelState,
    vault_dir: &Path,
    query: &str,
    k: usize,
    fusion: FusionChoice,
    guard: GuardChoice,
    filter: Option<&str>,
    explain: bool,
    allowed_slots: Option<&BTreeSet<SlotId>>,
    freshness: SearchFreshness,
    trace_sink: Option<&mut dyn FnMut(SearchTraceEvent)>,
) -> CliResult<SearchOutcome> {
    let mut trace = SearchTracer::new(trace_sink);
    let query_vectors =
        measure_query_vectors_with_slots_traced(state, query, allowed_slots, Some(&mut trace))?;
    search_outcome_with_measured_slots(
        vault,
        vault_dir,
        &query_vectors,
        k,
        fusion,
        guard,
        None,
        filter,
        explain,
        allowed_slots,
        freshness,
        SearchBudget::disabled(),
        None,
        Some(&mut trace),
    )
}

/// Run search with query vectors measured by the caller. This is used by warm
/// resident-service callers so query embedding does not cold-load GPU runtimes
/// inside the search process.
#[allow(clippy::too_many_arguments)]
pub fn search_outcome_with_query_vectors(
    vault: &AsterVault,
    vault_dir: &Path,
    query_vectors: &[(SlotId, SlotVector)],
    k: usize,
    fusion: FusionChoice,
    guard: GuardChoice,
    filter: Option<&str>,
    explain: bool,
    trace_sink: Option<&mut dyn FnMut(SearchTraceEvent)>,
) -> CliResult<SearchOutcome> {
    search_outcome_with_query_vectors_freshness(
        vault,
        vault_dir,
        query_vectors,
        k,
        fusion,
        guard,
        filter,
        explain,
        SearchFreshness::Fresh,
        SearchBudget::disabled(),
        trace_sink,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn search_outcome_with_query_vectors_freshness(
    vault: &AsterVault,
    vault_dir: &Path,
    query_vectors: &[(SlotId, SlotVector)],
    k: usize,
    fusion: FusionChoice,
    guard: GuardChoice,
    filter: Option<&str>,
    explain: bool,
    freshness: SearchFreshness,
    budget: SearchBudget<'_>,
    trace_sink: Option<&mut dyn FnMut(SearchTraceEvent)>,
) -> CliResult<SearchOutcome> {
    search_outcome_with_query_vectors_freshness_cached(
        vault,
        vault_dir,
        query_vectors,
        k,
        fusion,
        guard,
        None,
        filter,
        explain,
        freshness,
        budget,
        None,
        trace_sink,
    )
}

/// As [`search_outcome_with_query_vectors_freshness`] but with a per-run slot
/// cache and an optional calibrated in-region guard tau. `guard_tau = None`
/// uses [`DEFAULT_IN_REGION_GUARD_TAU`]; `Some(tau)` requires a finite value in
/// `(0.0, 1.0]` and is used verbatim as the in-region cosine threshold (#1088).
#[allow(clippy::too_many_arguments)]
pub fn search_outcome_with_query_vectors_freshness_cached(
    vault: &AsterVault,
    vault_dir: &Path,
    query_vectors: &[(SlotId, SlotVector)],
    k: usize,
    fusion: FusionChoice,
    guard: GuardChoice,
    guard_tau: Option<f32>,
    filter: Option<&str>,
    explain: bool,
    freshness: SearchFreshness,
    budget: SearchBudget<'_>,
    slot_cache: Option<&mut SearchSlotCache>,
    trace_sink: Option<&mut dyn FnMut(SearchTraceEvent)>,
) -> CliResult<SearchOutcome> {
    let allowed_slots = query_vectors
        .iter()
        .map(|(slot, _)| *slot)
        .collect::<BTreeSet<_>>();
    let mut trace = SearchTracer::new(trace_sink);
    search_outcome_with_measured_slots(
        vault,
        vault_dir,
        query_vectors,
        k,
        fusion,
        guard,
        guard_tau,
        filter,
        explain,
        Some(&allowed_slots),
        freshness,
        budget,
        slot_cache,
        Some(&mut trace),
    )
}

#[allow(clippy::too_many_arguments)]
fn search_outcome_with_measured_slots(
    vault: &AsterVault,
    vault_dir: &Path,
    query_vectors: &[(SlotId, SlotVector)],
    k: usize,
    fusion: FusionChoice,
    guard: GuardChoice,
    guard_tau: Option<f32>,
    filter: Option<&str>,
    explain: bool,
    allowed_slots: Option<&BTreeSet<SlotId>>,
    freshness: SearchFreshness,
    mut budget: SearchBudget<'_>,
    slot_cache: Option<&mut SearchSlotCache>,
    trace: Option<&mut SearchTracer<'_>>,
) -> CliResult<SearchOutcome> {
    let guard_tau = resolve_guard_tau(guard, guard_tau)?;
    let mut noop_trace;
    let trace = match trace {
        Some(trace) => trace,
        None => {
            noop_trace = SearchTracer::new(None);
            &mut noop_trace
        }
    };
    budget.check("search_start", 0)?;
    trace.emit("filters.parse.start", None, None);
    let filters = crate::filters::parse(filter)?;
    trace.emit("filters.parse.done", None, None);
    trace.emit_detail(
        "indexes.open.start",
        None,
        None,
        Some(vault_dir.display().to_string()),
    );
    let indexes = match PersistedSearchIndexes::open(vault_dir) {
        Ok(indexes) => indexes,
        Err(error) if is_stale_derived(&error) => {
            let read = SearchReadSnapshot::pin(vault);
            if vault_base_count_at(vault, read.snapshot())? == 0 {
                return Ok(SearchOutcome::empty());
            }
            return Err(error);
        }
        Err(error) => return Err(error),
    };
    trace.emit(
        "indexes.open.done",
        None,
        Some(indexes.max_len_for_slots(allowed_slots)),
    );
    if indexes.max_len_for_slots(allowed_slots) == 0 {
        trace.emit("indexes.empty", None, None);
        return Ok(SearchOutcome::empty());
    }
    trace.emit("indexes.ensure_bounded.start", None, None);
    indexes.ensure_search_bounded_for_slots(allowed_slots)?;
    trace.emit("indexes.ensure_bounded.done", None, None);
    if query_vectors.is_empty() {
        trace.emit("query_vectors.empty", None, None);
        return Err(no_indexable_query_vectors().into());
    }
    trace.emit("filter_candidates.start", None, None);
    let filter_candidates = indexes.filter_candidates(&filters)?;
    trace.emit(
        "filter_candidates.done",
        None,
        filter_candidates.as_ref().map(BTreeSet::len),
    );
    if filter_candidates.as_ref().is_some_and(|ids| ids.is_empty()) {
        trace.emit("filter_candidates.empty", None, Some(0));
        return Ok(SearchOutcome::empty());
    }
    let search_k = filter_candidates
        .as_ref()
        .map(|ids| ids.len())
        .unwrap_or_else(|| k.max(64));
    trace.emit_detail(
        "search_slots.start",
        None,
        Some(query_vectors.len()),
        Some(format!("search_k={search_k}")),
    );
    budget.check("before_search_slots", query_vectors.len())?;
    let per_slot = search_slots_with_cache(
        &indexes,
        vault_dir,
        query_vectors,
        search_k,
        guard,
        freshness,
        allowed_slots,
        filter_candidates.as_ref(),
        slot_cache,
        trace,
    )?;
    let searched_hits = per_slot.values().map(Vec::len).sum();
    budget.check("after_search_slots", searched_hits)?;
    trace.emit("search_slots.done", None, Some(per_slot.len()));
    let slots = per_slot.keys().copied().collect::<Vec<_>>();
    if slots.is_empty() {
        trace.emit("search_slots.empty", None, None);
        return Err(no_indexable_stored_vectors().into());
    }
    let strategy = fusion.to_strategy(&slots)?;
    let context = FusionContext {
        k: k.max(64),
        explain,
        strategy: strategy.clone(),
        weights: weights_for(&strategy, &slots),
        stage1_slots: stage1_slots(&strategy, query_vectors, &slots),
    };
    trace.emit_detail(
        "fusion.start",
        None,
        Some(per_slot.values().map(Vec::len).sum()),
        Some(format!("{strategy:?}")),
    );
    let mut hits = fusion::fuse(&per_slot, &context);
    trace.emit("fusion.done", None, Some(hits.len()));
    if guard != GuardChoice::InRegion {
        trace.emit("fusion.truncate.start", None, Some(hits.len()));
        renumber_and_truncate(&mut hits, k);
        trace.emit("fusion.truncate.done", None, Some(hits.len()));
    }
    let hydrate_hit_slots = guard == GuardChoice::InRegion;
    if guard == GuardChoice::InRegion {
        let before = hits.len();
        trace.emit("guard.prefilter.start", None, Some(before));
        budget.check("before_in_region_prefilter", before)?;
        hits = prefilter_in_region_candidates_traced(hits, query_vectors, guard_tau, trace);
        budget.check("after_in_region_prefilter", hits.len())?;
        trace.emit_detail(
            "guard.prefilter.done",
            None,
            Some(hits.len()),
            Some(format!(
                "filtered={} tau={guard_tau:.6}",
                before.saturating_sub(hits.len())
            )),
        );
    }
    trace.emit_detail(
        "hit_docs.hydrate.start",
        None,
        Some(hits.len()),
        Some(format!("hydrate_slots={hydrate_hit_slots}")),
    );
    budget.check("before_hit_hydration", hits.len())?;
    let (hit_docs, freshness_tag) = hydrate_hit_docs_with_bounded_readbacks(
        vault,
        &indexes,
        &hits,
        freshness,
        hydrate_hit_slots,
        trace,
        &mut budget,
    )?;
    budget.check("after_hit_hydration", hit_docs.len())?;
    trace.emit("hit_docs.hydrate.done", None, Some(hit_docs.len()));
    trace.emit("provenance.attach.start", None, Some(hits.len()));
    attach_verified_provenance(&mut hits, &hit_docs, vault_dir, freshness_tag)?;
    trace.emit("provenance.attach.done", None, Some(hits.len()));
    let applied_guard_tau = if guard == GuardChoice::InRegion {
        trace.emit("guard.in_region.start", None, Some(hits.len()));
        budget.check("before_in_region_guard", hits.len())?;
        hits = apply_in_region_guard_traced(hits, &hit_docs, query_vectors, guard_tau, trace);
        budget.check("after_in_region_guard", hits.len())?;
        trace.emit("guard.in_region.done", None, Some(hits.len()));
        renumber_and_truncate(&mut hits, k);
        Some(guard_tau)
    } else {
        None
    };
    trace.emit("search.done", None, Some(hits.len()));
    Ok(SearchOutcome {
        hits,
        guard_tau: applied_guard_tau,
        docs: hit_docs,
    })
}

#[cfg(test)]
mod tests;
