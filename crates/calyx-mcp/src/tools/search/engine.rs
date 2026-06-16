use std::collections::BTreeMap;
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::ledger_view::AsterLedgerCfStore;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AnchorKind, AnchorValue, CalyxError, Constellation, CxId, Input, LedgerRef, Modality, SlotId,
    SlotState, SlotVector, VaultStore,
};
use calyx_ledger::{LedgerCfStore, SubjectId, VerifyResult, decode, verify_chain};
use calyx_registry::{VaultPanelState, load_vault_panel_state};
use calyx_sextant::fusion;
use calyx_sextant::{
    AnchorPredicate, FusionContext, FusionStrategy, Hit, HnswIndex, IndexSearchHit, InvertedIndex,
    MaxSimIndex, MetadataPredicate, ProvenanceSource, QueryFilters, RrfProfile, ScalarOp,
    ScalarPredicate, SextantIndex,
};
use serde::Serialize;
use serde_json::Value;

use crate::server::{ToolError, ToolResult};

use super::output::KernelAnswerOut;
use super::{NeighborsRequest, SearchRequest};
use crate::tools::vault::store::{ResolvedVault, home_dir, resolve_vault_info, vault_salt};

pub(super) const HNSW_SEED: u64 = 0x0050_4836_3354_3034;

pub(super) struct SearchOutcome {
    pub(super) hits: Vec<Hit>,
    pub(super) docs: BTreeMap<CxId, Constellation>,
}

#[derive(Serialize)]
pub(super) struct NeighborOut {
    cx_id: String,
    score: f32,
    slot: u16,
}

pub(super) fn search(request: &SearchRequest) -> ToolResult<SearchOutcome> {
    let resolved = resolve_requested_vault(&request.vault)?;
    let vault = open_vault(&resolved)?;
    let state = load_vault_panel_state(&resolved.path)?;
    let docs = filtered_docs(load_docs(&vault)?, request.filter.clone())?;
    if docs.is_empty() {
        return Ok(SearchOutcome {
            hits: Vec::new(),
            docs,
        });
    }
    let ledger_refs = verify_ledger_before_provenance(&resolved.path)?;
    let query_vectors = measure_query_vectors(&state, &request.query)?;
    if query_vectors.is_empty() {
        return Err(no_indexable_query_vectors().into());
    }
    let per_slot = search_slots(&docs, &query_vectors)?;
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
    attach_stored_provenance(&mut hits, &docs, vault.latest_seq(), &ledger_refs)?;
    renumber_and_truncate(&mut hits, request.k);
    Ok(SearchOutcome { hits, docs })
}

fn verify_ledger_before_provenance(path: &Path) -> ToolResult<BTreeMap<CxId, LedgerRef>> {
    let store = AsterLedgerCfStore::open(path).map_err(|error| {
        if error.code == "CALYX_LEDGER_CORRUPT" {
            CalyxError::ledger_chain_broken(format!(
                "search provenance ledger chain unreadable: {}",
                error.message
            ))
        } else {
            error
        }
    })?;
    let end = store.scan()?.len() as u64;
    match verify_chain(&store, 0..end)? {
        VerifyResult::Intact { .. } => latest_cx_refs(&store),
        VerifyResult::Broken { at_seq, .. } | VerifyResult::Corrupt { at_seq, .. } => {
            Err(CalyxError::ledger_chain_broken(format!(
                "search provenance ledger chain broken at seq={at_seq}"
            ))
            .into())
        }
    }
}

fn latest_cx_refs(store: &AsterLedgerCfStore) -> ToolResult<BTreeMap<CxId, LedgerRef>> {
    let mut refs = BTreeMap::new();
    for row in store.scan()? {
        let entry = decode(&row.bytes)?;
        if let SubjectId::Cx(cx_id) = entry.subject {
            refs.insert(
                cx_id,
                LedgerRef {
                    seq: entry.seq,
                    hash: entry.entry_hash,
                },
            );
        }
    }
    Ok(refs)
}

pub(super) fn neighbors(request: &NeighborsRequest) -> ToolResult<Vec<NeighborOut>> {
    let resolved = resolve_requested_vault(&request.vault)?;
    let vault = open_vault(&resolved)?;
    let docs = load_docs(&vault)?;
    let seed = docs.get(&request.cx_id).ok_or_else(|| {
        CalyxError::vault_access_denied(format!("cx_id {} does not exist in vault", request.cx_id))
    })?;
    let mut out = Vec::new();
    for (slot, vector) in seed.slots.iter().filter(|(slot, vector)| {
        request.slot.is_none_or(|wanted| wanted == **slot) && indexable(vector)
    }) {
        for hit in search_one_slot(&docs, *slot, vector)?
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

fn filtered_docs(
    docs: BTreeMap<CxId, Constellation>,
    raw_filter: Option<Value>,
) -> ToolResult<BTreeMap<CxId, Constellation>> {
    let filter = parse_filter(raw_filter)?;
    Ok(docs
        .into_iter()
        .filter(|(_, cx)| filter_matches(cx, &filter))
        .collect())
}

fn parse_filter(raw: Option<Value>) -> ToolResult<QueryFilters> {
    let filters: QueryFilters = raw
        .map(serde_json::from_value)
        .transpose()
        .map_err(|err| ToolError::invalid_params(format!("parse filter JSON: {err}")))?
        .unwrap_or_default();
    filters.validate()?;
    Ok(filters)
}

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
) -> ToolResult<BTreeMap<SlotId, Vec<IndexSearchHit>>> {
    let mut out = BTreeMap::new();
    for (slot, query) in query_vectors {
        let hits = search_one_slot(docs, *slot, query)?;
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
) -> ToolResult<Vec<IndexSearchHit>> {
    let mut index = new_index(slot, query)?;
    let mut inserted = 0usize;
    for cx in docs.values() {
        if let Some(vector) = cx.slots.get(&slot)
            && same_index_shape(query, vector)
        {
            index.insert(cx.cx_id, vector.clone(), cx.provenance.seq)?;
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
    seq: u64,
    ledger_refs: &BTreeMap<CxId, LedgerRef>,
) -> ToolResult<()> {
    for hit in hits {
        let cx = docs.get(&hit.cx_id).ok_or_else(|| {
            CalyxError::vault_access_denied(format!(
                "stored constellation missing for hit {}",
                hit.cx_id
            ))
        })?;
        hit.provenance = ledger_refs
            .get(&hit.cx_id)
            .cloned()
            .unwrap_or_else(|| cx.provenance.clone());
        hit.provenance_source = ProvenanceSource::Stored;
        hit.freshness = if seq >= cx.provenance.seq {
            calyx_sextant::FreshnessTag::fresh(seq)
        } else {
            calyx_sextant::FreshnessTag::fresh(cx.provenance.seq)
        };
    }
    Ok(())
}

pub(super) fn load_docs(vault: &AsterVault) -> ToolResult<BTreeMap<CxId, Constellation>> {
    let snapshot = vault.snapshot();
    let mut docs = BTreeMap::new();
    for (key, _) in vault.scan_cf_at(snapshot, ColumnFamily::Base)? {
        let bytes: [u8; 16] = key.as_slice().try_into().map_err(|_| {
            CalyxError::vault_access_denied(format!("base CF key has {} bytes", key.len()))
        })?;
        let cx_id = CxId::from_bytes(bytes);
        docs.insert(cx_id, vault.get(cx_id, snapshot)?);
    }
    Ok(docs)
}

fn filter_matches(cx: &Constellation, filters: &QueryFilters) -> bool {
    filters
        .scalars
        .iter()
        .all(|filter| scalar_matches(cx, filter))
        && filters
            .anchors
            .iter()
            .all(|filter| anchor_matches(cx, filter))
        && filters
            .metadata
            .iter()
            .all(|filter| metadata_matches(cx, filter))
}

fn scalar_matches(cx: &Constellation, filter: &ScalarPredicate) -> bool {
    cx.scalars
        .get(&filter.name)
        .is_some_and(|actual| match filter.op {
            ScalarOp::Eq => actual == &filter.value,
            ScalarOp::Gt => *actual > filter.value,
            ScalarOp::Gte => *actual >= filter.value,
            ScalarOp::Lt => *actual < filter.value,
            ScalarOp::Lte => *actual <= filter.value,
        })
}

fn anchor_matches(cx: &Constellation, filter: &AnchorPredicate) -> bool {
    cx.anchors.iter().any(|anchor| {
        anchor.kind == filter.kind
            && filter
                .value
                .as_ref()
                .is_none_or(|value| anchor_value_matches(&anchor.value, value))
            && filter
                .min_confidence
                .is_none_or(|minimum| anchor.confidence >= minimum)
            && filter
                .source
                .as_ref()
                .is_none_or(|source| &anchor.source == source)
    })
}

fn metadata_matches(cx: &Constellation, filter: &MetadataPredicate) -> bool {
    match filter {
        MetadataPredicate::Vault(vault) => cx.vault_id == *vault,
        MetadataPredicate::Modality(modality) => cx.modality == *modality,
        MetadataPredicate::PanelVersion(version) => cx.panel_version == *version,
        MetadataPredicate::CreatedAt { min, max } => {
            min.is_none_or(|value| cx.created_at >= value)
                && max.is_none_or(|value| cx.created_at <= value)
        }
        MetadataPredicate::InputRedacted(expected) => cx.input_ref.redacted == *expected,
        MetadataPredicate::InputPointerContains(fragment) => cx
            .input_ref
            .pointer
            .as_deref()
            .is_some_and(|pointer| pointer.contains(fragment)),
    }
}

fn anchor_value_matches(actual: &AnchorValue, expected: &AnchorValue) -> bool {
    actual == expected
}

fn has_grounding(cx: &Constellation, anchor: Option<&AnchorKind>) -> bool {
    cx.anchors
        .iter()
        .any(|item| anchor.is_none_or(|kind| &item.kind == kind))
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
