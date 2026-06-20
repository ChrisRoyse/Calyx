use calyx_core::{CalyxError, Result};

use super::custom::batch::TokenBatch;

pub(in crate::runtime::onnx) fn multi_rows(
    shape: &[i64],
    values: &[f32],
    batch: &TokenBatch,
    token_dim: usize,
) -> Result<Vec<Vec<Vec<f32>>>> {
    match shape {
        [actual_batch, seq, actual_dim]
            if positive_usize(*actual_batch) == Some(batch.batch)
                && positive_usize(*seq) == Some(batch.seq)
                && positive_usize(*actual_dim) == Some(token_dim) =>
        {
            token_rows(values, batch, token_dim)
        }
        _ => Err(CalyxError::lens_dim_mismatch(format!(
            "ONNX ColBERT output shape {shape:?} is incompatible with batch={} seq={} token_dim={token_dim}",
            batch.batch, batch.seq
        ))),
    }
}

fn token_rows(values: &[f32], batch: &TokenBatch, token_dim: usize) -> Result<Vec<Vec<Vec<f32>>>> {
    let expected = batch.batch * batch.seq * token_dim;
    if values.len() != expected {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "ONNX ColBERT token output has {} floats, expected {expected}",
            values.len()
        )));
    }
    let mut rows = Vec::with_capacity(batch.batch);
    for row in 0..batch.batch {
        let token_start = row * batch.seq * token_dim;
        let mask_start = row * batch.seq;
        let mut tokens = Vec::new();
        for token in 0..batch.seq {
            if batch.mask[mask_start + token] <= 0 {
                continue;
            }
            let start = token_start + token * token_dim;
            let end = start + token_dim;
            let data = values[start..end].to_vec();
            ensure_finite(&data)?;
            tokens.push(data);
        }
        if tokens.is_empty() {
            return Err(CalyxError::lens_dim_mismatch(
                "ONNX ColBERT returned no unmasked token vectors",
            ));
        }
        rows.push(tokens);
    }
    Ok(rows)
}

fn ensure_finite(values: &[f32]) -> Result<()> {
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(CalyxError::lens_numerical_invariant(
            "ONNX ColBERT token is NaN or Inf",
        ))
    }
}

fn positive_usize(value: i64) -> Option<usize> {
    usize::try_from(value).ok().filter(|value| *value > 0)
}
