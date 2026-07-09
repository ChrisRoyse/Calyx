use std::cmp::Ordering;
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::encode::decode_constellation_base;
use calyx_core::{CalyxError, CxId, LedgerRef, SlotVector, VaultStore};
use serde::{Deserialize, Serialize};

use super::{CALYX_INVALID_ASK_QUERY, cosine, error, open_aster, validate_query};
use crate::leapable::VaultMode;
use crate::migrate::adapter::{BASE_SLOT, METADATA_GTE_LENS_ID};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct LensProvenance {
    pub(crate) slot_id: u16,
    pub(crate) lens_id: String,
    pub(crate) ledger_ref: LedgerRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Hit {
    pub(crate) rank: usize,
    pub(crate) chunk_id: String,
    pub(crate) database_name: String,
    pub(crate) cx_id: String,
    pub(crate) score: f64,
    pub(crate) ledger_ref: LedgerRef,
    pub(crate) per_lens_provenance: Vec<LensProvenance>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct AskResult {
    pub(crate) mode: VaultMode,
    pub(crate) top_k: usize,
    pub(crate) hits: Vec<Hit>,
}

#[derive(Clone, Debug)]
struct HitCandidate {
    chunk_id: String,
    database_name: String,
    cx_id: CxId,
    score: f64,
    ledger_ref: LedgerRef,
    lens_id: String,
}

pub(crate) fn ask_calyx(
    calyx_dir: &Path,
    mode: VaultMode,
    query_vec: &[f32],
    top_k: usize,
) -> Result<AskResult, CalyxError> {
    validate_query(query_vec, top_k)?;
    let (aster, _manifest) = open_aster(calyx_dir)?;
    let snapshot = aster.snapshot();
    let mut candidates = Vec::with_capacity(top_k);
    for (_key, bytes) in aster.scan_cf_at(snapshot, ColumnFamily::Base)? {
        let cx = decode_constellation_base(&bytes)?;
        let Some(SlotVector::Dense { dim, data }) =
            aster.read_slot_vector_at(snapshot, cx.cx_id, BASE_SLOT)?
        else {
            continue;
        };
        if dim as usize != query_vec.len() || data.len() != query_vec.len() {
            return Err(error(
                CALYX_INVALID_ASK_QUERY,
                format!(
                    "query dim {} does not match base slot dim {dim}",
                    query_vec.len()
                ),
                "ask with a finite query vector matching the vault base slot dimension",
            ));
        }
        let score = cosine(query_vec, &data);
        let chunk_id = cx.chunk_id().unwrap_or("");
        if !candidate_would_rank(score, chunk_id, &candidates, top_k) {
            continue;
        }
        let candidate = HitCandidate {
            chunk_id: chunk_id.to_string(),
            database_name: cx.database_name().unwrap_or("").to_string(),
            cx_id: cx.cx_id,
            score,
            ledger_ref: cx.provenance,
            lens_id: cx
                .metadata
                .get(METADATA_GTE_LENS_ID)
                .cloned()
                .unwrap_or_else(|| "unknown".to_string()),
        };
        insert_candidate(&mut candidates, candidate, top_k);
    }
    candidates.sort_by(compare_candidates);
    let hits = candidates
        .into_iter()
        .enumerate()
        .map(|(idx, candidate)| candidate.into_hit(idx + 1))
        .collect();
    Ok(AskResult { mode, top_k, hits })
}

fn candidate_would_rank(
    score: f64,
    chunk_id: &str,
    candidates: &[HitCandidate],
    top_k: usize,
) -> bool {
    candidates.len() < top_k
        || worst_candidate(candidates).is_some_and(|worst| {
            compare_score(score, chunk_id, worst.score, &worst.chunk_id).is_lt()
        })
}

fn insert_candidate(candidates: &mut Vec<HitCandidate>, candidate: HitCandidate, top_k: usize) {
    if candidates.len() < top_k {
        candidates.push(candidate);
        return;
    }
    if let Some(worst) = worst_candidate_index(candidates)
        && compare_candidates(&candidate, &candidates[worst]).is_lt()
    {
        candidates[worst] = candidate;
    }
}

fn worst_candidate(candidates: &[HitCandidate]) -> Option<&HitCandidate> {
    candidates
        .iter()
        .max_by(|left, right| compare_candidates(left, right))
}

fn worst_candidate_index(candidates: &[HitCandidate]) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| compare_candidates(left, right))
        .map(|(idx, _)| idx)
}

fn compare_candidates(left: &HitCandidate, right: &HitCandidate) -> Ordering {
    compare_score(left.score, &left.chunk_id, right.score, &right.chunk_id)
}

fn compare_score(
    left_score: f64,
    left_chunk: &str,
    right_score: f64,
    right_chunk: &str,
) -> Ordering {
    right_score
        .partial_cmp(&left_score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| left_chunk.cmp(right_chunk))
}

impl HitCandidate {
    fn into_hit(self, rank: usize) -> Hit {
        Hit {
            rank,
            chunk_id: self.chunk_id,
            database_name: self.database_name,
            cx_id: self.cx_id.to_string(),
            score: self.score,
            ledger_ref: self.ledger_ref.clone(),
            per_lens_provenance: vec![LensProvenance {
                slot_id: BASE_SLOT.get(),
                lens_id: self.lens_id,
                ledger_ref: self.ledger_ref,
            }],
        }
    }
}
