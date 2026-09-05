//! No-flatten kernel index over complete typed constellation panels.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::Read;

use calyx_core::{
    CxId, PreparedSlotSimilarity, SimilaritySlotShape, SlotId, SlotSimilarityEvidence, SlotVector,
    prepare_slot_similarity, prepared_slot_similarity_score, slot_ambient_dim, slot_similarity,
    slot_similarity_shape,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::kernel_index::{KernelStore, PANEL_RRF_K};
use crate::{Kernel, LodestarError, Result};

pub const TYPED_PANEL_FORMAT_VERSION: u32 = 1;
pub type TypedPanelVectors = BTreeMap<SlotId, SlotVector>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedSlotContract {
    pub shape: SimilaritySlotShape,
    pub ambient_dim: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypedPanelFusionLane {
    pub slot: SlotId,
    pub similarity: SlotSimilarityEvidence,
    pub rank: usize,
    pub rrf_contribution: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypedPanelFusionHit {
    pub cx_id: CxId,
    pub score: f32,
    pub lanes: Vec<TypedPanelFusionLane>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypedPanelKernelVectorRow {
    pub cx_id: CxId,
    pub vectors: TypedPanelVectors,
}

#[derive(Clone, Debug)]
pub struct TypedPanelKernelIndex {
    pub kernel_id: CxId,
    slots: Vec<SlotId>,
    contracts: BTreeMap<SlotId, TypedSlotContract>,
    rows: Vec<TypedPanelKernelVectorRow>,
}

impl TypedPanelKernelIndex {
    pub fn slots(&self) -> &[SlotId] {
        &self.slots
    }

    pub fn contracts(&self) -> &BTreeMap<SlotId, TypedSlotContract> {
        &self.contracts
    }

    pub fn rows(&self) -> &[TypedPanelKernelVectorRow] {
        &self.rows
    }

    fn from_rows(
        kernel_id: CxId,
        slots: Vec<SlotId>,
        rows: Vec<TypedPanelKernelVectorRow>,
    ) -> Result<Self> {
        let candidates = rows
            .iter()
            .map(|row| (row.cx_id, &row.vectors))
            .collect::<BTreeMap<_, _>>();
        if candidates.len() != rows.len() {
            return invalid("typed panel kernel index contains duplicate constellation ids");
        }
        let contracts = validate_typed_panel_refs(&slots, &candidates)?;
        Ok(Self {
            kernel_id,
            slots,
            contracts,
            rows,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct TypedPanelKernelSnapshot {
    format_version: u32,
    kernel_id: CxId,
    slots: Vec<SlotId>,
    contracts: BTreeMap<SlotId, TypedSlotContract>,
    fusion: String,
    rrf_k: u32,
    rows: Vec<TypedPanelKernelVectorRow>,
}

#[derive(Serialize)]
struct TypedPanelKernelSnapshotRef<'a> {
    format_version: u32,
    kernel_id: CxId,
    slots: &'a [SlotId],
    contracts: &'a BTreeMap<SlotId, TypedSlotContract>,
    fusion: &'a str,
    rrf_k: u32,
    rows: &'a [TypedPanelKernelVectorRow],
}

/// Streamed physical readback of a typed panel index. Every row is parsed,
/// contract-validated, and discarded; only this bounded proof state survives.
#[derive(Clone, Debug, PartialEq)]
pub struct TypedPanelKernelIndexReadback {
    pub kernel_id: CxId,
    pub slots: Vec<SlotId>,
    pub contracts: BTreeMap<SlotId, TypedSlotContract>,
    pub row_count: usize,
}

pub fn build_typed_panel_kernel_index(
    kernel: &Kernel,
    slots: &[SlotId],
    embeddings: &BTreeMap<CxId, TypedPanelVectors>,
) -> Result<TypedPanelKernelIndex> {
    if kernel.members.is_empty() {
        return Err(LodestarError::KernelEmptyResult);
    }
    let rows = kernel
        .members
        .iter()
        .map(|cx_id| {
            let vectors = embeddings
                .get(cx_id)
                .cloned()
                .ok_or(LodestarError::KernelEmbeddingMissing { cx_id: *cx_id })?;
            Ok(TypedPanelKernelVectorRow {
                cx_id: *cx_id,
                vectors,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    TypedPanelKernelIndex::from_rows(kernel.kernel_id, slots.to_vec(), rows)
}

/// Build the terminal typed panel index by moving selected member vectors out
/// of a corpus map that has no remaining consumers.
pub fn build_typed_panel_kernel_index_owned(
    kernel: &Kernel,
    slots: &[SlotId],
    mut embeddings: BTreeMap<CxId, TypedPanelVectors>,
) -> Result<TypedPanelKernelIndex> {
    if kernel.members.is_empty() {
        return Err(LodestarError::KernelEmptyResult);
    }
    let rows = kernel
        .members
        .iter()
        .map(|cx_id| {
            let vectors = embeddings
                .remove(cx_id)
                .ok_or(LodestarError::KernelEmbeddingMissing { cx_id: *cx_id })?;
            Ok(TypedPanelKernelVectorRow {
                cx_id: *cx_id,
                vectors,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    drop(embeddings);
    TypedPanelKernelIndex::from_rows(kernel.kernel_id, slots.to_vec(), rows)
}

pub fn typed_panel_kernel_search(
    index: &TypedPanelKernelIndex,
    query: &TypedPanelVectors,
    top_k: usize,
) -> Result<Vec<TypedPanelFusionHit>> {
    let candidates = index
        .rows
        .iter()
        .map(|row| (row.cx_id, &row.vectors))
        .collect::<BTreeMap<_, _>>();
    rank_typed_panel_top_k_candidate_refs(query, &candidates, &index.slots, top_k)
}

/// Rank every kernel member exactly when the consumer needs only the canonical
/// CxId order, avoiding evidence materialization for rows that will be
/// discarded immediately.
pub fn typed_panel_kernel_rank_ids(
    index: &TypedPanelKernelIndex,
    query: &TypedPanelVectors,
) -> Result<Vec<CxId>> {
    validate_typed_panel_vector("query", query, &index.slots, &index.contracts)?;
    let candidate_rows = index
        .rows
        .iter()
        .map(|row| (row.cx_id, &row.vectors))
        .collect::<Vec<_>>();
    let prepared_candidates = prepare_typed_panel_rows(&candidate_rows, &index.slots)?;
    let query_rows = [(index.kernel_id, query)];
    let prepared_query = prepare_typed_panel_rows(&query_rows, &index.slots)?
        .pop()
        .expect("one prepared typed panel query");
    let candidate_refs = prepared_candidates.iter().collect::<Vec<_>>();
    Ok(rank_typed_panel_top_k_prepared_ids(
        &prepared_query,
        &candidate_refs,
        index.slots.len(),
        prepared_candidates.len(),
        None,
    )?
    .ids)
}

/// Exact resident ranking with complete lane evidence for the winning kernel
/// member in the same pass. This keeps the canonical full id order required by
/// path derivation while avoiding a second unprepared all-member rank merely to
/// reconstruct the admission proof.
pub fn typed_panel_kernel_rank_with_nearest(
    index: &TypedPanelKernelIndex,
    query: &TypedPanelVectors,
) -> Result<(Vec<CxId>, TypedPanelFusionHit)> {
    validate_typed_panel_vector("query", query, &index.slots, &index.contracts)?;
    if index.rows.is_empty() {
        return Err(LodestarError::KernelEmptyResult);
    }
    let candidate_rows = index
        .rows
        .iter()
        .map(|row| (row.cx_id, &row.vectors))
        .collect::<Vec<_>>();
    let prepared_candidates = prepare_typed_panel_rows(&candidate_rows, &index.slots)?;
    let query_rows = [(index.kernel_id, query)];
    let prepared_query = prepare_typed_panel_rows(&query_rows, &index.slots)?
        .pop()
        .expect("one prepared typed panel query");
    let slot_count = index.slots.len();
    let rank_cells = prepared_candidates
        .len()
        .checked_mul(slot_count)
        .ok_or_else(|| LodestarError::KernelIndexCodec {
            detail: "typed panel resident rank evidence size overflow".to_string(),
        })?;
    let mut ranks = vec![0_usize; rank_cells];
    let mut fused_scores = vec![0.0_f32; prepared_candidates.len()];
    let normalizer = (PANEL_RRF_K as f32 + 1.0) / slot_count as f32;
    let mut lane = vec![(0_usize, 0.0_f32); prepared_candidates.len()];

    for slot_offset in 0..slot_count {
        let query_vector = &prepared_query.vectors[slot_offset];
        lane.par_iter_mut()
            .enumerate()
            .try_for_each(|(candidate_offset, entry)| {
                *entry = (
                    candidate_offset,
                    prepared_slot_similarity_score(
                        query_vector,
                        &prepared_candidates[candidate_offset].vectors[slot_offset],
                    )
                    .map_err(LodestarError::from)?,
                );
                Ok::<(), LodestarError>(())
            })?;
        lane.par_sort_unstable_by(|left, right| {
            right.1.total_cmp(&left.1).then_with(|| {
                prepared_candidates[left.0]
                    .cx_id
                    .cmp(&prepared_candidates[right.0].cx_id)
            })
        });
        for (rank_offset, (candidate_offset, _score)) in lane.iter().enumerate() {
            let rank = rank_offset + 1;
            ranks[*candidate_offset * slot_count + slot_offset] = rank;
            fused_scores[*candidate_offset] += normalizer / (PANEL_RRF_K as f32 + rank as f32);
        }
    }

    let mut fused_order = (0..prepared_candidates.len()).collect::<Vec<_>>();
    fused_order.par_sort_unstable_by(|&left, &right| {
        fused_scores[right]
            .total_cmp(&fused_scores[left])
            .then_with(|| {
                prepared_candidates[left]
                    .cx_id
                    .cmp(&prepared_candidates[right].cx_id)
            })
    });
    let ids = fused_order
        .iter()
        .map(|&offset| prepared_candidates[offset].cx_id)
        .collect::<Vec<_>>();
    let winner_offset = fused_order[0];
    let winner = &index.rows[winner_offset];
    let lanes = index
        .slots
        .iter()
        .enumerate()
        .map(|(slot_offset, &slot)| {
            let similarity = slot_similarity(
                query.get(&slot).expect("validated query slot"),
                winner.vectors.get(&slot).expect("validated candidate slot"),
            )
            .map_err(LodestarError::from)?;
            let rank = ranks[winner_offset * slot_count + slot_offset];
            Ok(TypedPanelFusionLane {
                slot,
                similarity,
                rank,
                rrf_contribution: normalizer / (PANEL_RRF_K as f32 + rank as f32),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let score = lanes.iter().map(|lane| lane.rrf_contribution).sum::<f32>();
    if score.to_bits() != fused_scores[winner_offset].to_bits() {
        return invalid(format!(
            "typed panel resident rank reconstruction drift for {}: fused={} reconstructed={score}",
            winner.cx_id, fused_scores[winner_offset]
        ));
    }
    Ok((
        ids,
        TypedPanelFusionHit {
            cx_id: winner.cx_id,
            score,
            lanes,
        },
    ))
}

pub fn rank_typed_panel_candidates(
    query: &TypedPanelVectors,
    candidates: &BTreeMap<CxId, TypedPanelVectors>,
    slots: &[SlotId],
) -> Result<Vec<TypedPanelFusionHit>> {
    let refs = candidates
        .iter()
        .map(|(id, vectors)| (*id, vectors))
        .collect::<BTreeMap<_, _>>();
    rank_typed_panel_candidate_refs(query, &refs, slots)
}

pub fn rank_typed_panel_top_k_candidates(
    query: &TypedPanelVectors,
    candidates: &BTreeMap<CxId, TypedPanelVectors>,
    slots: &[SlotId],
    top_k: usize,
) -> Result<Vec<TypedPanelFusionHit>> {
    let refs = candidates
        .iter()
        .map(|(id, vectors)| (*id, vectors))
        .collect::<BTreeMap<_, _>>();
    rank_typed_panel_top_k_candidate_refs(query, &refs, slots, top_k)
}

pub fn rank_typed_panel_top_k_candidate_refs(
    query: &TypedPanelVectors,
    candidates: &BTreeMap<CxId, &TypedPanelVectors>,
    slots: &[SlotId],
    top_k: usize,
) -> Result<Vec<TypedPanelFusionHit>> {
    rank_typed_panel_top_k_candidate_refs_inner(query, candidates, slots, top_k)
}

pub fn rank_typed_panel_candidate_refs(
    query: &TypedPanelVectors,
    candidates: &BTreeMap<CxId, &TypedPanelVectors>,
    slots: &[SlotId],
) -> Result<Vec<TypedPanelFusionHit>> {
    let contracts = validate_typed_panel_refs(slots, candidates)?;
    validate_typed_panel_vector("query", query, slots, &contracts)?;
    let mut evidence = candidates
        .keys()
        .map(|id| (*id, Vec::with_capacity(slots.len())))
        .collect::<BTreeMap<_, _>>();
    let normalizer = (PANEL_RRF_K as f32 + 1.0) / slots.len() as f32;
    for &slot in slots {
        let query_vector = query.get(&slot).expect("validated query slot");
        let lane_results = candidates
            .par_iter()
            .map(|(id, vectors)| {
                let candidate = vectors.get(&slot).expect("validated candidate slot");
                slot_similarity(query_vector, candidate)
                    .map(|similarity| (*id, similarity))
                    .map_err(LodestarError::from)
            })
            .collect::<Vec<_>>();
        let mut lane = lane_results.into_iter().collect::<Result<Vec<_>>>()?;
        lane.par_sort_unstable_by(|left, right| {
            right
                .1
                .score
                .total_cmp(&left.1.score)
                .then_with(|| left.0.cmp(&right.0))
        });
        for (offset, (id, similarity)) in lane.into_iter().enumerate() {
            let rank = offset + 1;
            evidence
                .get_mut(&id)
                .expect("candidate evidence")
                .push(TypedPanelFusionLane {
                    slot,
                    similarity,
                    rank,
                    rrf_contribution: normalizer / (PANEL_RRF_K as f32 + rank as f32),
                });
        }
    }
    let mut hits = evidence
        .into_iter()
        .map(|(cx_id, lanes)| TypedPanelFusionHit {
            cx_id,
            score: lanes.iter().map(|lane| lane.rrf_contribution).sum(),
            lanes,
        })
        .collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.cx_id.cmp(&right.cx_id))
    });
    Ok(hits)
}

struct PreparedTypedPanelRow<'a> {
    cx_id: CxId,
    vectors: Vec<PreparedSlotSimilarity<'a>>,
}

/// Validated full-corpus rows and exact block norms shared by every recall and
/// refinement pass in one immutable kernel build.
pub struct PreparedTypedPanelCorpus<'a> {
    full: &'a BTreeMap<CxId, TypedPanelVectors>,
    slots: Vec<SlotId>,
    contracts: BTreeMap<SlotId, TypedSlotContract>,
    rows: Vec<PreparedTypedPanelRow<'a>>,
}

struct PreparedTypedPanelRecallQuery {
    query_offset: usize,
    full_ids: Vec<CxId>,
    lane_prefix_ids: BTreeSet<CxId>,
}

/// Exact full-corpus winners and per-lane support prefixes shared by every
/// recall/refinement gate for one immutable corpus and parameter contract.
///
/// Full-corpus ranking is the dominant kernel-build cost. The reference keeps
/// only bounded CxId proof state, so refined kernels can be compared against
/// the exact same full result without scoring and sorting all corpus rows
/// again.
pub struct PreparedTypedPanelRecallReference<'a> {
    full: &'a BTreeMap<CxId, TypedPanelVectors>,
    corpus_len: usize,
    slots: Vec<SlotId>,
    contracts: BTreeMap<SlotId, TypedSlotContract>,
    params: crate::RecallTestParams,
    lane_depth: usize,
    held_out: Vec<CxId>,
    queries: Vec<PreparedTypedPanelRecallQuery>,
}

struct TypedPanelTopKIdsResult {
    ids: Vec<CxId>,
    lane_prefix_ids: BTreeSet<CxId>,
}

#[derive(Default)]
struct TypedPanelRankWorkspace {
    fused_scores: Vec<f32>,
    lane_prefix: Vec<bool>,
    lane: Vec<(usize, f32)>,
    fused_order: Vec<usize>,
}

fn prepare_typed_panel_rows<'a>(
    candidate_rows: &[(CxId, &'a TypedPanelVectors)],
    slots: &[SlotId],
) -> Result<Vec<PreparedTypedPanelRow<'a>>> {
    candidate_rows
        .par_iter()
        .map(|(cx_id, vectors)| {
            let prepared = slots
                .iter()
                .map(|slot| {
                    let vector = vectors.get(slot).ok_or_else(|| {
                        invalid_error(format!("candidate {cx_id} missing prepared slot {slot}"))
                    })?;
                    prepare_slot_similarity(vector).map_err(LodestarError::from)
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(PreparedTypedPanelRow {
                cx_id: *cx_id,
                vectors: prepared,
            })
        })
        .collect()
}

pub fn prepare_typed_panel_corpus<'a>(
    full: &'a BTreeMap<CxId, TypedPanelVectors>,
    slots: &[SlotId],
) -> Result<PreparedTypedPanelCorpus<'a>> {
    let refs = full
        .iter()
        .map(|(cx_id, vectors)| (*cx_id, vectors))
        .collect::<BTreeMap<_, _>>();
    let contracts = validate_typed_panel_refs(slots, &refs)?;
    let candidate_rows = refs
        .iter()
        .map(|(cx_id, vectors)| (*cx_id, *vectors))
        .collect::<Vec<_>>();
    let rows = prepare_typed_panel_rows(&candidate_rows, slots)?;
    Ok(PreparedTypedPanelCorpus {
        full,
        slots: slots.to_vec(),
        contracts,
        rows,
    })
}

fn rank_typed_panel_top_k_prepared_ids(
    query: &PreparedTypedPanelRow<'_>,
    candidate_rows: &[&PreparedTypedPanelRow<'_>],
    slot_count: usize,
    top_k: usize,
    lane_depth: Option<usize>,
) -> Result<TypedPanelTopKIdsResult> {
    rank_typed_panel_top_k_prepared_ids_with_workspace(
        query,
        candidate_rows,
        slot_count,
        top_k,
        lane_depth,
        &mut TypedPanelRankWorkspace::default(),
    )
}

fn rank_typed_panel_top_k_prepared_ids_with_workspace(
    query: &PreparedTypedPanelRow<'_>,
    candidate_rows: &[&PreparedTypedPanelRow<'_>],
    slot_count: usize,
    top_k: usize,
    lane_depth: Option<usize>,
    workspace: &mut TypedPanelRankWorkspace,
) -> Result<TypedPanelTopKIdsResult> {
    if top_k == 0 {
        return invalid("typed panel top-k must be positive");
    }
    workspace.fused_scores.resize(candidate_rows.len(), 0.0);
    workspace.fused_scores.fill(0.0);
    if lane_depth.is_some() {
        workspace.lane_prefix.resize(candidate_rows.len(), false);
        workspace.lane_prefix.fill(false);
    } else {
        workspace.lane_prefix.clear();
    }
    let normalizer = (PANEL_RRF_K as f32 + 1.0) / slot_count as f32;
    workspace.lane.resize(candidate_rows.len(), (0, 0.0));
    for (candidate_offset, entry) in workspace.lane.iter_mut().enumerate() {
        *entry = (candidate_offset, 0.0);
    }

    for slot_offset in 0..slot_count {
        let query_vector = &query.vectors[slot_offset];
        workspace
            .lane
            .par_iter_mut()
            .try_for_each(|(candidate_offset, score)| {
                *score = prepared_slot_similarity_score(
                    query_vector,
                    &candidate_rows[*candidate_offset].vectors[slot_offset],
                )
                .map_err(LodestarError::from)?;
                Ok::<(), LodestarError>(())
            })?;
        workspace.lane.par_sort_unstable_by(|left, right| {
            right.1.total_cmp(&left.1).then_with(|| {
                candidate_rows[left.0]
                    .cx_id
                    .cmp(&candidate_rows[right.0].cx_id)
            })
        });
        for (rank_offset, (candidate_offset, _)) in workspace.lane.iter().enumerate() {
            let rank = rank_offset + 1;
            workspace.fused_scores[*candidate_offset] +=
                normalizer / (PANEL_RRF_K as f32 + rank as f32);
            if lane_depth.is_some_and(|depth| rank <= depth) {
                workspace.lane_prefix[*candidate_offset] = true;
            }
        }
    }

    workspace.fused_order.resize(candidate_rows.len(), 0);
    for (candidate_offset, entry) in workspace.fused_order.iter_mut().enumerate() {
        *entry = candidate_offset;
    }
    workspace.fused_order.par_sort_unstable_by(|&left, &right| {
        workspace.fused_scores[right]
            .total_cmp(&workspace.fused_scores[left])
            .then_with(|| candidate_rows[left].cx_id.cmp(&candidate_rows[right].cx_id))
    });
    let ids = workspace
        .fused_order
        .iter()
        .take(top_k.min(workspace.fused_order.len()))
        .map(|&candidate_offset| candidate_rows[candidate_offset].cx_id)
        .collect();
    let lane_prefix_ids = lane_depth.map_or_else(BTreeSet::new, |_| {
        candidate_rows
            .iter()
            .zip(&workspace.lane_prefix)
            .filter_map(|(row, included)| included.then_some(row.cx_id))
            .collect()
    });
    Ok(TypedPanelTopKIdsResult {
        ids,
        lane_prefix_ids,
    })
}

fn rank_typed_panel_top_k_candidate_refs_inner(
    query: &TypedPanelVectors,
    candidates: &BTreeMap<CxId, &TypedPanelVectors>,
    slots: &[SlotId],
    top_k: usize,
) -> Result<Vec<TypedPanelFusionHit>> {
    if top_k == 0 {
        return invalid("typed panel top-k must be positive");
    }
    let contracts = validate_typed_panel_refs(slots, candidates)?;
    validate_typed_panel_vector("query", query, slots, &contracts)?;
    let candidate_rows = candidates
        .iter()
        .map(|(cx_id, vectors)| (*cx_id, *vectors))
        .collect::<Vec<_>>();
    let rank_cells = candidate_rows
        .len()
        .checked_mul(slots.len())
        .ok_or_else(|| LodestarError::KernelIndexCodec {
            detail: "typed panel dense rank workspace size overflow".to_string(),
        })?;
    let mut ranks = vec![0_usize; rank_cells];
    let mut fused_scores = vec![0.0_f32; candidate_rows.len()];
    let normalizer = (PANEL_RRF_K as f32 + 1.0) / slots.len() as f32;

    for (slot_offset, &slot) in slots.iter().enumerate() {
        let query_vector = query.get(&slot).expect("validated query slot");
        let lane_results = candidate_rows
            .par_iter()
            .enumerate()
            .map(|(candidate_offset, (cx_id, vectors))| {
                let candidate = vectors.get(&slot).expect("validated candidate slot");
                slot_similarity(query_vector, candidate)
                    .map(|similarity| (candidate_offset, *cx_id, similarity))
                    .map_err(LodestarError::from)
            })
            .collect::<Vec<_>>();
        let mut lane = lane_results.into_iter().collect::<Result<Vec<_>>>()?;
        lane.par_sort_unstable_by(|left, right| {
            right
                .2
                .score
                .total_cmp(&left.2.score)
                .then_with(|| left.1.cmp(&right.1))
        });
        for (rank_offset, (candidate_offset, _, _)) in lane.into_iter().enumerate() {
            let rank = rank_offset + 1;
            ranks[candidate_offset * slots.len() + slot_offset] = rank;
            fused_scores[candidate_offset] += normalizer / (PANEL_RRF_K as f32 + rank as f32);
        }
    }

    let mut fused_order = (0..candidate_rows.len()).collect::<Vec<_>>();
    fused_order.par_sort_unstable_by(|&left, &right| {
        fused_scores[right]
            .total_cmp(&fused_scores[left])
            .then_with(|| candidate_rows[left].0.cmp(&candidate_rows[right].0))
    });
    fused_order.truncate(top_k.min(fused_order.len()));
    let hits = fused_order
        .into_iter()
        .map(|candidate_offset| {
            let (cx_id, vectors) = candidate_rows[candidate_offset];
            let lanes = slots
                .iter()
                .enumerate()
                .map(|(slot_offset, &slot)| {
                    let query_vector = query.get(&slot).expect("validated query slot");
                    let candidate = vectors.get(&slot).expect("validated candidate slot");
                    let similarity =
                        slot_similarity(query_vector, candidate).map_err(LodestarError::from)?;
                    let rank = ranks[candidate_offset * slots.len() + slot_offset];
                    Ok(TypedPanelFusionLane {
                        slot,
                        similarity,
                        rank,
                        rrf_contribution: normalizer / (PANEL_RRF_K as f32 + rank as f32),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let reconstructed_score = lanes.iter().map(|lane| lane.rrf_contribution).sum::<f32>();
            if reconstructed_score.to_bits() != fused_scores[candidate_offset].to_bits() {
                return invalid(format!(
                    "typed panel dense rank reconstruction drift for {cx_id}: fused={} reconstructed={reconstructed_score}",
                    fused_scores[candidate_offset]
                ));
            }
            Ok(TypedPanelFusionHit {
                cx_id,
                score: reconstructed_score,
                lanes,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(hits)
}

pub fn write_typed_panel_kernel_index(
    index: &TypedPanelKernelIndex,
    store: &dyn KernelStore,
) -> Result<()> {
    let snapshot = TypedPanelKernelSnapshotRef {
        format_version: TYPED_PANEL_FORMAT_VERSION,
        kernel_id: index.kernel_id,
        slots: &index.slots,
        contracts: &index.contracts,
        fusion: "typed_block_rrf",
        rrf_k: PANEL_RRF_K,
        rows: &index.rows,
    };
    store.install_index_streaming(index.kernel_id, &|writer| {
        serde_json::to_writer(writer, &snapshot).map_err(codec_error)
    })
}

pub fn load_typed_panel_kernel_index(
    kernel_id: CxId,
    store: &dyn KernelStore,
) -> Result<TypedPanelKernelIndex> {
    let Some(reader) = store.read_index_reader(kernel_id)? else {
        return Err(LodestarError::KernelIndexNotFound { kernel_id });
    };
    load_typed_panel_kernel_index_reader(kernel_id, reader)
}

/// Load, contract-validate, byte-count, and SHA-256 verify one physical typed
/// index in a single sequential pass. Long-lived consumers use this to pin the
/// exact immutable artifact without first hashing all bytes and then parsing
/// the same multi-GB file a second time.
pub fn load_typed_panel_kernel_index_verified(
    kernel_id: CxId,
    store: &dyn KernelStore,
    expected_bytes: u64,
    expected_sha256: &str,
) -> Result<TypedPanelKernelIndex> {
    let Some(reader) = store.read_index_reader(kernel_id)? else {
        return Err(LodestarError::KernelIndexNotFound { kernel_id });
    };
    let mut verified = Sha256Reader::new(reader);
    let index = load_typed_panel_kernel_index_reader(kernel_id, &mut verified)?;
    let (observed_bytes, observed_sha256) = verified.finish();
    if observed_bytes != expected_bytes || observed_sha256 != expected_sha256 {
        return Err(LodestarError::KernelIndexCodec {
            detail: format!(
                "typed panel kernel index physical digest mismatch: expected bytes={expected_bytes} sha256={expected_sha256}, observed bytes={observed_bytes} sha256={observed_sha256}"
            ),
        });
    }
    Ok(index)
}

fn load_typed_panel_kernel_index_reader(
    kernel_id: CxId,
    reader: impl Read,
) -> Result<TypedPanelKernelIndex> {
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    let snapshot = TypedPanelKernelSnapshot::deserialize(&mut deserializer).map_err(codec_error)?;
    deserializer.end().map_err(codec_error)?;
    if snapshot.format_version != TYPED_PANEL_FORMAT_VERSION
        || snapshot.kernel_id != kernel_id
        || snapshot.fusion != "typed_block_rrf"
        || snapshot.rrf_k != PANEL_RRF_K
    {
        return Err(LodestarError::KernelIndexCodec {
            detail: "invalid typed panel kernel index contract".to_string(),
        });
    }
    let index =
        TypedPanelKernelIndex::from_rows(snapshot.kernel_id, snapshot.slots, snapshot.rows)?;
    if index.contracts != snapshot.contracts {
        return Err(LodestarError::KernelIndexCodec {
            detail: "typed panel contracts differ from physical row readback".to_string(),
        });
    }
    Ok(index)
}

struct Sha256Reader<R> {
    inner: R,
    hasher: Sha256,
    bytes: u64,
}

impl<R> Sha256Reader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            bytes: 0,
        }
    }

    fn finish(self) -> (u64, String) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let digest = self.hasher.finalize();
        let mut encoded = String::with_capacity(digest.len() * 2);
        for byte in digest {
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
        (self.bytes, encoded)
    }
}

impl<R: Read> Read for Sha256Reader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.hasher.update(&buffer[..read]);
        self.bytes = self.bytes.saturating_add(read as u64);
        Ok(read)
    }
}

/// Verify an on-disk typed panel index without materializing a second complete
/// copy of its rows.
pub fn verify_typed_panel_kernel_index(
    kernel_id: CxId,
    store: &dyn KernelStore,
) -> Result<TypedPanelKernelIndexReadback> {
    let Some(reader) = store.read_index_reader(kernel_id)? else {
        return Err(LodestarError::KernelIndexNotFound { kernel_id });
    };
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    let readback =
        TypedPanelKernelIndexReadback::deserialize(&mut deserializer).map_err(codec_error)?;
    deserializer.end().map_err(codec_error)?;
    if readback.kernel_id != kernel_id {
        return Err(LodestarError::KernelIndexCodec {
            detail: format!(
                "typed panel kernel index kernel id {} did not match requested {kernel_id}",
                readback.kernel_id
            ),
        });
    }
    Ok(readback)
}

/// Stream-validate every row while independently counting and hashing the same
/// physical bytes. This collapses the publication readback and artifact digest
/// into one pass without weakening either proof.
pub fn verify_typed_panel_kernel_index_physical(
    kernel_id: CxId,
    store: &dyn KernelStore,
) -> Result<(TypedPanelKernelIndexReadback, u64, String)> {
    let Some(reader) = store.read_index_reader(kernel_id)? else {
        return Err(LodestarError::KernelIndexNotFound { kernel_id });
    };
    let mut verified = Sha256Reader::new(reader);
    let mut deserializer = serde_json::Deserializer::from_reader(&mut verified);
    let readback =
        TypedPanelKernelIndexReadback::deserialize(&mut deserializer).map_err(codec_error)?;
    deserializer.end().map_err(codec_error)?;
    drop(deserializer);
    if readback.kernel_id != kernel_id {
        return Err(LodestarError::KernelIndexCodec {
            detail: format!(
                "typed panel kernel index kernel id {} did not match requested {kernel_id}",
                readback.kernel_id
            ),
        });
    }
    let (bytes, sha256) = verified.finish();
    Ok((readback, bytes, sha256))
}

impl<'de> Deserialize<'de> for TypedPanelKernelIndexReadback {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(TypedPanelReadbackVisitor)
    }
}

struct TypedPanelReadbackVisitor;

impl<'de> serde::de::Visitor<'de> for TypedPanelReadbackVisitor {
    type Value = TypedPanelKernelIndexReadback;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a typed panel kernel index object")
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        use serde::de::Error as _;
        let mut format_version: Option<u32> = None;
        let mut kernel_id: Option<CxId> = None;
        let mut slots: Option<Vec<SlotId>> = None;
        let mut contracts: Option<BTreeMap<SlotId, TypedSlotContract>> = None;
        let mut fusion: Option<String> = None;
        let mut rrf_k: Option<u32> = None;
        let mut row_count: Option<usize> = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "format_version" => {
                    if format_version.is_some() {
                        return Err(A::Error::duplicate_field("format_version"));
                    }
                    let observed = map.next_value()?;
                    if observed != TYPED_PANEL_FORMAT_VERSION {
                        return Err(A::Error::custom(format!(
                            "unsupported typed panel kernel index format version {observed}"
                        )));
                    }
                    format_version = Some(observed);
                }
                "kernel_id" => {
                    if kernel_id.is_some() {
                        return Err(A::Error::duplicate_field("kernel_id"));
                    }
                    kernel_id = Some(map.next_value()?);
                }
                "slots" => {
                    if slots.is_some() {
                        return Err(A::Error::duplicate_field("slots"));
                    }
                    slots = Some(map.next_value()?);
                }
                "contracts" => {
                    if contracts.is_some() {
                        return Err(A::Error::duplicate_field("contracts"));
                    }
                    contracts = Some(map.next_value()?);
                }
                "fusion" => {
                    if fusion.is_some() {
                        return Err(A::Error::duplicate_field("fusion"));
                    }
                    let observed: String = map.next_value()?;
                    if observed != "typed_block_rrf" {
                        return Err(A::Error::custom(format!(
                            "unsupported typed panel fusion {observed}"
                        )));
                    }
                    fusion = Some(observed);
                }
                "rrf_k" => {
                    if rrf_k.is_some() {
                        return Err(A::Error::duplicate_field("rrf_k"));
                    }
                    let observed = map.next_value()?;
                    if observed != PANEL_RRF_K {
                        return Err(A::Error::custom(format!(
                            "unsupported typed panel rrf_k {observed}"
                        )));
                    }
                    rrf_k = Some(observed);
                }
                "rows" => {
                    if row_count.is_some() {
                        return Err(A::Error::duplicate_field("rows"));
                    }
                    let slots = slots.as_ref().ok_or_else(|| {
                        A::Error::custom("typed panel index rows appeared before slots")
                    })?;
                    let contracts = contracts.as_ref().ok_or_else(|| {
                        A::Error::custom("typed panel index rows appeared before contracts")
                    })?;
                    row_count =
                        Some(map.next_value_seed(TypedPanelRowStreamSeed { slots, contracts })?);
                }
                other => {
                    return Err(A::Error::custom(format!(
                        "unexpected typed panel kernel index field {other}"
                    )));
                }
            }
        }
        let format_version =
            format_version.ok_or_else(|| A::Error::missing_field("format_version"))?;
        if format_version != TYPED_PANEL_FORMAT_VERSION {
            return Err(A::Error::custom(format!(
                "unsupported typed panel kernel index format version {format_version}"
            )));
        }
        let fusion = fusion.ok_or_else(|| A::Error::missing_field("fusion"))?;
        if fusion != "typed_block_rrf" {
            return Err(A::Error::custom(format!(
                "unsupported typed panel fusion {fusion}"
            )));
        }
        let rrf_k = rrf_k.ok_or_else(|| A::Error::missing_field("rrf_k"))?;
        if rrf_k != PANEL_RRF_K {
            return Err(A::Error::custom(format!(
                "unsupported typed panel rrf_k {rrf_k}"
            )));
        }
        Ok(TypedPanelKernelIndexReadback {
            kernel_id: kernel_id.ok_or_else(|| A::Error::missing_field("kernel_id"))?,
            slots: slots.ok_or_else(|| A::Error::missing_field("slots"))?,
            contracts: contracts.ok_or_else(|| A::Error::missing_field("contracts"))?,
            row_count: row_count.ok_or_else(|| A::Error::missing_field("rows"))?,
        })
    }
}

struct TypedPanelRowStreamSeed<'a> {
    slots: &'a [SlotId],
    contracts: &'a BTreeMap<SlotId, TypedSlotContract>,
}

impl<'de> serde::de::DeserializeSeed<'de> for TypedPanelRowStreamSeed<'_> {
    type Value = usize;

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<usize, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_seq(self)
    }
}

impl<'de> serde::de::Visitor<'de> for TypedPanelRowStreamSeed<'_> {
    type Value = usize;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a sequence of typed panel kernel vector rows")
    }

    fn visit_seq<A>(self, mut seq: A) -> std::result::Result<usize, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        use serde::de::Error as _;
        if self.slots.len() < 2
            || !self.slots.windows(2).all(|pair| pair[0] < pair[1])
            || self.contracts.len() != self.slots.len()
            || self
                .slots
                .iter()
                .any(|slot| !self.contracts.contains_key(slot))
            || self
                .contracts
                .values()
                .any(|contract| contract.ambient_dim == 0)
        {
            return Err(A::Error::custom(
                "typed panel index contracts differ from the declared slot contract",
            ));
        }
        let mut seen = BTreeSet::new();
        let mut count = 0_usize;
        while let Some(row) = seq.next_element::<TypedPanelKernelVectorRow>()? {
            if !seen.insert(row.cx_id) {
                return Err(A::Error::custom(format!(
                    "typed panel kernel index contains duplicate constellation id {}",
                    row.cx_id
                )));
            }
            validate_typed_panel_vector(
                &format!("row {}", row.cx_id),
                &row.vectors,
                self.slots,
                self.contracts,
            )
            .map_err(|error| A::Error::custom(error.to_string()))?;
            count += 1;
        }
        if count == 0 {
            return Err(A::Error::custom("typed panel kernel index has no rows"));
        }
        Ok(count)
    }
}

pub fn typed_panel_kernel_recall_test(
    index: &TypedPanelKernelIndex,
    full: &BTreeMap<CxId, TypedPanelVectors>,
    params: &crate::RecallTestParams,
    corpus_name: &str,
) -> Result<crate::RecallReport> {
    let prepared = prepare_typed_panel_corpus(full, &index.slots)?;
    typed_panel_kernel_recall_test_prepared(index, &prepared, params, corpus_name)
}

pub fn typed_panel_kernel_recall_test_prepared(
    index: &TypedPanelKernelIndex,
    prepared: &PreparedTypedPanelCorpus<'_>,
    params: &crate::RecallTestParams,
    corpus_name: &str,
) -> Result<crate::RecallReport> {
    if prepared.slots != index.slots || prepared.contracts != index.contracts {
        return invalid("full corpus typed panel contracts differ from the kernel index");
    }
    let kernel_candidate_rows = index
        .rows
        .iter()
        .map(|row| (row.cx_id, &row.vectors))
        .collect::<Vec<_>>();
    let prepared_kernel = prepare_typed_panel_rows(&kernel_candidate_rows, &index.slots)?;
    let kernel_candidates = prepared_kernel.iter().collect::<Vec<_>>();
    let reference = prepare_typed_panel_recall_reference(prepared, params, 0)?;
    typed_panel_kernel_recall_against_reference_candidates(
        prepared,
        &kernel_candidates,
        &reference,
        corpus_name,
    )
}

/// Measure exact recall against kernel members already resident in the prepared
/// full corpus, without cloning their vector payloads into an intermediate
/// index.
pub fn typed_panel_kernel_recall_test_members_prepared(
    kernel: &Kernel,
    prepared: &PreparedTypedPanelCorpus<'_>,
    params: &crate::RecallTestParams,
    corpus_name: &str,
) -> Result<crate::RecallReport> {
    let reference = prepare_typed_panel_recall_reference(prepared, params, 0)?;
    typed_panel_kernel_recall_test_members_against_reference(
        kernel,
        prepared,
        &reference,
        corpus_name,
    )
}

/// Compare a kernel against a previously sealed exact full-corpus reference.
/// This is equivalent to recomputing the full side because the reference is
/// contract-bound to the corpus, slots, held-out set, and parameters.
pub fn typed_panel_kernel_recall_test_members_against_reference(
    kernel: &Kernel,
    prepared: &PreparedTypedPanelCorpus<'_>,
    reference: &PreparedTypedPanelRecallReference<'_>,
    corpus_name: &str,
) -> Result<crate::RecallReport> {
    if kernel.members.is_empty() {
        return Err(LodestarError::KernelEmptyResult);
    }
    let mut seen = BTreeSet::new();
    let mut kernel_candidates = Vec::with_capacity(kernel.members.len());
    for cx_id in &kernel.members {
        if !seen.insert(*cx_id) {
            return invalid(format!(
                "kernel recall members contain duplicate constellation id {cx_id}"
            ));
        }
        let offset = prepared
            .rows
            .binary_search_by_key(cx_id, |row| row.cx_id)
            .map_err(|_| LodestarError::KernelEmbeddingMissing { cx_id: *cx_id })?;
        kernel_candidates.push(&prepared.rows[offset]);
    }
    typed_panel_kernel_recall_against_reference_candidates(
        prepared,
        &kernel_candidates,
        reference,
        corpus_name,
    )
}

/// Score and sort the immutable full corpus once for all recall/refinement
/// consumers. `lane_depth=0` records fused winners only; a positive depth also
/// records the exact prefix of every lane used by rank-stabilization support.
pub fn prepare_typed_panel_recall_reference<'a>(
    prepared: &PreparedTypedPanelCorpus<'a>,
    params: &crate::RecallTestParams,
    lane_depth: usize,
) -> Result<PreparedTypedPanelRecallReference<'a>> {
    validate_recall_params(params, prepared.full)?;
    let held_out = held_out(prepared.full, params.held_out_fraction, params.rng_seed);
    if held_out.is_empty() {
        return Err(LodestarError::RecallEmptyCorpus);
    }
    let full_candidates = prepared.rows.iter().collect::<Vec<_>>();
    let mut queries = Vec::with_capacity(held_out.len());
    let mut workspace = TypedPanelRankWorkspace::default();
    for id in &held_out {
        let query_offset = prepared
            .rows
            .binary_search_by_key(id, |row| row.cx_id)
            .map_err(|_| {
                invalid_error(format!("held-out typed panel row {id} was not prepared"))
            })?;
        let ranked = rank_typed_panel_top_k_prepared_ids_with_workspace(
            &prepared.rows[query_offset],
            &full_candidates,
            prepared.slots.len(),
            params.top_k,
            (lane_depth > 0).then_some(lane_depth),
            &mut workspace,
        )?;
        queries.push(PreparedTypedPanelRecallQuery {
            query_offset,
            full_ids: ranked.ids,
            lane_prefix_ids: ranked.lane_prefix_ids,
        });
    }
    Ok(PreparedTypedPanelRecallReference {
        full: prepared.full,
        corpus_len: prepared.full.len(),
        slots: prepared.slots.clone(),
        contracts: prepared.contracts.clone(),
        params: params.clone(),
        lane_depth,
        held_out,
        queries,
    })
}

fn typed_panel_kernel_recall_against_reference_candidates(
    prepared: &PreparedTypedPanelCorpus<'_>,
    kernel_candidates: &[&PreparedTypedPanelRow<'_>],
    reference: &PreparedTypedPanelRecallReference<'_>,
    corpus_name: &str,
) -> Result<crate::RecallReport> {
    validate_recall_reference(prepared, reference)?;
    if kernel_candidates.is_empty() {
        return Err(LodestarError::KernelEmptyResult);
    }
    let mut total = 0.0_f32;
    let mut workspace = TypedPanelRankWorkspace::default();
    for query in &reference.queries {
        let kernel_ids = rank_typed_panel_top_k_prepared_ids_with_workspace(
            &prepared.rows[query.query_offset],
            kernel_candidates,
            prepared.slots.len(),
            reference.params.top_k,
            None,
            &mut workspace,
        )?
        .ids
        .into_iter()
        .collect::<BTreeSet<_>>();
        let overlap = query
            .full_ids
            .iter()
            .filter(|id| kernel_ids.contains(id))
            .count();
        total += overlap as f32 / query.full_ids.len() as f32;
    }
    let ratio = total / reference.held_out.len() as f32;
    Ok(crate::RecallReport {
        kernel_only: ratio,
        full: 1.0,
        ratio,
        approx_factor: 1.0,
        tau_star_estimate: 0,
        tau_star_exact: true,
        recall_test_params: Some(reference.params.clone()),
        corpus_name: Some(corpus_name.to_string()),
        n_queries_tested: reference.held_out.len(),
        held_out: reference.held_out.clone(),
        warning: (ratio < reference.params.min_recall_ratio).then(|| {
            format!(
                "{}: ratio={ratio:.6} min={:.6}",
                crate::recall_test::CALYX_KERNEL_RECALL_BELOW_GATE,
                reference.params.min_recall_ratio
            )
        }),
    })
}

pub fn typed_panel_kernel_recall_gate(
    index: &TypedPanelKernelIndex,
    full: &BTreeMap<CxId, TypedPanelVectors>,
    params: &crate::RecallTestParams,
    corpus_name: &str,
) -> Result<crate::RecallReport> {
    let report = typed_panel_kernel_recall_test(index, full, params, corpus_name)?;
    if report.ratio < params.min_recall_ratio {
        return Err(LodestarError::RecallBelowGate {
            ratio: report.ratio,
            min: params.min_recall_ratio,
        });
    }
    Ok(report)
}

/// Exact fused winners plus per-lane rank prefixes for recall refinement.
pub fn typed_panel_rank_stabilization_support_set(
    slots: &[SlotId],
    full: &BTreeMap<CxId, TypedPanelVectors>,
    params: &crate::RecallTestParams,
    lane_depth: usize,
) -> Result<crate::RecallSupportReport> {
    let prepared = prepare_typed_panel_corpus(full, slots)?;
    typed_panel_rank_stabilization_support_set_prepared(&prepared, params, lane_depth)
}

pub fn typed_panel_rank_stabilization_support_set_prepared(
    prepared: &PreparedTypedPanelCorpus<'_>,
    params: &crate::RecallTestParams,
    lane_depth: usize,
) -> Result<crate::RecallSupportReport> {
    let reference = prepare_typed_panel_recall_reference(prepared, params, lane_depth)?;
    typed_panel_rank_stabilization_support_from_reference(&reference)
}

/// Extract exact fused winners and lane prefixes from a previously ranked
/// full-corpus reference without rescoring the corpus.
pub fn typed_panel_rank_stabilization_support_from_reference(
    reference: &PreparedTypedPanelRecallReference<'_>,
) -> Result<crate::RecallSupportReport> {
    if reference.lane_depth == 0 {
        return invalid(
            "typed panel recall reference has no lane prefixes; prepare it with positive lane_depth",
        );
    }
    let mut members = BTreeSet::new();
    let mut candidate_hits = 0;
    for query in &reference.queries {
        let mut query_support = query.lane_prefix_ids.clone();
        query_support.extend(query.full_ids.iter().copied());
        candidate_hits += query_support.len();
        members.extend(query_support);
    }
    Ok(crate::RecallSupportReport {
        members: members.into_iter().collect(),
        n_queries_tested: reference.held_out.len(),
        held_out: reference.held_out.clone(),
        candidate_hits,
    })
}

fn validate_recall_reference(
    prepared: &PreparedTypedPanelCorpus<'_>,
    reference: &PreparedTypedPanelRecallReference<'_>,
) -> Result<()> {
    validate_recall_params(&reference.params, prepared.full)?;
    let expected_held_out = held_out(
        prepared.full,
        reference.params.held_out_fraction,
        reference.params.rng_seed,
    );
    if !std::ptr::eq(reference.full, prepared.full)
        || reference.corpus_len != prepared.full.len()
        || reference.slots != prepared.slots
        || reference.contracts != prepared.contracts
        || reference.held_out != expected_held_out
        || reference.queries.len() != reference.held_out.len()
        || reference
            .queries
            .iter()
            .zip(&reference.held_out)
            .any(|(query, id)| {
                prepared.rows.get(query.query_offset).map(|row| row.cx_id) != Some(*id)
                    || query.full_ids.is_empty()
                    || query.full_ids.len() != reference.params.top_k.min(reference.corpus_len)
            })
    {
        return invalid(
            "typed panel recall reference differs from the prepared corpus or recall contract",
        );
    }
    Ok(())
}

fn validate_typed_panel_refs(
    slots: &[SlotId],
    candidates: &BTreeMap<CxId, &TypedPanelVectors>,
) -> Result<BTreeMap<SlotId, TypedSlotContract>> {
    if slots.len() < 2 || candidates.is_empty() {
        return invalid(format!(
            "typed panel fusion needs at least two slots and one candidate; slots={} candidates={}",
            slots.len(),
            candidates.len()
        ));
    }
    if !slots.windows(2).all(|pair| pair[0] < pair[1]) {
        return invalid("typed panel slots must be strictly increasing and unique");
    }
    let first = *candidates.values().next().expect("nonempty candidates");
    let mut contracts = BTreeMap::new();
    for &slot in slots {
        let vector = first
            .get(&slot)
            .ok_or_else(|| invalid_error("first candidate missing slot"))?;
        vector.validate_schema()?;
        contracts.insert(
            slot,
            TypedSlotContract {
                shape: slot_similarity_shape(vector)?,
                ambient_dim: slot_ambient_dim(vector)?,
            },
        );
    }
    for (id, vectors) in candidates {
        validate_typed_panel_vector(&format!("candidate {id}"), vectors, slots, &contracts)?;
    }
    Ok(contracts)
}

fn validate_typed_panel_vector(
    label: &str,
    vectors: &TypedPanelVectors,
    slots: &[SlotId],
    contracts: &BTreeMap<SlotId, TypedSlotContract>,
) -> Result<()> {
    if vectors.len() != slots.len() || vectors.keys().copied().ne(slots.iter().copied()) {
        return invalid(format!(
            "{label} slots differ from the complete typed panel contract"
        ));
    }
    for &slot in slots {
        let vector = vectors.get(&slot).expect("validated slot key");
        vector.validate_schema()?;
        let observed = TypedSlotContract {
            shape: slot_similarity_shape(vector)?,
            ambient_dim: slot_ambient_dim(vector)?,
        };
        if observed != contracts[&slot] {
            return invalid(format!(
                "{label} slot {slot} contract {observed:?} differs from {:?}",
                contracts[&slot]
            ));
        }
    }
    Ok(())
}

fn validate_recall_params(
    params: &crate::RecallTestParams,
    full: &BTreeMap<CxId, TypedPanelVectors>,
) -> Result<()> {
    if full.is_empty() {
        return Err(LodestarError::RecallEmptyCorpus);
    }
    if !params.held_out_fraction.is_finite()
        || !(0.0..=1.0).contains(&params.held_out_fraction)
        || params.top_k == 0
        || !params.min_recall_ratio.is_finite()
        || !(0.0..=1.0).contains(&params.min_recall_ratio)
    {
        return Err(LodestarError::RecallInvalidParams {
            detail: "invalid typed panel recall parameters".to_string(),
        });
    }
    Ok(())
}

fn held_out(full: &BTreeMap<CxId, TypedPanelVectors>, fraction: f32, seed: u64) -> Vec<CxId> {
    let target = ((full.len() as f32) * fraction).ceil() as usize;
    let mut keyed = full
        .keys()
        .enumerate()
        .map(|(ordinal, id)| {
            let mut hasher = blake3::Hasher::new();
            hasher.update(&seed.to_be_bytes());
            hasher.update(&(ordinal as u64).to_be_bytes());
            hasher.update(id.as_bytes());
            (*hasher.finalize().as_bytes(), *id)
        })
        .collect::<Vec<_>>();
    keyed.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    keyed
        .into_iter()
        .take(target.min(full.len()))
        .map(|(_, id)| id)
        .collect()
}

fn invalid<T>(detail: impl Into<String>) -> Result<T> {
    Err(invalid_error(detail))
}

fn invalid_error(detail: impl Into<String>) -> LodestarError {
    LodestarError::KernelInvalidParams {
        detail: detail.into(),
    }
}

fn codec_error(error: serde_json::Error) -> LodestarError {
    LodestarError::KernelIndexCodec {
        detail: error.to_string(),
    }
}

#[cfg(test)]
mod issue1963_candidate_device_tests {
    use super::*;

    fn dense(data: Vec<f32>) -> SlotVector {
        SlotVector::Dense {
            dim: data.len() as u32,
            data,
        }
    }

    fn cx(byte: u8) -> CxId {
        CxId::from_bytes([byte; 16])
    }

    fn panel(slots: &[SlotId], data: &[f32]) -> TypedPanelVectors {
        slots.iter().map(|s| (*s, dense(data.to_vec()))).collect()
    }

    /// Core correctness contract behind GPU candidate generation (#1963): a
    /// candidate set that is a *superset* of the exact top-K (the extras being
    /// strictly worse neighbours the GPU may over-fetch) yields byte-identical
    /// top-K fusion hits — same cx_ids, same fused scores, same per-lane rank and
    /// the same frozen `SlotSimilarityEvidence`. So candidate GENERATION on GPU
    /// cannot perturb the persisted edge evidence, which is always the exact CPU
    /// rerank (`rank_typed_panel_candidate_refs`) over the union.
    #[test]
    fn superset_candidate_set_preserves_topk_edges_and_evidence() {
        let slots = [SlotId::new(3), SlotId::new(7)];
        let query = panel(&slots, &[1.0, 0.0]);
        // Cosine to the query strictly decreasing: near > mid > far > extra_far.
        let near = (cx(0x11), panel(&slots, &[1.0, 0.02]));
        let mid = (cx(0x22), panel(&slots, &[0.7, 0.7]));
        let far = (cx(0x33), panel(&slots, &[0.0, 1.0]));
        let extra_far = (cx(0x44), panel(&slots, &[-0.1, 1.0]));

        let exact: BTreeMap<CxId, &TypedPanelVectors> =
            [(near.0, &near.1), (mid.0, &mid.1)].into_iter().collect();
        let superset: BTreeMap<CxId, &TypedPanelVectors> = [
            (near.0, &near.1),
            (mid.0, &mid.1),
            (far.0, &far.1),
            (extra_far.0, &extra_far.1),
        ]
        .into_iter()
        .collect();

        let exact_hits = rank_typed_panel_candidate_refs(&query, &exact, &slots).unwrap();
        let superset_hits = rank_typed_panel_candidate_refs(&query, &superset, &slots).unwrap();

        assert_eq!(exact_hits.len(), 2, "exact set has exactly the top-2");
        assert!(superset_hits.len() >= 2);
        // The exact top-K is a byte-identical prefix of the superset result.
        for (exact_hit, superset_hit) in exact_hits.iter().zip(superset_hits.iter()) {
            assert_eq!(exact_hit.cx_id, superset_hit.cx_id);
            assert_eq!(
                exact_hit.score.to_bits(),
                superset_hit.score.to_bits(),
                "fused score must be bit-identical for {}",
                exact_hit.cx_id
            );
            assert_eq!(
                exact_hit.lanes, superset_hit.lanes,
                "per-lane rank + SlotSimilarityEvidence must be identical for {}",
                exact_hit.cx_id
            );
        }
        assert_eq!(exact_hits[0].cx_id, near.0, "nearest neighbour ranks first");
    }

    /// Ranking is deterministic run-to-run for a fixed candidate set: the
    /// total_cmp + CxId tie-break yields a stable order even when two candidates
    /// are exactly equidistant. Guarantees reproducible weave edges (#1963).
    #[test]
    fn equidistant_candidates_break_ties_deterministically() {
        let slots = [SlotId::new(1), SlotId::new(2)];
        let query = panel(&slots, &[1.0, 0.0]);
        let a = (cx(0xAA), panel(&slots, &[1.0, 0.0]));
        let b = (cx(0xBB), panel(&slots, &[1.0, 0.0]));
        let candidates: BTreeMap<CxId, &TypedPanelVectors> =
            [(a.0, &a.1), (b.0, &b.1)].into_iter().collect();

        let first = rank_typed_panel_candidate_refs(&query, &candidates, &slots).unwrap();
        let second = rank_typed_panel_candidate_refs(&query, &candidates, &slots).unwrap();
        assert_eq!(
            first, second,
            "identical inputs must produce identical order"
        );
        // Tie-break is by ascending CxId, so 0xAA precedes 0xBB.
        assert_eq!(first[0].cx_id, a.0);
        assert_eq!(first[1].cx_id, b.0);
    }
}
