use std::collections::BTreeSet;
use std::path::Path;

use calyx_aster::ledger_view::AsterLedgerCfStore;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, SlotId, SlotVector};
use calyx_sextant::FusionContext;
use calyx_sextant::{apply_in_region_guard_to_hits, fusion};

use crate::engine_fusion::{stage1_slots, weights_for};
use crate::engine_measure::{no_indexable_query_vectors, no_indexable_stored_vectors};
use crate::engine_slot_cache::{SearchSlotCache, search_slots_with_cache};
use crate::engine_slot_fanout::search_slots_uncached;
use crate::engine_trace::SearchTracer;
use crate::error::CliResult;
use crate::persisted::PersistedSearchIndexes;
use crate::provenance::attach_verified_provenance;

use super::guard::{ResolvedGuard, apply_in_region_guard_traced, resolve_guard};
use super::hydration::hydrate_hit_docs_with_bounded_readbacks;
use super::support::{
    SearchReadSnapshot, is_stale_derived, renumber_and_truncate, vault_base_count_at,
};
use super::{FusionChoice, GuardChoice, SearchBudget, SearchFreshness, SearchOutcome};

const EXACT_METADATA_CANDIDATE_FLOOR: usize = 32;

#[allow(clippy::too_many_arguments)]
pub(super) fn search_outcome_with_measured_slots(
    vault: &AsterVault,
    vault_dir: &Path,
    query_vectors: &[(SlotId, SlotVector)],
    metadata_query: Option<&str>,
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
    ledger_view: Option<&AsterLedgerCfStore>,
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
    let generation = indexes.generation()?;
    trace.emit(
        "indexes.open.done",
        None,
        Some(indexes.max_len_for_slots(allowed_slots)),
    );
    if indexes.max_len_for_slots(allowed_slots) == 0 {
        trace.emit("indexes.empty", None, None);
        return Ok(SearchOutcome::empty_with_generation(generation));
    }
    trace.emit("indexes.pin_budget.start", None, None);
    let pin_budget = indexes.pin_budget_preflight_for_slots(allowed_slots)?;
    trace.emit_detail(
        "indexes.pin_budget.done",
        None,
        None,
        Some(format!(
            "required_bytes={} projected_process_bytes={} configured_bytes={}",
            pin_budget.required_bytes,
            pin_budget.projected_process_bytes,
            pin_budget.configured_bytes
        )),
    );
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
        return Ok(SearchOutcome::empty_with_generation(generation));
    }
    trace.emit("metadata_candidates.start", None, None);
    let mut metadata_candidates = metadata_query
        .map(|query| indexes.exact_metadata_candidates(query))
        .transpose()?
        .unwrap_or_default();
    if let Some(filter_candidates) = &filter_candidates {
        metadata_candidates.retain(|cx_id| filter_candidates.contains(cx_id));
    }
    trace.emit(
        "metadata_candidates.done",
        None,
        Some(metadata_candidates.len()),
    );
    // Exact metadata candidates are recalled independently across every slot,
    // so they do not need the generic or cross-encoder candidate headroom.
    // Preserve the wider floor when metadata recall found nothing, and keep
    // ordinary non-reranked searches at their historical 64.
    let retrieval_floor = match (metadata_query, metadata_candidates.is_empty()) {
        (Some(_), false) => EXACT_METADATA_CANDIDATE_FLOOR,
        (Some(_), true) => super::rerank::RERANK_CANDIDATE_FLOOR,
        (None, _) => 64,
    };
    let retrieval_k = k
        .max(retrieval_floor)
        .saturating_add(metadata_candidates.len());
    let search_k = filter_candidates
        .as_ref()
        .map(|ids| ids.len())
        .unwrap_or_else(|| k.max(retrieval_floor));
    trace.emit_detail(
        "search_slots.start",
        None,
        Some(query_vectors.len()),
        Some(format!("search_k={search_k}")),
    );
    budget.check("before_search_slots", query_vectors.len())?;
    let mut per_slot = search_slots_with_cache(
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
    if !metadata_candidates.is_empty() {
        trace.emit(
            "metadata_candidates.recall.start",
            None,
            Some(metadata_candidates.len()),
        );
        let (metadata_per_slot, _) = search_slots_uncached(
            &indexes,
            query_vectors,
            metadata_candidates.len(),
            Some(&metadata_candidates),
            trace,
        )?;
        let mut added = 0usize;
        for (slot, metadata_hits) in metadata_per_slot {
            let hits = per_slot.entry(slot).or_default();
            let mut seen = hits.iter().map(|hit| hit.cx_id).collect::<BTreeSet<_>>();
            for mut hit in metadata_hits {
                if seen.insert(hit.cx_id) {
                    hit.rank = hits.len() + 1;
                    hits.push(hit);
                    added += 1;
                }
            }
        }
        trace.emit_detail(
            "metadata_candidates.recall.done",
            None,
            Some(added),
            Some(format!("candidate_count={}", metadata_candidates.len())),
        );
    }
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
        k: retrieval_k,
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
    let required_metadata_hits = if metadata_candidates.is_empty() {
        Vec::new()
    } else {
        let metadata_per_slot = per_slot
            .iter()
            .map(|(slot, hits)| {
                (
                    *slot,
                    hits.iter()
                        .filter(|hit| metadata_candidates.contains(&hit.cx_id))
                        .cloned()
                        .collect(),
                )
            })
            .collect();
        fusion::fuse(&metadata_per_slot, &context)
    };
    let mut hits = fusion::fuse(&per_slot, &context);
    if !required_metadata_hits.is_empty() {
        let required_ids = required_metadata_hits
            .iter()
            .map(|hit| hit.cx_id)
            .collect::<BTreeSet<_>>();
        let mut seen = hits.iter().map(|hit| hit.cx_id).collect::<BTreeSet<_>>();
        hits.extend(
            required_metadata_hits
                .into_iter()
                .filter(|hit| seen.insert(hit.cx_id)),
        );
        if hits.len() > retrieval_k {
            let (mut required, mut others): (Vec<_>, Vec<_>) = hits
                .into_iter()
                .partition(|hit| required_ids.contains(&hit.cx_id));
            others.truncate(retrieval_k.saturating_sub(required.len()));
            others.append(&mut required);
            others.sort_by(|left, right| {
                right
                    .score
                    .total_cmp(&left.score)
                    .then_with(|| left.cx_id.cmp(&right.cx_id))
            });
            for (index, hit) in others.iter_mut().enumerate() {
                hit.rank = index + 1;
            }
            hits = others;
        }
    }
    trace.emit("fusion.done", None, Some(hits.len()));
    if guard != GuardChoice::InRegion {
        // Enforce the caller-requested final top-k (`k`), NOT the internal
        // retrieval window (`retrieval_k`). `retrieval_k` is deliberately wider
        // (the 64/128 fusion+rerank candidate budget) to give rank fusion the
        // recall it needs; exposing that window as the returned result set is
        // the #1846 contract breach (final list pruned to the window instead of
        // to `size`). Fusion has already ordered hits by descending score with
        // the documented cx_id tie-break, so truncating here keeps the top `k`.
        // Doing it before hydration also bounds the readback fan-out to `k`.
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
    budget.check("before_hit_hydration", hits.len())?;
    let (hit_docs, freshness_tag, _provenance_read) = hydrate_hit_docs_with_bounded_readbacks(
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
    attach_verified_provenance(
        &mut hits,
        &hit_docs,
        vault_dir,
        ledger_view,
        freshness_tag,
        trace,
    )?;
    trace.emit("provenance.attach.done", None, Some(hits.len()));
    let mut dropped_guard_hits = Vec::new();
    let applied_guard_tau = match &resolved_guard {
        ResolvedGuard::Off => None,
        ResolvedGuard::OperatorTau(tau) => {
            let tau = *tau;
            trace.emit("guard.in_region.start", None, Some(hits.len()));
            budget.check("before_in_region_guard", hits.len())?;
            hits = apply_in_region_guard_traced(hits, &hit_docs, query_vectors, tau, trace)?;
            budget.check("after_in_region_guard", hits.len())?;
            trace.emit("guard.in_region.done", None, Some(hits.len()));
            // In-region carries the wide `retrieval_k` pool through prefilter +
            // guard (guarding drops candidates), then trims to the caller `k`
            // only after guarding so up to `k` survivors remain (#1846).
            renumber_and_truncate(&mut hits, k);
            Some(tau)
        }
        ResolvedGuard::Profile(profile) => apply_profile_guard(
            &hit_docs,
            query_vectors,
            // Trim the profile-guard survivors to the caller `k`, not the wide
            // internal `retrieval_k` window (#1846). apply_profile_guard's own
            // final renumber_and_truncate uses this argument.
            k,
            trace,
            &mut budget,
            &mut hits,
            &mut dropped_guard_hits,
            profile,
        )?,
    };
    trace.emit("search.done", None, Some(hits.len()));
    Ok(SearchOutcome {
        hits,
        guard_tau: applied_guard_tau,
        docs: hit_docs,
        dropped_guard_hits,
        generation: Some(generation),
    })
}

#[allow(clippy::too_many_arguments)]
fn apply_profile_guard(
    hit_docs: &std::collections::BTreeMap<calyx_core::CxId, calyx_core::Constellation>,
    query_vectors: &[(SlotId, SlotVector)],
    k: usize,
    trace: &mut SearchTracer<'_>,
    budget: &mut SearchBudget<'_>,
    hits: &mut Vec<calyx_sextant::Hit>,
    dropped_guard_hits: &mut Vec<calyx_sextant::DroppedGuardHit>,
    profile: &calyx_ward::GuardProfile,
) -> CliResult<Option<f32>> {
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
    *dropped_guard_hits =
        apply_in_region_guard_to_hits(hit_docs, profile, query_vectors, hits, true)?;
    budget.check("after_in_region_guard", hits.len())?;
    for dropped in dropped_guard_hits {
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
    renumber_and_truncate(hits, k);
    Ok(None)
}
