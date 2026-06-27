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
use calyx_core::{CalyxError, Constellation, CxId, SlotId, SlotVector, VaultStore};
use calyx_sextant::fusion;
use calyx_sextant::{
    FreshnessTag, FusionContext, FusionStrategy, Hit, ProvenanceSource, RrfProfile,
};

use crate::error::CliResult;
use crate::persisted::{PersistedSearchIndexes, load_docs};

/// In-region guard cosine threshold (mirrors the CLI default).
const GUARD_TAU: f32 = 0.999;

/// Fusion strategy choice (transport-agnostic; the CLI flag parser and the HTTP
/// request both map onto this, then it resolves to a concrete `FusionStrategy`
/// against the live slot set).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FusionChoice {
    Rrf,
    WeightedRrf,
    SingleLens,
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
            Self::SingleLens => slots
                .first()
                .copied()
                .map(|slot| FusionStrategy::SingleLens { slot })
                .ok_or_else(|| {
                    crate::error::SearchError::usage("single-lens search has no active lens slot")
                }),
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
}

impl SearchOutcome {
    fn empty() -> Self {
        Self {
            hits: Vec::new(),
            guard_tau: None,
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
    let filters = crate::filters::parse(filter)?;
    let indexes = match PersistedSearchIndexes::open(vault_dir) {
        Ok(indexes) => indexes,
        Err(error) if is_stale_derived(&error) && vault_base_count(vault)? == 0 => {
            return Ok(SearchOutcome::empty());
        }
        Err(error) => return Err(error),
    };
    if indexes.max_len() == 0 {
        return Ok(SearchOutcome::empty());
    }
    let query_vectors = measure_query_vectors(state, query)?;
    if query_vectors.is_empty() {
        return Err(no_indexable_query_vectors().into());
    }
    let filter_candidates = indexes.filter_candidates(&filters)?;
    if filter_candidates.as_ref().is_some_and(|ids| ids.is_empty()) {
        return Ok(SearchOutcome::empty());
    }
    let search_k = filter_candidates
        .as_ref()
        .map(|ids| ids.len())
        .unwrap_or_else(|| k.max(64));
    let per_slot = search_slots(
        &indexes,
        &query_vectors,
        search_k,
        filter_candidates.as_ref(),
    )?;
    let slots = per_slot.keys().copied().collect::<Vec<_>>();
    if slots.is_empty() {
        return Err(no_indexable_stored_vectors().into());
    }
    let strategy = fusion.to_strategy(&slots)?;
    let context = FusionContext {
        k: k.max(64),
        explain,
        strategy: strategy.clone(),
        weights: weights_for(&strategy, &slots),
        stage1_slots: stage1_slots(&strategy, &query_vectors, &slots),
    };
    let mut hits = fusion::fuse(&per_slot, &context);
    let hit_docs = hit_docs(vault, &hits)?;
    attach_stored_provenance(&mut hits, &hit_docs, vault.latest_seq())?;
    let guard_tau = if guard == GuardChoice::InRegion {
        hits = apply_in_region_guard(hits, &hit_docs, &query_vectors);
        Some(GUARD_TAU)
    } else {
        None
    };
    renumber_and_truncate(&mut hits, k);
    Ok(SearchOutcome { hits, guard_tau })
}

fn is_stale_derived(error: &crate::error::SearchError) -> bool {
    matches!(error, crate::error::SearchError::Calyx(inner) if inner.code == "CALYX_STALE_DERIVED")
}

/// Measure the query through every active text lens that is materialized in the
/// registry, keeping only indexable vectors.
pub fn measure_query_vectors(
    state: &calyx_registry::VaultPanelState,
    query: &str,
) -> CliResult<Vec<(SlotId, SlotVector)>> {
    use calyx_core::{Input, Modality, SlotState};
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

fn search_slots(
    indexes: &PersistedSearchIndexes,
    query_vectors: &[(SlotId, SlotVector)],
    k: usize,
    filter_candidates: Option<&BTreeSet<CxId>>,
) -> CliResult<BTreeMap<SlotId, Vec<calyx_sextant::IndexSearchHit>>> {
    let mut out = BTreeMap::new();
    for (slot, query) in query_vectors {
        let hits = if let Some(candidates) = filter_candidates {
            indexes.search_filtered(*slot, query, k, candidates)?
        } else {
            indexes.search(*slot, query, k)?
        };
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

fn attach_stored_provenance(
    hits: &mut [Hit],
    docs: &BTreeMap<CxId, Constellation>,
    seq: u64,
) -> CliResult {
    for hit in hits {
        let cx = docs.get(&hit.cx_id).ok_or_else(|| {
            CalyxError::vault_access_denied(format!(
                "stored constellation missing for hit {}",
                hit.cx_id
            ))
        })?;
        hit.provenance = cx.provenance.clone();
        hit.provenance_source = ProvenanceSource::Stored;
        hit.freshness = FreshnessTag::fresh(seq);
    }
    Ok(())
}

fn hit_docs(vault: &AsterVault, hits: &[Hit]) -> CliResult<BTreeMap<CxId, Constellation>> {
    let snapshot = vault.snapshot();
    let mut docs = BTreeMap::new();
    for hit in hits {
        let cx_id = hit.cx_id;
        docs.insert(cx_id, vault.get(cx_id, snapshot)?);
    }
    Ok(docs)
}

fn vault_base_count(vault: &AsterVault) -> CliResult<usize> {
    Ok(load_docs(vault)?.len())
}

fn renumber_and_truncate(hits: &mut Vec<Hit>, k: usize) {
    hits.truncate(k);
    for (idx, hit) in hits.iter_mut().enumerate() {
        hit.rank = idx + 1;
    }
}

fn indexable(vector: &SlotVector) -> bool {
    matches!(
        vector,
        SlotVector::Dense { .. } | SlotVector::Sparse { .. } | SlotVector::Multi { .. }
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_core::{
        Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId,
    };
    use calyx_sextant::{
        CausalConfidence, FreshnessTag, Hit, PerLensContribution, ProvenanceSource,
    };
    use std::collections::BTreeMap;
    use ulid::Ulid;

    #[test]
    fn no_indexable_errors_are_stale_derived() {
        assert_eq!(no_indexable_query_vectors().code, "CALYX_STALE_DERIVED");
        assert_eq!(no_indexable_stored_vectors().code, "CALYX_STALE_DERIVED");
    }

    #[test]
    fn cosine_is_one_for_identical_and_zero_for_orthogonal() {
        assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0]), Some(1.0));
        assert_eq!(cosine(&[1.0, 0.0], &[0.0, 1.0]), Some(0.0));
        assert_eq!(cosine(&[1.0], &[1.0, 0.0]), None);
        assert_eq!(cosine(&[], &[]), None);
    }

    #[test]
    fn in_region_guard_rejects_orthogonal_dense_hit() {
        let slot = SlotId::new(0);
        let id = cx(2);
        let mut docs = BTreeMap::new();
        docs.insert(id, constellation(id, vec![0.0, 1.0]));
        let hit = sample_hit(id);
        let query_vectors = vec![(
            slot,
            SlotVector::Dense {
                dim: 2,
                data: vec![1.0, 0.0],
            },
        )];

        // Orthogonal query vs stored vector → cosine 0 < GUARD_TAU → filtered out.
        let kept = apply_in_region_guard(vec![hit], &docs, &query_vectors);
        assert!(kept.is_empty(), "orthogonal hit must be guard-rejected");
    }

    #[test]
    fn in_region_guard_keeps_aligned_dense_hit() {
        let slot = SlotId::new(0);
        let id = cx(3);
        let mut docs = BTreeMap::new();
        docs.insert(id, constellation(id, vec![1.0, 0.0]));
        let hit = sample_hit(id);
        let query_vectors = vec![(
            slot,
            SlotVector::Dense {
                dim: 2,
                data: vec![1.0, 0.0],
            },
        )];
        let kept = apply_in_region_guard(vec![hit], &docs, &query_vectors);
        assert_eq!(
            kept.len(),
            1,
            "identical vector (cosine 1.0) must pass the guard"
        );
    }

    fn cx(seed: u8) -> CxId {
        CxId::from_bytes([seed; 16])
    }

    fn sample_hit(cx_id: CxId) -> Hit {
        Hit {
            cx_id,
            score: 0.834,
            rank: 1,
            event_time_secs: None,
            temporal_scores: None,
            causal_confidence: CausalConfidence::Absent,
            causal_gate: None,
            per_lens: vec![PerLensContribution {
                slot: SlotId::new(0),
                rank: 1,
                raw_score: 0.91,
                weight: 0.5,
                contribution: 0.455,
            }],
            cross_terms_used: false,
            guard: None,
            provenance: LedgerRef {
                seq: 42,
                hash: [7; 32],
            },
            provenance_source: ProvenanceSource::Stored,
            freshness: FreshnessTag::fresh(42),
            explain: None,
        }
    }

    fn constellation(cx_id: CxId, dense: Vec<f32>) -> Constellation {
        let mut slots = BTreeMap::new();
        slots.insert(
            SlotId::new(0),
            SlotVector::Dense {
                dim: dense.len() as u32,
                data: dense,
            },
        );
        Constellation {
            cx_id,
            vault_id: VaultId::from_ulid(Ulid::from_bytes([9; 16])),
            panel_version: 1,
            created_at: 1,
            input_ref: InputRef {
                hash: [0; 32],
                pointer: None,
                redacted: false,
            },
            modality: Modality::Text,
            slots,
            scalars: BTreeMap::new(),
            metadata: BTreeMap::new(),
            anchors: Vec::new(),
            provenance: LedgerRef {
                seq: 1,
                hash: [1; 32],
            },
            flags: CxFlags::default(),
        }
    }
}
