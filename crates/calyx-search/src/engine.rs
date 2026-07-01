//! The shared search query path: measure the query through the active text
//! lenses, recall per slot from the persisted indexes, fuse (RRF / weighted /
//! pipeline / single-lens), attach stored provenance, optionally apply the
//! in-region guard, then rank+truncate. Extracted from the CLI (#573) so the
//! CLI and `calyx-web-api` run the IDENTICAL path. Takes an already-opened
//! vault + panel state (the caller owns vault lifecycle); never resolves a CLI
//! home and never prints — failures are structured errors.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use calyx_aster::vault::AsterVault;
use calyx_core::{Constellation, CxId, SlotId, SlotVector};
use calyx_sextant::fusion;
use calyx_sextant::{FusionContext, FusionStrategy, Hit, RrfProfile};

use crate::engine_fusion::{stage1_slots, weights_for};
pub use crate::engine_measure::{measure_query_vectors, measure_query_vectors_with_slots};
use crate::engine_measure::{
    measure_query_vectors_with_slots_traced, no_indexable_query_vectors,
    no_indexable_stored_vectors, slot_vector_shape,
};
pub use crate::engine_trace::SearchTraceEvent;
use crate::engine_trace::SearchTracer;
use crate::error::CliResult;
use crate::persisted::PersistedSearchIndexes;
use crate::provenance::{attach_verified_provenance, hit_docs_at};

mod support;
#[cfg(test)]
use support::cosine;
use support::{
    SearchReadSnapshot, apply_in_region_guard, index_freshness_tag, is_stale_derived,
    renumber_and_truncate, vault_base_count_at,
};

/// In-region guard cosine threshold (mirrors the CLI default).
const GUARD_TAU: f32 = 0.999;

/// Bounded MVCC reader lease for a whole search readback pass.
const SEARCH_READER_LEASE_MS: u64 = 300_000;

/// Fusion strategy choice (transport-agnostic; the CLI flag parser and the HTTP
/// request both map onto this, then it resolves to a concrete `FusionStrategy`
/// against the live slot set).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FusionChoice {
    Rrf,
    WeightedRrf,
    WeightedRrfProfile(RrfProfile),
    SingleLens,
    SingleLensSlot(SlotId),
    KernelFirst,
    Pipeline,
}

impl FusionChoice {
    pub fn to_strategy(self, slots: &[SlotId]) -> CliResult<FusionStrategy> {
        match self {
            Self::Rrf => Ok(FusionStrategy::Rrf),
            Self::WeightedRrf => Ok(FusionStrategy::WeightedRrf {
                profile: RrfProfile::General,
            }),
            Self::WeightedRrfProfile(profile) => Ok(FusionStrategy::WeightedRrf { profile }),
            Self::SingleLens => slots
                .first()
                .copied()
                .map(|slot| FusionStrategy::SingleLens { slot })
                .ok_or_else(|| {
                    crate::error::SearchError::usage("single-lens search has no active lens slot")
                }),
            Self::SingleLensSlot(slot) => {
                if slots.contains(&slot) {
                    Ok(FusionStrategy::SingleLens { slot })
                } else {
                    Err(crate::error::SearchError::usage(format!(
                        "single-lens search requested slot {slot}, but the slot has no active persisted search results"
                    )))
                }
            }
            Self::KernelFirst => Ok(FusionStrategy::WeightedRrf {
                profile: RrfProfile::Kernel,
            }),
            Self::Pipeline => Ok(FusionStrategy::Pipeline),
        }
    }
}

/// Guard choice for a search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardChoice {
    Off,
    InRegion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchFreshness {
    Fresh,
    StaleOk,
}

/// The result of a search: ranked hits (each carrying score + stored
/// provenance) and the guard tau actually applied (if any).
pub struct SearchOutcome {
    pub hits: Vec<Hit>,
    pub guard_tau: Option<f32>,
    pub docs: BTreeMap<CxId, Constellation>,
}

impl SearchOutcome {
    fn empty() -> Self {
        Self {
            hits: Vec::new(),
            guard_tau: None,
            docs: BTreeMap::new(),
        }
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
        filter,
        explain,
        allowed_slots,
        freshness,
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
        filter,
        explain,
        Some(&allowed_slots),
        freshness,
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
    filter: Option<&str>,
    explain: bool,
    allowed_slots: Option<&BTreeSet<SlotId>>,
    freshness: SearchFreshness,
    trace: Option<&mut SearchTracer<'_>>,
) -> CliResult<SearchOutcome> {
    let mut noop_trace;
    let trace = match trace {
        Some(trace) => trace,
        None => {
            noop_trace = SearchTracer::new(None);
            &mut noop_trace
        }
    };
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
    let per_slot = search_slots(
        &indexes,
        query_vectors,
        search_k,
        filter_candidates.as_ref(),
        trace,
    )?;
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
    trace.emit_detail(
        "hit_docs.hydrate.start",
        None,
        Some(hits.len()),
        Some(format!("hydrate_slots={hydrate_hit_slots}")),
    );
    let (hit_docs, freshness_tag) = hydrate_hit_docs_with_bounded_readbacks(
        vault,
        &indexes,
        &hits,
        freshness,
        hydrate_hit_slots,
        trace,
    )?;
    trace.emit("hit_docs.hydrate.done", None, Some(hit_docs.len()));
    trace.emit("provenance.attach.start", None, Some(hits.len()));
    attach_verified_provenance(&mut hits, &hit_docs, vault_dir, freshness_tag)?;
    trace.emit("provenance.attach.done", None, Some(hits.len()));
    let guard_tau = if guard == GuardChoice::InRegion {
        trace.emit("guard.in_region.start", None, Some(hits.len()));
        hits = apply_in_region_guard(hits, &hit_docs, query_vectors);
        trace.emit("guard.in_region.done", None, Some(hits.len()));
        renumber_and_truncate(&mut hits, k);
        Some(GUARD_TAU)
    } else {
        None
    };
    trace.emit("search.done", None, Some(hits.len()));
    Ok(SearchOutcome {
        hits,
        guard_tau,
        docs: hit_docs,
    })
}

fn hydrate_hit_docs_with_bounded_readbacks(
    vault: &AsterVault,
    indexes: &PersistedSearchIndexes,
    hits: &[Hit],
    freshness: SearchFreshness,
    hydrate_hit_slots: bool,
    trace: &mut SearchTracer<'_>,
) -> CliResult<(BTreeMap<CxId, Constellation>, calyx_sextant::FreshnessTag)> {
    if hits.is_empty() {
        let read = pin_search_readback(vault, trace, "empty_hit_set", None, 0);
        let freshness_tag = verify_index_freshness(indexes, read.seq(), freshness, trace)?;
        return Ok((BTreeMap::new(), freshness_tag));
    }

    let mut docs = BTreeMap::new();
    let mut expected_seq = None;
    let mut freshness_tag = None;
    for (hit_index, hit) in hits.iter().enumerate() {
        let read = pin_search_readback(
            vault,
            trace,
            "hit_doc_hydration",
            Some(hit.cx_id),
            hit_index + 1,
        );
        if let Some(seq) = expected_seq {
            if read.seq() != seq {
                return Err(calyx_core::CalyxError::stale_derived(format!(
                    "vault advanced during search hit hydration: initial pinned seq {seq}, \
                     hit_index={hit_index}, cx_id={}, new pinned seq {}, search index base seq {}; \
                     refusing to mix index hits with a newer Base snapshot",
                    hit.cx_id,
                    read.seq(),
                    indexes.base_seq()
                ))
                .into());
            }
        } else {
            expected_seq = Some(read.seq());
        }
        let tag = verify_index_freshness(indexes, read.seq(), freshness, trace)?;
        if freshness_tag.is_none() {
            freshness_tag = Some(tag);
        }
        trace.emit_detail(
            "hit_doc.hydrate.start",
            None,
            Some(hit_index + 1),
            Some(format!(
                "cx_id={} snapshot_seq={} hydrate_slots={hydrate_hit_slots}",
                hit.cx_id,
                read.seq()
            )),
        );
        let one = hit_docs_at(
            vault,
            std::slice::from_ref(hit),
            read.snapshot(),
            hydrate_hit_slots,
        )
        .map_err(|error| contextualize_hit_hydration_error(error, hit, hit_index, &read))?;
        docs.extend(one);
        trace.emit_detail(
            "hit_doc.hydrate.done",
            None,
            Some(hit_index + 1),
            Some(format!("cx_id={} snapshot_seq={}", hit.cx_id, read.seq())),
        );
    }

    let tag = freshness_tag.ok_or_else(|| {
        calyx_core::CalyxError::stale_derived(
            "search hydration produced hits but no verified freshness tag",
        )
    })?;
    Ok((docs, tag))
}

fn pin_search_readback<'a>(
    vault: &'a AsterVault,
    trace: &mut SearchTracer<'_>,
    phase: &'static str,
    cx_id: Option<CxId>,
    hit_ordinal: usize,
) -> SearchReadSnapshot<'a> {
    trace.emit_detail(
        "snapshot.pin.start",
        None,
        if hit_ordinal == 0 {
            None
        } else {
            Some(hit_ordinal)
        },
        Some(snapshot_detail(phase, cx_id, None)),
    );
    let read = SearchReadSnapshot::pin(vault);
    trace.emit_detail(
        "snapshot.pin.done",
        None,
        Some(read.seq() as usize),
        Some(snapshot_detail(
            phase,
            cx_id,
            Some(format!(
                "lease_id={} max_age_ms={} expires_at={}",
                read.lease_id(),
                read.lease_max_age_ms(),
                read.lease_expires_at()
            )),
        )),
    );
    read
}

fn verify_index_freshness(
    indexes: &PersistedSearchIndexes,
    pinned_seq: u64,
    freshness: SearchFreshness,
    trace: &mut SearchTracer<'_>,
) -> CliResult<calyx_sextant::FreshnessTag> {
    trace.emit_detail(
        "indexes.freshness.start",
        None,
        None,
        Some(format!(
            "pinned_seq={pinned_seq} index_base_seq={}",
            indexes.base_seq()
        )),
    );
    let freshness_tag = index_freshness_tag(indexes, pinned_seq, freshness)?;
    trace.emit_detail(
        "indexes.freshness.done",
        None,
        None,
        Some(format!(
            "{} pinned_seq={pinned_seq} index_base_seq={}",
            freshness_tag.policy,
            indexes.base_seq()
        )),
    );
    Ok(freshness_tag)
}

fn contextualize_hit_hydration_error(
    error: crate::error::SearchError,
    hit: &Hit,
    hit_index: usize,
    read: &SearchReadSnapshot<'_>,
) -> crate::error::SearchError {
    if error.code() != "CALYX_READER_LEASE_EXPIRED" {
        return error;
    }
    calyx_core::CalyxError::reader_lease_expired(format!(
        "reader lease expired while hydrating search hit: hit_index={hit_index}, cx_id={}, \
         snapshot_seq={}, lease_id={}, max_age_ms={}, expires_at={}",
        hit.cx_id,
        read.seq(),
        read.lease_id(),
        read.lease_max_age_ms(),
        read.lease_expires_at()
    ))
    .into()
}

fn snapshot_detail(phase: &str, cx_id: Option<CxId>, extra: Option<String>) -> String {
    match (cx_id, extra) {
        (Some(cx_id), Some(extra)) => format!("phase={phase} cx_id={cx_id} {extra}"),
        (Some(cx_id), None) => format!("phase={phase} cx_id={cx_id}"),
        (None, Some(extra)) => format!("phase={phase} {extra}"),
        (None, None) => format!("phase={phase}"),
    }
}

fn search_slots(
    indexes: &PersistedSearchIndexes,
    query_vectors: &[(SlotId, SlotVector)],
    k: usize,
    filter_candidates: Option<&BTreeSet<CxId>>,
    trace: &mut SearchTracer<'_>,
) -> CliResult<BTreeMap<SlotId, Vec<calyx_sextant::IndexSearchHit>>> {
    let mut out = BTreeMap::new();
    for (slot, query) in query_vectors {
        trace.emit_detail(
            "search_slot.start",
            Some(*slot),
            Some(k),
            Some(slot_vector_shape(query)),
        );
        let hits = if let Some(candidates) = filter_candidates {
            indexes.search_filtered(*slot, query, k, candidates)?
        } else {
            indexes.search(*slot, query, k)?
        };
        trace.emit("search_slot.done", Some(*slot), Some(hits.len()));
        if !hits.is_empty() {
            out.insert(*slot, hits);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
