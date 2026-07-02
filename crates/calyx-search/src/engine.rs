//! The shared search query path: measure the query through the active text
//! lenses, recall per slot from the persisted indexes, fuse (RRF / weighted /
//! pipeline / single-lens), attach stored provenance, optionally apply the
//! in-region guard, then rank+truncate. Extracted from the CLI (#573) so the
//! CLI and `calyx-web-api` run the IDENTICAL path. Takes an already-opened
//! vault + panel state (the caller owns vault lifecycle); never resolves a CLI
//! home and never prints — failures are structured errors.

use std::collections::BTreeSet;
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, SlotId, SlotVector, VaultStore};
use calyx_sextant::FusionContext;
use calyx_sextant::{apply_in_region_guard_to_hits, fusion};
use calyx_ward::GuardProfile;

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
mod hydration_cache;
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

/// Historical flat in-region cosine threshold. Since #1094 this is NEVER
/// applied implicitly: `--guard in-region` without an operator tau loads the
/// calibrated Ward guard profile instead (fail-closed
/// `CALYX_GUARD_PROVISIONAL` when absent). The constant remains only as the
/// documented strict near-duplicate reference value for operators choosing an
/// explicit flat tau (issue #1088) and for the flat-path unit tests.
pub const DEFAULT_IN_REGION_GUARD_TAU: f32 = 0.999;

/// Test-only alias for the default tau, used by guard/support unit tests that
/// assert behavior at the default threshold.
#[cfg(test)]
const GUARD_TAU: f32 = DEFAULT_IN_REGION_GUARD_TAU;

/// Bounded MVCC reader lease for a whole search readback pass.
const SEARCH_READER_LEASE_MS: u64 = 300_000;

/// How the in-region guard will actually be enforced for this search (#1094).
///
/// `--guard in-region` WITHOUT an operator tau is profile-backed: the
/// calibrated Ward [`GuardProfile`] is loaded from the Guard CF and applied
/// with per-slot conformal taus (MCP parity). An explicit operator tau keeps
/// the flat single-tau cosine gate with its provenance recorded in the
/// outcome (#1088 probe-matrix calibration path). There is no silent default
/// tau anywhere: a missing profile fails closed with
/// `CALYX_GUARD_PROVISIONAL`.
enum ResolvedGuard {
    Off,
    OperatorTau(f32),
    Profile(Box<GuardProfile>),
}

/// Resolve the guard mode. `guard = Off` rejects a supplied tau (usage
/// error). For `in-region`, an operator-supplied tau must be finite and in
/// `(0.0, 1.0]` — cosine similarity is bounded by 1.0 and a non-positive
/// threshold accepts everything, defeating the guard. Without a tau the
/// calibrated Ward profile is loaded from the Guard CF and fails closed when
/// absent/uncalibrated/panel-mismatched. Never silently clamp or default
/// (issues #1088, #1094).
fn resolve_guard(
    vault: &AsterVault,
    guard: GuardChoice,
    guard_tau: Option<f32>,
    guard_panel_version: Option<u64>,
) -> CliResult<ResolvedGuard> {
    match (guard, guard_tau) {
        (GuardChoice::Off, None) => Ok(ResolvedGuard::Off),
        (GuardChoice::Off, Some(_)) => Err(crate::error::SearchError::usage(
            "guard tau was supplied but guard mode is not in-region; pass --guard in-region or omit the tau",
        )),
        (GuardChoice::InRegion, Some(tau)) if !tau.is_finite() || tau <= 0.0 || tau > 1.0 => {
            Err(crate::error::SearchError::usage(format!(
                "in-region guard tau {tau} is out of range; supply a finite cosine threshold in (0.0, 1.0]"
            )))
        }
        (GuardChoice::InRegion, Some(tau)) => Ok(ResolvedGuard::OperatorTau(tau)),
        (GuardChoice::InRegion, None) => Ok(ResolvedGuard::Profile(Box::new(
            load_default_guard_profile(vault, guard_panel_version)?,
        ))),
    }
}

/// Load the calibrated default Ward guard profile from the Guard CF, failing
/// closed with `CALYX_GUARD_PROVISIONAL` when it is missing, undecodable,
/// uncalibrated, or calibrated for a different panel version — the same gate
/// the MCP search path enforces (`calyx-mcp/src/tools/search/engine.rs`).
///
/// The caller must have opened the vault with [`ColumnFamily::Guard`]
/// selected: reading an unselected CF silently returns `None`, which would
/// masquerade as a missing profile (#1094).
fn load_default_guard_profile(
    vault: &AsterVault,
    guard_panel_version: Option<u64>,
) -> CliResult<GuardProfile> {
    let Some(bytes) = vault.read_cf_at(
        vault.snapshot(),
        ColumnFamily::Guard,
        DEFAULT_GUARD_PROFILE_KEY,
    )?
    else {
        return Err(CalyxError::guard_provisional(
            "in-region search requires a calibrated default guard profile (Guard CF key `profile\\0default`); run `calyx guard <vault> calibrate` or supply an explicit operator tau",
        )
        .into());
    };
    let profile: GuardProfile = serde_json::from_slice(&bytes).map_err(|error| {
        CalyxError::guard_provisional(format!("decode default guard profile: {error}"))
    })?;
    if let Some(panel_version) = guard_panel_version
        && profile.panel_version != panel_version
    {
        return Err(CalyxError::guard_provisional(format!(
            "guard profile panel_version {} does not match active panel {panel_version}; recalibrate the guard for the current panel",
            profile.panel_version
        ))
        .into());
    }
    if !profile.is_calibrated() {
        return Err(CalyxError::guard_provisional(
            "default guard profile is not calibrated; run `calyx guard <vault> calibrate`",
        )
        .into());
    }
    Ok(profile)
}

/// Guard CF key of the default calibrated profile (MCP parity).
pub const DEFAULT_GUARD_PROFILE_KEY: &[u8] = b"profile\0default";

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
        Some(u64::from(state.panel.version)),
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
        None,
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
    guard_panel_version: Option<u64>,
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
        guard_panel_version,
        filter,
        explain,
        freshness,
        budget,
        None,
        trace_sink,
    )
}

/// As [`search_outcome_with_query_vectors_freshness`] but with a per-run slot
/// cache and an optional operator-supplied in-region guard tau. `Some(tau)`
/// requires a finite value in `(0.0, 1.0]` and is used verbatim as the flat
/// in-region cosine threshold (#1088). `guard_tau = None` with
/// `guard = in-region` applies the calibrated Ward guard profile from the
/// Guard CF with per-slot conformal taus, failing closed with
/// `CALYX_GUARD_PROVISIONAL` when the profile is absent/uncalibrated or (when
/// `guard_panel_version` is supplied) calibrated for a different panel
/// version (#1094). There is no silent default tau.
#[allow(clippy::too_many_arguments)]
pub fn search_outcome_with_query_vectors_freshness_cached(
    vault: &AsterVault,
    vault_dir: &Path,
    query_vectors: &[(SlotId, SlotVector)],
    k: usize,
    fusion: FusionChoice,
    guard: GuardChoice,
    guard_tau: Option<f32>,
    guard_panel_version: Option<u64>,
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
        guard_panel_version,
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
    guard_panel_version: Option<u64>,
    filter: Option<&str>,
    explain: bool,
    allowed_slots: Option<&BTreeSet<SlotId>>,
    freshness: SearchFreshness,
    mut budget: SearchBudget<'_>,
    slot_cache: Option<&mut SearchSlotCache>,
    trace: Option<&mut SearchTracer<'_>>,
) -> CliResult<SearchOutcome> {
    // Resolve (and for profile mode, load + validate) the guard BEFORE any
    // expensive slot search: an uncalibrated vault must fail closed with
    // CALYX_GUARD_PROVISIONAL in milliseconds, not after seconds of recall
    // (the #1103 lesson applied to #1094).
    let resolved_guard = resolve_guard(vault, guard, guard_tau, guard_panel_version)?;
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
    if let ResolvedGuard::OperatorTau(tau) = resolved_guard {
        // The flat prefilter is an optimization bound to the operator's flat
        // tau; the profile path guards post-hydration with per-slot taus and
        // must see every fused candidate.
        let before = hits.len();
        trace.emit("guard.prefilter.start", None, Some(before));
        budget.check("before_in_region_prefilter", before)?;
        hits = prefilter_in_region_candidates_traced(hits, query_vectors, tau, trace);
        budget.check("after_in_region_prefilter", hits.len())?;
        trace.emit_detail(
            "guard.prefilter.done",
            None,
            Some(hits.len()),
            Some(format!(
                "filtered={} tau={tau:.6}",
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
        vault_dir,
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
    attach_verified_provenance(&mut hits, &hit_docs, vault_dir, freshness_tag, trace)?;
    trace.emit("provenance.attach.done", None, Some(hits.len()));
    let mut dropped_guard_hits = Vec::new();
    let applied_guard_tau = match &resolved_guard {
        ResolvedGuard::Off => None,
        ResolvedGuard::OperatorTau(tau) => {
            let tau = *tau;
            trace.emit("guard.in_region.start", None, Some(hits.len()));
            budget.check("before_in_region_guard", hits.len())?;
            hits = apply_in_region_guard_traced(hits, &hit_docs, query_vectors, tau, trace);
            budget.check("after_in_region_guard", hits.len())?;
            trace.emit("guard.in_region.done", None, Some(hits.len()));
            renumber_and_truncate(&mut hits, k);
            Some(tau)
        }
        ResolvedGuard::Profile(profile) => {
            let before = hits.len();
            trace.emit_detail(
                "guard.in_region.start",
                None,
                Some(before),
                Some(format!(
                    "mode=profile guard_id={} panel_version={} policy={:?} slots={}",
                    profile.guard_id,
                    profile.panel_version,
                    profile.policy,
                    profile.tau.len()
                )),
            );
            budget.check("before_in_region_guard", before)?;
            dropped_guard_hits = apply_in_region_guard_to_hits(
                &hit_docs,
                profile,
                query_vectors,
                &mut hits,
                true,
            )?;
            budget.check("after_in_region_guard", hits.len())?;
            for dropped in &dropped_guard_hits {
                trace.emit_detail(
                    "guard.in_region.dropped",
                    None,
                    None,
                    Some(format!(
                        "cx_id={} reason={:?}",
                        dropped.cx_id, dropped.reason
                    )),
                );
            }
            trace.emit_detail(
                "guard.in_region.done",
                None,
                Some(hits.len()),
                Some(format!(
                    "mode=profile dropped={}",
                    before.saturating_sub(hits.len())
                )),
            );
            if before > 0 && hits.is_empty() {
                return Err(CalyxError::guard_ood(format!(
                    "in-region guard blocked all {before} search candidates"
                ))
                .into());
            }
            renumber_and_truncate(&mut hits, k);
            // Per-slot calibrated taus: no single flat tau to report. The
            // hits carry their guard verdict evidence.
            None
        }
    };
    trace.emit("search.done", None, Some(hits.len()));
    Ok(SearchOutcome {
        hits,
        guard_tau: applied_guard_tau,
        docs: hit_docs,
        dropped_guard_hits,
    })
}

#[cfg(test)]
mod tests;
