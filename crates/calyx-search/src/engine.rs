//! The shared search query path: measure the query through the active text
//! lenses, recall per slot from the persisted indexes, fuse (RRF / weighted /
//! pipeline / single-lens), attach stored provenance, optionally apply the
//! in-region guard, then rank+truncate. Extracted from the CLI (#573) so the
//! CLI and `calyx-web-api` run the IDENTICAL path. Takes an already-opened
//! vault + panel state (the caller owns vault lifecycle); never resolves a CLI
//! home and never prints — failures are structured errors.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use calyx_aster::mvcc::{Freshness, Snapshot};
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
use crate::persisted::{PersistedSearchIndexes, load_docs_at};
use crate::provenance::{attach_verified_provenance, hit_docs_at};

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
    search_outcome_with_slots(
        vault, state, vault_dir, query, k, fusion, guard, filter, explain, None,
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
) -> CliResult<SearchOutcome> {
    let allowed_slots = query_vectors
        .iter()
        .map(|(slot, _)| *slot)
        .collect::<BTreeSet<_>>();
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
        None,
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
    trace.emit("snapshot.pin.start", None, None);
    let read = SearchReadSnapshot::pin(vault);
    trace.emit("snapshot.pin.done", None, Some(read.seq() as usize));
    trace.emit("indexes.ensure_fresh.start", None, None);
    indexes.ensure_fresh_at_snapshot(read.seq())?;
    trace.emit("indexes.ensure_fresh.done", None, None);
    let hydrate_hit_slots = guard == GuardChoice::InRegion;
    trace.emit_detail(
        "hit_docs.hydrate.start",
        None,
        Some(hits.len()),
        Some(format!("hydrate_slots={hydrate_hit_slots}")),
    );
    let hit_docs = hit_docs_at(vault, &hits, read.snapshot(), hydrate_hit_slots)?;
    trace.emit("hit_docs.hydrate.done", None, Some(hit_docs.len()));
    trace.emit("provenance.attach.start", None, Some(hits.len()));
    attach_verified_provenance(&mut hits, &hit_docs, vault_dir, read.seq())?;
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

struct SearchReadSnapshot<'a> {
    vault: &'a AsterVault,
    snapshot: Snapshot,
}

impl<'a> SearchReadSnapshot<'a> {
    fn pin(vault: &'a AsterVault) -> Self {
        Self {
            vault,
            snapshot: vault.pin_reader(Freshness::FreshDerived, SEARCH_READER_LEASE_MS),
        }
    }

    fn snapshot(&self) -> Snapshot {
        self.snapshot
    }

    fn seq(&self) -> u64 {
        self.snapshot.seq()
    }
}

impl Drop for SearchReadSnapshot<'_> {
    fn drop(&mut self) {
        let _ = self.vault.release_reader(self.snapshot.lease().id());
    }
}

fn is_stale_derived(error: &crate::error::SearchError) -> bool {
    matches!(error, crate::error::SearchError::Calyx(inner) if inner.code == "CALYX_STALE_DERIVED")
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

/// Keep only hits whose best per-lens cosine to the query meets the guard tau.
/// (The library filters silently; surfacing a per-hit "blocked" notice is a
/// presentation concern left to the caller.)
fn apply_in_region_guard(
    hits: Vec<Hit>,
    docs: &BTreeMap<CxId, Constellation>,
    query_vectors: &[(SlotId, SlotVector)],
) -> Vec<Hit> {
    hits.into_iter()
        .filter(|hit| {
            guard_cosine(hit, docs, query_vectors).is_some_and(|value| value >= GUARD_TAU)
        })
        .collect()
}

fn guard_cosine(
    hit: &Hit,
    docs: &BTreeMap<CxId, Constellation>,
    query_vectors: &[(SlotId, SlotVector)],
) -> Option<f32> {
    let cx = docs.get(&hit.cx_id)?;
    hit.per_lens
        .iter()
        .filter_map(|item| {
            let query = query_vectors
                .iter()
                .find(|(slot, _)| *slot == item.slot)?
                .1
                .as_dense()?;
            let doc = cx.slots.get(&item.slot)?.as_dense()?;
            cosine(query, doc)
        })
        .max_by(f32::total_cmp)
}

fn vault_base_count_at(vault: &AsterVault, snapshot: Snapshot) -> CliResult<usize> {
    Ok(load_docs_at(vault, snapshot)?.len())
}

fn renumber_and_truncate(hits: &mut Vec<Hit>, k: usize) {
    hits.truncate(k);
    for (idx, hit) in hits.iter_mut().enumerate() {
        hit.rank = idx + 1;
    }
}

fn cosine(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.len() != right.len() || left.is_empty() {
        return None;
    }
    let (mut dot, mut l2, mut r2) = (0.0f32, 0.0f32, 0.0f32);
    for (l, r) in left.iter().zip(right) {
        dot += l * r;
        l2 += l * l;
        r2 += r * r;
    }
    (l2 > 0.0 && r2 > 0.0).then(|| dot / (l2.sqrt() * r2.sqrt()))
}

#[cfg(test)]
mod tests;
