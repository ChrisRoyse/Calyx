//! PH55 ASK execution: retrieval grounding plus PH33/PH49-compatible stubs.

use std::collections::{BTreeMap, BTreeSet};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::layers::RecordKey;
use calyx_aster::vault::AsterVault;
use calyx_core::{AbsentReason, Clock, CxId, Result, Seq, SlotId, SlotVector, VaultStore};
use serde::{Deserialize, Serialize};

use crate::error::{
    CALYX_ANSWER_UNGROUNDED, CALYX_INVALID_ARGUMENT, CALYX_LENS_NOT_FOUND, sextant_error,
};
use crate::fusion::rrf::rrf_fuse_restricted;
use crate::fusion::{FusionContext, FusionStrategy};
use crate::index::IndexSearchHit;

use super::{AskSpec, DEFAULT_ASK_TOP_K, ProvenancedRow};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AskResult {
    pub answer: String,
    pub grounding: Vec<ProvenancedRow>,
    pub gaps: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oracle_conf: Option<f32>,
}

pub fn ask<C>(vault: &AsterVault<C>, spec: &AskSpec, snapshot_seq: Seq) -> Result<AskResult>
where
    C: Clock,
{
    if spec.question.trim().is_empty() {
        return Err(sextant_error(
            CALYX_INVALID_ARGUMENT,
            "ASK question must not be empty",
        ));
    }

    let top_k = effective_top_k(spec.top_k);
    let candidates = candidate_set(vault, spec, snapshot_seq)?;
    if candidates.is_empty() {
        return Err(sextant_error(
            CALYX_ANSWER_UNGROUNDED,
            "ASK produced no visible grounding candidates",
        ));
    }

    let top = retrieve(vault, &spec.question, snapshot_seq, top_k, &candidates)?;
    if top.is_empty() {
        return Err(sextant_error(
            CALYX_ANSWER_UNGROUNDED,
            "ASK retrieval returned no grounded candidates",
        ));
    }

    let kernel = kernel_answer_stub(&top);
    if kernel.grounding_cx_ids.is_empty() {
        return Err(sextant_error(
            CALYX_ANSWER_UNGROUNDED,
            "kernel answer returned no grounding CxIds",
        ));
    }

    let scores = top
        .into_iter()
        .map(|hit| (hit.cx_id, hit.score))
        .collect::<BTreeMap<_, _>>();
    let grounding = kernel
        .grounding_cx_ids
        .into_iter()
        .map(|cx_id| grounded_row(vault, snapshot_seq, cx_id, scores.get(&cx_id).copied()))
        .collect::<Result<Vec<_>>>()?;

    Ok(AskResult {
        answer: kernel.text,
        grounding,
        gaps: kernel.gaps,
        oracle_conf: oracle_stub_conf(spec.oracle),
    })
}

fn effective_top_k(top_k: usize) -> usize {
    if top_k == 0 { DEFAULT_ASK_TOP_K } else { top_k }
}

fn candidate_set<C>(
    vault: &AsterVault<C>,
    spec: &AskSpec,
    snapshot_seq: Seq,
) -> Result<BTreeSet<CxId>>
where
    C: Clock,
{
    if !spec.context_cx_ids.is_empty() {
        return Ok(spec
            .context_cx_ids
            .iter()
            .copied()
            .filter(|cx_id| vault.get(*cx_id, snapshot_seq).is_ok())
            .collect());
    }
    Ok(vault
        .scan_cf_at(snapshot_seq, ColumnFamily::Base)?
        .into_iter()
        .filter_map(|(key, _)| cx_id_from_base_key(&key))
        .collect::<BTreeSet<_>>())
}

fn retrieve<C>(
    vault: &AsterVault<C>,
    question: &str,
    snapshot_seq: Seq,
    top_k: usize,
    candidates: &BTreeSet<CxId>,
) -> Result<Vec<crate::hit::Hit>>
where
    C: Clock,
{
    let mut per_slot = BTreeMap::<SlotId, Vec<ScoredCandidate>>::new();
    for cx_id in candidates {
        let cx = vault.get(*cx_id, snapshot_seq)?;
        for (slot, vector) in &cx.slots {
            if let Some(score) = score_slot(question, *cx_id, *slot, vector) {
                per_slot.entry(*slot).or_default().push(ScoredCandidate {
                    cx_id: *cx_id,
                    score,
                });
            }
        }
    }
    if per_slot.is_empty() {
        return Err(sextant_error(
            CALYX_LENS_NOT_FOUND,
            "ASK retrieval found no available lens slots for visible candidates",
        ));
    }

    let ranked = per_slot
        .into_iter()
        .map(|(slot, mut scored)| {
            scored.sort_by(|left, right| {
                right
                    .score
                    .total_cmp(&left.score)
                    .then_with(|| left.cx_id.to_string().cmp(&right.cx_id.to_string()))
            });
            let hits = scored
                .into_iter()
                .enumerate()
                .map(|(idx, item)| IndexSearchHit {
                    cx_id: item.cx_id,
                    score: item.score,
                    rank: idx + 1,
                })
                .collect::<Vec<_>>();
            (slot, hits)
        })
        .collect::<BTreeMap<_, _>>();

    Ok(rrf_fuse_restricted(
        &ranked,
        &FusionContext {
            k: top_k,
            explain: false,
            strategy: FusionStrategy::Rrf,
            weights: BTreeMap::new(),
            stage1_slots: Vec::new(),
        },
        candidates,
    ))
}

fn score_slot(question: &str, cx_id: CxId, slot: SlotId, vector: &SlotVector) -> Option<f32> {
    match vector {
        SlotVector::Dense { dim, data } if *dim as usize == data.len() && !data.is_empty() => {
            Some(score_dense(question, cx_id, slot, data))
        }
        SlotVector::Sparse { entries, .. } if !entries.is_empty() => {
            let sum = entries
                .iter()
                .filter(|entry| entry.val.is_finite())
                .map(|entry| entry.val.abs())
                .sum::<f32>();
            finite_positive(sum).map(|score| score + tie_break(question, cx_id, slot))
        }
        SlotVector::Multi { token_dim, tokens } if *token_dim > 0 && !tokens.is_empty() => {
            let sum = tokens
                .iter()
                .flat_map(|token| token.iter())
                .filter(|value| value.is_finite())
                .map(|value| value.abs())
                .sum::<f32>();
            finite_positive(sum).map(|score| score + tie_break(question, cx_id, slot))
        }
        SlotVector::Absent {
            reason: AbsentReason::LensUnavailable | AbsentReason::LensInactive,
        } => None,
        SlotVector::Absent { .. } => None,
        _ => None,
    }
}

fn score_dense(question: &str, cx_id: CxId, slot: SlotId, data: &[f32]) -> f32 {
    let query = query_features(question, data.len());
    let dot = data
        .iter()
        .zip(query.iter())
        .filter(|(left, right)| left.is_finite() && right.is_finite())
        .map(|(left, right)| left * right)
        .sum::<f32>();
    finite_positive(dot.abs()).unwrap_or(0.0) + tie_break(question, cx_id, slot)
}

fn query_features(question: &str, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|idx| {
            let mut hasher = blake3::Hasher::new();
            hasher.update(b"calyx-ph55-ask-query-feature-v1");
            hasher.update(question.as_bytes());
            hasher.update(&(idx as u32).to_be_bytes());
            let hash = hasher.finalize();
            let raw = u32::from_be_bytes(hash.as_bytes()[0..4].try_into().unwrap());
            (raw as f32 / u32::MAX as f32).max(f32::EPSILON)
        })
        .collect()
}

fn tie_break(question: &str, cx_id: CxId, slot: SlotId) -> f32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"calyx-ph55-ask-rank-tiebreak-v1");
    hasher.update(question.as_bytes());
    hasher.update(cx_id.as_bytes());
    hasher.update(&slot.get().to_be_bytes());
    let hash = hasher.finalize();
    let raw = u16::from_be_bytes(hash.as_bytes()[0..2].try_into().unwrap());
    f32::from(raw) / f32::from(u16::MAX) * 1.0e-6
}

fn finite_positive(value: f32) -> Option<f32> {
    (value.is_finite() && value > 0.0).then_some(value)
}

fn grounded_row<C>(
    vault: &AsterVault<C>,
    snapshot_seq: Seq,
    cx_id: CxId,
    score: Option<f32>,
) -> Result<ProvenancedRow>
where
    C: Clock,
{
    let ledger_ref = vault.get(cx_id, snapshot_seq).ok().map(|cx| cx.provenance);
    Ok(ProvenancedRow {
        key: RecordKey::from_bytes(cx_id.as_bytes().to_vec())?,
        value: None,
        score,
        ledger_ref,
    })
}

fn cx_id_from_base_key(key: &[u8]) -> Option<CxId> {
    let bytes: [u8; 16] = key.try_into().ok()?;
    Some(CxId::from_bytes(bytes))
}

fn oracle_stub_conf(_enabled: bool) -> Option<f32> {
    None
}

fn kernel_answer_stub(top: &[crate::hit::Hit]) -> KernelAnswerStub {
    KernelAnswerStub {
        text: "[kernel stub]".to_string(),
        grounding_cx_ids: top.iter().map(|hit| hit.cx_id).collect(),
        gaps: Vec::new(),
    }
}

struct ScoredCandidate {
    cx_id: CxId,
    score: f32,
}

struct KernelAnswerStub {
    text: String,
    grounding_cx_ids: Vec<CxId>,
    gaps: Vec<String>,
}

#[cfg(test)]
mod fsv_tests;
#[cfg(test)]
mod tests;
