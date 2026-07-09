use std::collections::HashMap;

use calyx_core::{CxId, Result};
use calyx_forge::{PreparedQuant, Quantizer, TurboQuantCodec, new_seed};

use super::HnswIndex;
use crate::error::{CALYX_SEXTANT_VECTOR_SHAPE, sextant_error};
use crate::index::{QuantConfig, QuantKind};
use crate::util::{cosine, top_k};

impl HnswIndex {
    pub(super) fn turbo_codec_for(&self, quant: &QuantConfig) -> Option<TurboQuantCodec> {
        let QuantKind::TurboQuant { level } = quant.kind else {
            return None;
        };
        if self.dim == 0 {
            return None;
        }
        TurboQuantCodec::new(
            new_seed(self.dim as usize, &turbo_seed_entropy(self)),
            level,
        )
        .ok()
    }

    pub(super) fn prepare_turbo(&self, values: &[f32]) -> Result<Option<PreparedQuant>> {
        let Some(codec) = &self.turbo_codec else {
            return Ok(None);
        };
        let normalized = normalize_for_cosine(values)?;
        let encoded = codec.encode(&normalized).map_err(quant_error)?;
        codec.prepare(&encoded).map(Some).map_err(quant_error)
    }

    pub(super) fn score_row(
        &self,
        query: &[f32],
        query_prepared: Option<&PreparedQuant>,
        idx: usize,
    ) -> f32 {
        if let (Some(codec), Some(query), Some(candidate)) = (
            self.turbo_codec.as_ref(),
            query_prepared,
            self.rows[idx].prepared.as_ref(),
        ) {
            return codec.dot_prepared(query, candidate);
        }
        cosine(query, &self.rows[idx].vector)
    }

    pub(super) fn exact_rerank(
        &self,
        query: &[f32],
        candidates: HashMap<CxId, f32>,
        k: usize,
    ) -> Vec<(CxId, f32)> {
        top_k(
            candidates
                .keys()
                .filter_map(|cx_id| {
                    self.positions.get(cx_id).and_then(|idx| {
                        let row = &self.rows[*idx];
                        (!row.deleted).then(|| (*cx_id, cosine(query, &row.vector)))
                    })
                })
                .collect(),
            k,
        )
    }
}

fn turbo_seed_entropy(index: &HnswIndex) -> Vec<u8> {
    let mut entropy = Vec::with_capacity(34);
    entropy.extend_from_slice(b"calyx-sextant-hnsw-tq-v1");
    entropy.extend_from_slice(&index.slot.get().to_le_bytes());
    entropy.extend_from_slice(&index.dim.to_le_bytes());
    entropy.extend_from_slice(&index.seed.to_le_bytes());
    entropy
}

fn normalize_for_cosine(values: &[f32]) -> Result<Vec<f32>> {
    if let Some(idx) = values.iter().position(|value| !value.is_finite()) {
        return Err(sextant_error(
            CALYX_SEXTANT_VECTOR_SHAPE,
            format!("TurboQuant HNSW vector contains non-finite coefficient at index {idx}"),
        ));
    }
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm == 0.0 {
        return Ok(vec![0.0; values.len()]);
    }
    Ok(values.iter().map(|value| *value / norm).collect())
}

fn quant_error(error: calyx_forge::ForgeError) -> calyx_core::CalyxError {
    sextant_error(
        CALYX_SEXTANT_VECTOR_SHAPE,
        format!("TurboQuant HNSW scoring failed closed: {error}"),
    )
}
