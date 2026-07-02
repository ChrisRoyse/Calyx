use std::collections::BTreeMap;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AnchorKind, AnchorValue, CalyxError, Constellation, CxId, Input, Modality, SlotId, SlotState,
    SlotVector, VaultStore,
};
use calyx_registry::{VaultPanelState, load_vault_panel_state};
use calyx_sextant::fusion;
use calyx_sextant::{
    AnchorPredicate, DroppedGuardHit, FreshnessRequirement, FusionContext, FusionStrategy, Hit,
    HnswIndex, IndexSearchHit, InvertedIndex, MaxSimIndex, MetadataPredicate, ProvenanceSource,
    QueryFilters, RrfProfile, ScalarOp, ScalarPredicate, SextantIndex,
    apply_in_region_guard_to_hits,
};
use calyx_ward::GuardProfile;
use serde::Serialize;
use serde_json::Value;

use crate::server::{ToolError, ToolResult};

use super::output::KernelAnswerOut;
use super::{NeighborsRequest, SearchGuard, SearchRequest};
use crate::tools::search::ledger_provenance::VerifiedSearchLedger;
use crate::tools::vault::store::{ResolvedVault, home_dir, resolve_vault_info, vault_salt};

pub(super) const HNSW_SEED: u64 = 0x0050_4836_3354_3034;

pub(super) struct SearchOutcome {
    pub(super) hits: Vec<Hit>,
    pub(super) docs: BTreeMap<CxId, Constellation>,
    pub(super) dropped_guard_hits: Vec<DroppedGuardHit>,
}

#[derive(Serialize)]
pub(super) struct NeighborOut {
    cx_id: String,
    score: f32,
    slot: u16,
}

pub(super) fn search(request: &SearchRequest) -> ToolResult<SearchOutcome> {
    let resolved = resolve_requested_vault(&request.vault)?;
    let ledger = VerifiedSearchLedger::open(&resolved.path)?;
    let vault = open_vault(&resolved)?;
    let state = load_vault_panel_state(&resolved.path)?;
    let loaded = load_docs(&vault)?;
    let snapshot_seq = loaded.snapshot_seq;
    let docs = filtered_docs(loaded.docs, request.filter.clone())?;
    if docs.is_empty() {
        return Ok(SearchOutcome {
            hits: Vec::new(),
            docs,
            dropped_guard_hits: Vec::new(),
        });
    }
    let query_vectors = measure_query_vectors(&state, &request.query)?;
    if query_vectors.is_empty() {
        return Err(no_indexable_query_vectors().into());
    }
    let per_slot = search_slots(&docs, &query_vectors, snapshot_seq)?;
    let slots = per_slot.keys().copied().collect::<Vec<_>>();
    if slots.is_empty() {
        return Err(no_indexable_stored_vectors().into());
    }
    let strategy = request.fusion.to_strategy(&slots)?;
    let context = FusionContext {
        k: docs.len().max(request.k),
        explain: request.explain,
        strategy: strategy.clone(),
        weights: weights_for(&strategy, &slots),
        stage1_slots: stage1_slots(&strategy, &query_vectors, &slots),
    };
    let mut hits = fusion::fuse(&per_slot, &context);
    attach_stored_provenance(&mut hits, &docs, snapshot_seq, &ledger, &request.freshness)?;
    let dropped_guard_hits = if request.guard == SearchGuard::InRegion {
        let before = hits.len();
        let profile = load_default_guard_profile(&vault, &state)?;
        let dropped =
            apply_in_region_guard_to_hits(&docs, &profile, &query_vectors, &mut hits, true)?;
        if before > 0 && hits.is_empty() {
            return Err(CalyxError::guard_ood(format!(
                "in-region guard blocked all {before} search candidates"
            ))
            .into());
        }
        dropped
    } else {
        Vec::new()
    };
    renumber_and_truncate(&mut hits, request.k);
    Ok(SearchOutcome {
        hits,
        docs,
        dropped_guard_hits,
    })
}

fn load_default_guard_profile(
    vault: &AsterVault,
    state: &VaultPanelState,
) -> ToolResult<GuardProfile> {
    let Some(bytes) =
        vault.read_cf_at(vault.snapshot(), ColumnFamily::Guard, default_guard_key())?
    else {
        return Err(CalyxError::guard_provisional(
            "search guard requires a calibrated default guard profile",
        )
        .into());
    };
    let profile: GuardProfile = serde_json::from_slice(&bytes).map_err(|error| {
        CalyxError::guard_provisional(format!("decode default guard profile: {error}"))
    })?;
    if profile.panel_version != u64::from(state.panel.version) {
        return Err(CalyxError::guard_provisional(format!(
            "guard profile panel_version {} does not match active panel {}",
            profile.panel_version, state.panel.version
        ))
        .into());
    }
    if !profile.is_calibrated() {
        return Err(CalyxError::guard_provisional("search guard profile is not calibrated").into());
    }
    Ok(profile)
}

fn default_guard_key() -> &'static [u8] {
    b"profile\0default"
}

pub(super) fn neighbors(request: &NeighborsRequest) -> ToolResult<Vec<NeighborOut>> {
    let resolved = resolve_requested_vault(&request.vault)?;
    let vault = open_vault(&resolved)?;
    let loaded = load_docs(&vault)?;
    let docs = loaded.docs;
    let seed = docs.get(&request.cx_id).ok_or_else(|| {
        CalyxError::vault_access_denied(format!("cx_id {} does not exist in vault", request.cx_id))
    })?;
    let mut out = Vec::new();
    for (slot, vector) in seed.slots.iter().filter(|(slot, vector)| {
        request.slot.is_none_or(|wanted| wanted == **slot) && indexable(vector)
    }) {
        for hit in search_one_slot(&docs, *slot, vector, loaded.snapshot_seq)?
            .into_iter()
            .take(request.k)
        {
            out.push(NeighborOut {
                cx_id: hit.cx_id.to_string(),
                score: hit.score,
                slot: slot.get(),
            });
        }
    }
    if out.is_empty() && request.slot.is_some() {
        return Err(no_indexable_stored_vectors().into());
    }
    out.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.slot.cmp(&b.slot))
            .then_with(|| a.cx_id.cmp(&b.cx_id))
    });
    out.truncate(request.k);
    Ok(out)
}

pub(super) fn kernel_report(
    docs: &BTreeMap<CxId, Constellation>,
    hits: &[Hit],
    anchor: Option<&AnchorKind>,
) -> ToolResult<KernelAnswerOut> {
    let grounded = docs
        .values()
        .filter(|cx| has_grounding(cx, anchor))
        .map(|cx| cx.cx_id)
        .collect::<Vec<_>>();
    if grounded.is_empty() {
        return Err(CalyxError::kernel_ungrounded("kernel-answer has no grounded anchors").into());
    }
    let mut kernel_ids = hits
        .iter()
        .map(|hit| hit.cx_id)
        .filter(|cx_id| grounded.contains(cx_id))
        .take(5)
        .collect::<Vec<_>>();
    if kernel_ids.is_empty() {
        kernel_ids.extend(grounded.iter().copied().take(5));
    }
    let gap_count = docs.len().saturating_sub(grounded.len());
    let gaps = (gap_count > 0)
        .then(|| format!("grounding_gaps:{gap_count}"))
        .into_iter()
        .collect();
    Ok(KernelAnswerOut {
        answer: format!(
            "grounded kernel answer over {} anchored constellations",
            grounded.len()
        ),
        kernel_cx_ids: kernel_ids.into_iter().map(|id| id.to_string()).collect(),
        recall: grounded.len() as f32 / docs.len().max(1) as f32,
        gaps,
    })
}

mod filters;
use filters::{filtered_docs, has_grounding};

pub(super) fn measure_query_vectors(
    state: &VaultPanelState,
    query: &str,
) -> ToolResult<Vec<(SlotId, SlotVector)>> {
    let input = Input::new(Modality::Text, query.as_bytes().to_vec());
    let mut out = Vec::new();
    for slot in &state.panel.slots {
        if slot.state == SlotState::Active
            && slot.modality == Modality::Text
            && state.registry.contains(slot.lens_id)
        {
            let vector = state.registry.measure(slot.lens_id, &input)?;
            if indexable(&vector) {
                out.push((slot.slot_id, vector));
            }
        }
    }
    Ok(out)
}

fn search_slots(
    docs: &BTreeMap<CxId, Constellation>,
    query_vectors: &[(SlotId, SlotVector)],
    snapshot_seq: u64,
) -> ToolResult<BTreeMap<SlotId, Vec<IndexSearchHit>>> {
    let mut out = BTreeMap::new();
    for (slot, query) in query_vectors {
        let hits = search_one_slot(docs, *slot, query, snapshot_seq)?;
        if !hits.is_empty() {
            out.insert(*slot, hits);
        }
    }
    Ok(out)
}

fn search_one_slot(
    docs: &BTreeMap<CxId, Constellation>,
    slot: SlotId,
    query: &SlotVector,
    snapshot_seq: u64,
) -> ToolResult<Vec<IndexSearchHit>> {
    let mut index = new_index(slot, query)?;
    let mut inserted = 0usize;
    for cx in docs.values() {
        if let Some(vector) = cx.slots.get(&slot)
            && same_index_shape(query, vector)
        {
            index.insert(cx.cx_id, vector.clone(), snapshot_seq)?;
            inserted += 1;
        }
    }
    if inserted == 0 {
        return Ok(Vec::new());
    }
    Ok(index.search(query, inserted, Some(inserted.max(64)))?)
}

fn new_index(slot: SlotId, query: &SlotVector) -> ToolResult<Box<dyn SextantIndex>> {
    match query {
        SlotVector::Dense { dim, .. } => Ok(Box::new(HnswIndex::new(slot, *dim, HNSW_SEED))),
        SlotVector::Sparse { .. } => Ok(Box::new(InvertedIndex::new(slot))),
        SlotVector::Multi { token_dim, .. } => Ok(Box::new(MaxSimIndex::new(slot, *token_dim))),
        SlotVector::Absent { .. } => Err(ToolError::invalid_params(
            "query slot vector must be concrete",
        )),
    }
}

fn attach_stored_provenance(
    hits: &mut [Hit],
    docs: &BTreeMap<CxId, Constellation>,
    snapshot_seq: u64,
    ledger: &VerifiedSearchLedger,
    freshness: &FreshnessRequirement,
) -> ToolResult<()> {
    for hit in hits {
        let cx = docs.get(&hit.cx_id).ok_or_else(|| {
            CalyxError::vault_access_denied(format!(
                "stored constellation missing for hit {}",
                hit.cx_id
            ))
        })?;
        hit.provenance = ledger.require_ref(hit.cx_id, cx.provenance.clone())?;
        hit.provenance_source = ProvenanceSource::Stored;
        // The hits were computed over indexes built from the docs pinned at
        // `snapshot_seq`, so they are fresh at exactly that seq. `provenance.seq`
        // is a ledger ref in a different seq domain and must not leak into
        // freshness tags (issue #1104).
        hit.freshness = match freshness {
            FreshnessRequirement::FreshDerived => calyx_sextant::FreshnessTag::fresh(snapshot_seq),
            FreshnessRequirement::StaleOk { .. } => {
                calyx_sextant::FreshnessTag::stale_ok(snapshot_seq, snapshot_seq)
            }
        };
    }
    Ok(())
}

/// Documents loaded at one pinned vault snapshot together with that
/// snapshot's commit seq. Freshness reasoning over these docs must use
/// `snapshot_seq` only: per-doc `provenance.seq` values are ledger refs in a
/// different seq domain and drift from vault commit seqs on group-committed
/// vaults (issue #1104).
pub(super) struct LoadedDocs {
    pub(super) docs: BTreeMap<CxId, Constellation>,
    pub(super) snapshot_seq: u64,
}

pub(super) fn load_docs(vault: &AsterVault) -> ToolResult<LoadedDocs> {
    let snapshot = vault.snapshot();
    let mut docs = BTreeMap::new();
    for (key, _) in vault.scan_cf_at(snapshot, ColumnFamily::Base)? {
        let bytes: [u8; 16] = key.as_slice().try_into().map_err(|_| {
            CalyxError::vault_access_denied(format!("base CF key has {} bytes", key.len()))
        })?;
        let cx_id = CxId::from_bytes(bytes);
        docs.insert(cx_id, vault.get(cx_id, snapshot)?);
    }
    Ok(LoadedDocs {
        docs,
        snapshot_seq: snapshot,
    })
}

pub(super) fn resolve_requested_vault(vault: &str) -> ToolResult<ResolvedVault> {
    resolve_vault_info(&home_dir()?, vault)
}

pub(super) fn open_vault(resolved: &ResolvedVault) -> ToolResult<AsterVault> {
    Ok(AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::default(),
    )?)
}

fn renumber_and_truncate(hits: &mut Vec<Hit>, k: usize) {
    hits.truncate(k);
    for (idx, hit) in hits.iter_mut().enumerate() {
        hit.rank = idx + 1;
    }
}

pub(super) fn indexable(vector: &SlotVector) -> bool {
    matches!(
        vector,
        SlotVector::Dense { .. } | SlotVector::Sparse { .. } | SlotVector::Multi { .. }
    )
}

pub(super) fn same_index_shape(query: &SlotVector, stored: &SlotVector) -> bool {
    match (query, stored) {
        (SlotVector::Dense { dim: q, .. }, SlotVector::Dense { dim: s, .. }) => q == s,
        (SlotVector::Sparse { dim: q, .. }, SlotVector::Sparse { dim: s, .. }) => q == s,
        (SlotVector::Multi { token_dim: q, .. }, SlotVector::Multi { token_dim: s, .. }) => q == s,
        _ => false,
    }
}

fn weights_for(strategy: &FusionStrategy, slots: &[SlotId]) -> BTreeMap<SlotId, f32> {
    let Some(profile) = weighted_profile(strategy) else {
        return BTreeMap::new();
    };
    let profile_weights = fusion::profiles::lookup(profile)
        .map(|profile| profile.weights)
        .unwrap_or_default();
    slots
        .iter()
        .map(|slot| (*slot, profile_weights.get(slot).copied().unwrap_or(1.0)))
        .collect()
}

fn weighted_profile(strategy: &FusionStrategy) -> Option<RrfProfile> {
    match strategy {
        FusionStrategy::WeightedRrf { profile } => Some(*profile),
        _ => None,
    }
}

fn stage1_slots(
    strategy: &FusionStrategy,
    query_vectors: &[(SlotId, SlotVector)],
    slots: &[SlotId],
) -> Vec<SlotId> {
    if !matches!(strategy, FusionStrategy::Pipeline) {
        return Vec::new();
    }
    let sparse = query_vectors
        .iter()
        .filter_map(|(slot, vector)| matches!(vector, SlotVector::Sparse { .. }).then_some(*slot))
        .filter(|slot| slots.contains(slot))
        .collect::<Vec<_>>();
    if sparse.is_empty() {
        slots.first().copied().into_iter().collect()
    } else {
        sparse
    }
}

fn no_indexable_query_vectors() -> CalyxError {
    CalyxError::stale_derived(
        "search has no indexable query vectors from active text lenses; re-enable a concrete lens or remeasure the panel",
    )
}

fn no_indexable_stored_vectors() -> CalyxError {
    CalyxError::stale_derived(
        "search has no indexable stored slot vectors matching active query lenses; reingest or backfill stale slot rows",
    )
}
