use std::collections::BTreeMap;

use calyx_aster::mvcc::{Freshness, Snapshot};
use calyx_aster::vault::AsterVault;
use calyx_core::{
    Constellation, CxId, SlotId, SlotSimilarityEvidence, SlotVector, slot_similarity,
};
use calyx_sextant::{FreshnessTag, Hit};

#[cfg(test)]
use super::GUARD_TAU;
use super::{SEARCH_READER_LEASE_MS, SearchFreshness};
use crate::error::CliResult;
use crate::persisted::{PersistedSearchIndexes, load_docs_at};

pub(super) fn index_freshness_tag(
    indexes: &PersistedSearchIndexes,
    pinned_seq: u64,
    derived_content_seq: u64,
    freshness: SearchFreshness,
) -> CliResult<FreshnessTag> {
    match freshness {
        SearchFreshness::Fresh => {
            indexes.ensure_fresh_at_snapshot(pinned_seq, derived_content_seq)?;
            Ok(FreshnessTag::fresh(pinned_seq))
        }
        SearchFreshness::StaleOk => {
            let built_at_seq = indexes.base_seq();
            if built_at_seq > pinned_seq {
                return Err(calyx_core::CalyxError::stale_derived(format!(
                    "persistent search manifest base seq {built_at_seq} is ahead of pinned vault seq {pinned_seq}; rebuild the vault search indexes before search"
                ))
                .into());
            }
            Ok(FreshnessTag::stale_ok(built_at_seq, pinned_seq))
        }
    }
}

pub(super) struct SearchReadSnapshot<'a> {
    vault: &'a AsterVault,
    snapshot: Snapshot,
}

impl<'a> SearchReadSnapshot<'a> {
    pub(super) fn pin(vault: &'a AsterVault) -> Self {
        Self {
            vault,
            snapshot: vault.pin_reader(Freshness::FreshDerived, SEARCH_READER_LEASE_MS),
        }
    }

    pub(super) fn snapshot(&self) -> Snapshot {
        self.snapshot
    }

    pub(super) fn seq(&self) -> u64 {
        self.snapshot.seq()
    }

    /// Derived-content watermark observed at pin time, clamped to the pinned
    /// seq by the MVCC store (issue #1100).
    pub(super) fn derived_content_seq(&self) -> u64 {
        self.snapshot.derived_content_seq()
    }

    pub(super) fn lease_id(&self) -> u64 {
        self.snapshot.lease().id()
    }

    pub(super) fn lease_max_age_ms(&self) -> u64 {
        self.snapshot.lease().max_age_ms()
    }

    pub(super) fn lease_expires_at(&self) -> u64 {
        self.snapshot.lease().expires_at()
    }
}

impl Drop for SearchReadSnapshot<'_> {
    fn drop(&mut self) {
        let _ = self.vault.release_reader(self.snapshot.lease().id());
    }
}

pub(super) fn is_stale_derived(error: &crate::error::SearchError) -> bool {
    matches!(error, crate::error::SearchError::Calyx(inner) if inner.code == "CALYX_STALE_DERIVED")
}

/// Keep only hits whose best per-lens cosine to the query meets the guard tau.
#[cfg(test)]
pub(super) fn apply_in_region_guard(
    hits: Vec<Hit>,
    docs: &BTreeMap<CxId, Constellation>,
    query_vectors: &[(calyx_core::SlotId, SlotVector)],
) -> CliResult<Vec<Hit>> {
    let mut kept = Vec::new();
    for hit in hits {
        let evidence = guard_slot_evidence(&hit, docs, query_vectors)?;
        if evidence
            .iter()
            .all(|(_, similarity)| similarity.score >= GUARD_TAU)
        {
            kept.push(hit);
        }
    }
    Ok(kept)
}

pub(super) fn vault_base_count_at(vault: &AsterVault, snapshot: Snapshot) -> CliResult<usize> {
    Ok(load_docs_at(vault, snapshot)?.len())
}

pub(super) fn renumber_and_truncate(hits: &mut Vec<Hit>, k: usize) {
    hits.truncate(k);
    for (idx, hit) in hits.iter_mut().enumerate() {
        hit.rank = idx + 1;
    }
}

#[cfg(test)]
pub(super) fn cosine(left: &[f32], right: &[f32]) -> Option<f32> {
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

pub(super) fn guard_slot_evidence(
    hit: &Hit,
    docs: &BTreeMap<CxId, Constellation>,
    query_vectors: &[(calyx_core::SlotId, SlotVector)],
) -> CliResult<Vec<(SlotId, SlotSimilarityEvidence)>> {
    let cx = docs.get(&hit.cx_id).ok_or_else(|| {
        calyx_core::CalyxError::guard_provisional(format!(
            "operator guard cannot hydrate constellation {}",
            hit.cx_id
        ))
    })?;
    if query_vectors.is_empty() {
        return Err(calyx_core::CalyxError::guard_provisional(
            "operator guard requires at least one measured typed query slot",
        )
        .into());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut evidence = Vec::with_capacity(query_vectors.len());
    for (slot, query) in query_vectors {
        if !seen.insert(*slot) {
            return Err(calyx_core::CalyxError::guard_provisional(format!(
                "operator guard query contains duplicate slot {slot}"
            ))
            .into());
        }
        let stored = cx.slots.get(slot).ok_or_else(|| {
            calyx_core::CalyxError::guard_provisional(format!(
                "operator guard constellation {} is missing required slot {slot}",
                hit.cx_id
            ))
        })?;
        evidence.push((*slot, slot_similarity(query, stored)?));
    }
    Ok(evidence)
}
