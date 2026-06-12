//! Cross-lens agreement / disagreement search (PRD 10 §4).
//!
//! Anchored multi-view consensus: every candidate is compared to the anchor
//! constellation lens-by-lens with per-slot cosine similarity. `agree` ranks
//! by the *weakest* lens (min cosine — all lenses must concur for a high
//! score); `disagree` ranks by cross-lens spread (max − min cosine — one lens
//! says "same", another says "different": the multi-view class-anomaly).

use std::collections::BTreeMap;

use calyx_core::{CxId, Result, SlotId, SlotVector};
use serde::{Deserialize, Serialize};

use crate::error::{
    CALYX_SEXTANT_CONSENSUS_INSUFFICIENT_LENSES, CALYX_SEXTANT_CX_MISSING,
    CALYX_SEXTANT_DIM_MISMATCH, CALYX_SEXTANT_QUERY_SHAPE, CALYX_SEXTANT_VECTOR_SHAPE,
    sextant_error,
};
use crate::search::SearchEngine;

/// Minimum number of lenses a cross-lens consensus needs to be meaningful.
pub(crate) const MIN_CONSENSUS_LENSES: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsensusMode {
    Agree,
    Disagree,
}

/// One lens's similarity evidence for a candidate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotCosine {
    pub slot: SlotId,
    pub cosine: f32,
}

/// A candidate ranked by cross-lens consensus or anomaly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConsensusHit {
    pub cx_id: CxId,
    pub rank: usize,
    /// `Agree`: min per-slot cosine. `Disagree`: max − min per-slot cosine.
    pub score: f32,
    pub mean_cosine: f32,
    pub min_cosine: f32,
    pub max_cosine: f32,
    pub spread: f32,
    pub per_slot: Vec<SlotCosine>,
}

/// Full evidence for one agree/disagree call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConsensusReport {
    pub anchor: CxId,
    pub mode: ConsensusMode,
    /// Lenses the anchor exposed for comparison (dense, active).
    pub slots: Vec<SlotId>,
    pub hits: Vec<ConsensusHit>,
    /// Candidates that shared fewer than two lenses with the anchor and were
    /// therefore excluded from consensus ranking (reported, never silent).
    pub skipped_insufficient_overlap: Vec<CxId>,
}

/// Finds constellations the lenses *agree* are associated with the anchor.
pub fn agree(
    engine: &SearchEngine,
    anchor: CxId,
    k: usize,
    slot_filter: Option<&[SlotId]>,
) -> Result<ConsensusReport> {
    consensus(engine, anchor, k, slot_filter, ConsensusMode::Agree)
}

/// Finds constellations the lenses *disagree* about relative to the anchor.
pub fn disagree(
    engine: &SearchEngine,
    anchor: CxId,
    k: usize,
    slot_filter: Option<&[SlotId]>,
) -> Result<ConsensusReport> {
    consensus(engine, anchor, k, slot_filter, ConsensusMode::Disagree)
}

fn consensus(
    engine: &SearchEngine,
    anchor: CxId,
    k: usize,
    slot_filter: Option<&[SlotId]>,
    mode: ConsensusMode,
) -> Result<ConsensusReport> {
    if k == 0 {
        return Err(sextant_error(
            CALYX_SEXTANT_QUERY_SHAPE,
            "consensus k must be at least 1",
        ));
    }
    let slots = consensus_slots(engine, slot_filter)?;
    let anchor_vectors = dense_vectors(engine, &slots, anchor)?;
    if anchor_vectors.is_empty() {
        return Err(sextant_error(
            CALYX_SEXTANT_CX_MISSING,
            format!("anchor {anchor} has no dense vector in any participating slot"),
        ));
    }
    if anchor_vectors.len() < MIN_CONSENSUS_LENSES {
        return Err(sextant_error(
            CALYX_SEXTANT_CONSENSUS_INSUFFICIENT_LENSES,
            format!(
                "anchor {anchor} exposes {} dense lens(es); cross-lens consensus needs {}",
                anchor_vectors.len(),
                MIN_CONSENSUS_LENSES
            ),
        ));
    }
    let anchor_slots: Vec<SlotId> = anchor_vectors.keys().copied().collect();

    let mut hits = Vec::new();
    let mut skipped = Vec::new();
    for cx_id in engine.constellation_ids() {
        if cx_id == anchor {
            continue;
        }
        let mut per_slot = Vec::new();
        for (slot, anchor_data) in &anchor_vectors {
            if let Some(SlotVector::Dense { data, .. }) = engine.indexes.vector(*slot, cx_id)? {
                per_slot.push(SlotCosine {
                    slot: *slot,
                    cosine: dense_cosine(anchor_data, &data)?,
                });
            }
        }
        if per_slot.len() < MIN_CONSENSUS_LENSES {
            skipped.push(cx_id);
            continue;
        }
        let min = fold_cosine(&per_slot, f32::min);
        let max = fold_cosine(&per_slot, f32::max);
        let mean = per_slot.iter().map(|entry| entry.cosine).sum::<f32>() / per_slot.len() as f32;
        let spread = max - min;
        hits.push(ConsensusHit {
            cx_id,
            rank: 0,
            score: match mode {
                ConsensusMode::Agree => min,
                ConsensusMode::Disagree => spread,
            },
            mean_cosine: mean,
            min_cosine: min,
            max_cosine: max,
            spread,
            per_slot,
        });
    }
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.cx_id.cmp(&b.cx_id))
    });
    hits.truncate(k);
    for (idx, hit) in hits.iter_mut().enumerate() {
        hit.rank = idx + 1;
    }
    Ok(ConsensusReport {
        anchor,
        mode,
        slots: anchor_slots,
        hits,
        skipped_insufficient_overlap: skipped,
    })
}

fn consensus_slots(engine: &SearchEngine, slot_filter: Option<&[SlotId]>) -> Result<Vec<SlotId>> {
    let active = engine.indexes.slots();
    let Some(filter) = slot_filter else {
        return Ok(active);
    };
    for slot in filter {
        if !active.contains(slot) {
            return Err(crate::slot_index_map::SlotIndexMap::missing_slot_error(
                *slot,
            ));
        }
    }
    Ok(filter.to_vec())
}

/// Collects the dense vectors a constellation exposes on the given slots.
pub(crate) fn dense_vectors(
    engine: &SearchEngine,
    slots: &[SlotId],
    cx_id: CxId,
) -> Result<BTreeMap<SlotId, Vec<f32>>> {
    let mut vectors = BTreeMap::new();
    for slot in slots {
        if let Some(SlotVector::Dense { data, .. }) = engine.indexes.vector(*slot, cx_id)? {
            vectors.insert(*slot, data);
        }
    }
    Ok(vectors)
}

/// Fail-closed cosine between two dense vectors.
pub(crate) fn dense_cosine(a: &[f32], b: &[f32]) -> Result<f32> {
    if a.len() != b.len() {
        return Err(sextant_error(
            CALYX_SEXTANT_DIM_MISMATCH,
            format!("cosine dims differ: {} vs {}", a.len(), b.len()),
        ));
    }
    let mut dot = 0.0_f64;
    let mut norm_a = 0.0_f64;
    let mut norm_b = 0.0_f64;
    for (x, y) in a.iter().zip(b) {
        if !x.is_finite() || !y.is_finite() {
            return Err(sextant_error(
                CALYX_SEXTANT_VECTOR_SHAPE,
                "cosine requires finite vector components",
            ));
        }
        dot += f64::from(*x) * f64::from(*y);
        norm_a += f64::from(*x) * f64::from(*x);
        norm_b += f64::from(*y) * f64::from(*y);
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom <= f64::EPSILON {
        return Err(sextant_error(
            CALYX_SEXTANT_VECTOR_SHAPE,
            "cosine requires non-zero-norm vectors",
        ));
    }
    Ok((dot / denom) as f32)
}

fn fold_cosine(per_slot: &[SlotCosine], op: fn(f32, f32) -> f32) -> f32 {
    per_slot
        .iter()
        .map(|entry| entry.cosine)
        .reduce(op)
        .unwrap_or(0.0)
}
