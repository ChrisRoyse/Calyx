use calyx_core::{CalyxError, Result};

use super::CandlePoolingPolicy;
use crate::frozen::NormPolicy;
use crate::runtime::common::normalize_unit;

pub(super) fn mean_pool(tokens: &[Vec<f32>], mask: &[u32], dim: usize) -> Result<Vec<f32>> {
    let mut out = vec![0.0_f32; dim];
    let mut count = 0_u32;
    for (row, keep) in tokens.iter().zip(mask) {
        if *keep == 0 {
            continue;
        }
        if row.len() != dim {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "candle token dim {} != expected {dim}",
                row.len()
            )));
        }
        for (dst, value) in out.iter_mut().zip(row) {
            *dst += *value;
        }
        count += 1;
    }
    if count == 0 {
        return Err(CalyxError::lens_dim_mismatch(
            "candle attention mask selected no tokens",
        ));
    }
    let inv = 1.0 / count as f32;
    for value in &mut out {
        *value *= inv;
    }
    Ok(out)
}

pub(super) fn pool_tokens(
    tokens: &[Vec<f32>],
    mask: &[u32],
    dim: usize,
    policy: CandlePoolingPolicy,
) -> Result<Vec<f32>> {
    match policy {
        CandlePoolingPolicy::Mean => mean_pool(tokens, mask, dim),
        CandlePoolingPolicy::Cls => {
            let row = tokens
                .iter()
                .zip(mask)
                .find_map(|(row, keep)| (*keep != 0).then_some(row))
                .ok_or_else(|| {
                    CalyxError::lens_dim_mismatch("candle attention mask selected no tokens")
                })?;
            if row.len() != dim {
                return Err(CalyxError::lens_dim_mismatch(format!(
                    "candle CLS token dim {} != expected {dim}",
                    row.len()
                )));
            }
            Ok(row.clone())
        }
    }
}

pub(super) fn apply_norm(policy: NormPolicy, data: &mut [f32]) -> Result<()> {
    if data.iter().any(|value| !value.is_finite()) {
        return Err(CalyxError::lens_numerical_invariant(
            "candle runtime emitted NaN or Inf",
        ));
    }
    match policy {
        NormPolicy::None | NormPolicy::Finite => Ok(()),
        NormPolicy::L2 { .. } | NormPolicy::Unit { .. } => normalize_unit(data),
        NormPolicy::DeclaredByModel { .. } => Ok(()),
    }
}
