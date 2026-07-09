use std::path::Path;

use calyx_core::{CalyxError, Result, SlotShape, SlotVector, SparseEntry};
use ort::value::ValueType;

use super::batch::TokenBatch;
use super::{config_invalid, validate_config};
use crate::frozen::NormPolicy;
use crate::runtime::common::normalize_unit;
use crate::runtime::onnx::PoolingPolicy;

#[derive(Clone, Copy, Debug)]
pub(super) enum CustomOutput {
    Dense {
        dim: u32,
        pooling: PoolingPolicy,
        norm_policy: NormPolicy,
    },
    Sparse {
        dim: u32,
    },
}

impl CustomOutput {
    pub(super) const fn dim(self) -> u32 {
        match self {
            Self::Dense { dim, .. } | Self::Sparse { dim } => dim,
        }
    }

    pub(super) const fn shape(self) -> SlotShape {
        match self {
            Self::Dense { dim, .. } => SlotShape::Dense(dim),
            Self::Sparse { dim } => SlotShape::Sparse(dim),
        }
    }
}

pub(super) fn output_from_session(
    session: &ort::session::Session,
    expected_shape: Option<SlotShape>,
    pooling: PoolingPolicy,
    norm_policy: NormPolicy,
) -> Result<CustomOutput> {
    let metadata = output_metadata(session)?;
    let output = if matches!(expected_shape, Some(SlotShape::Sparse(_))) {
        if metadata.rank != 2 {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "custom ONNX sparse output {} rank {} must be [batch, dim]",
                metadata.name, metadata.rank
            )));
        }
        CustomOutput::Sparse { dim: metadata.dim }
    } else {
        CustomOutput::Dense {
            dim: metadata.dim,
            pooling,
            norm_policy,
        }
    };
    let shape = output.shape();
    if let Some(expected) = expected_shape
        && expected != shape
    {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "custom ONNX output shape {shape:?} != declared {expected:?}"
        )));
    }
    Ok(output)
}

pub(super) fn vectors_from_output(
    outputs: &ort::session::SessionOutputs<'_>,
    batch: &TokenBatch,
    output: CustomOutput,
) -> Result<Vec<SlotVector>> {
    let tensor = output_tensor(outputs)?;
    let (shape, values) = tensor
        .try_extract_tensor::<f32>()
        .map_err(|err| config_invalid(format!("custom ONNX output is not f32 tensor: {err}")))?;
    match output {
        CustomOutput::Dense {
            dim,
            pooling,
            norm_policy,
        } => dense_output_batch(shape, values, batch, pooling, dim, norm_policy),
        CustomOutput::Sparse { dim } => sparse_output_batch(shape, values, batch, dim),
    }
}

pub(in crate::runtime::onnx) fn pooling_from_config(path: &Path) -> Result<PoolingPolicy> {
    let value = validate_config(path)?;
    let Some(raw) = value
        .get("pooling")
        .or_else(|| value.get("pooling_policy"))
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(PoolingPolicy::Mean);
    };
    match raw {
        "mean" => Ok(PoolingPolicy::Mean),
        "cls" => Ok(PoolingPolicy::Cls),
        "last_token" | "last-token" => Ok(PoolingPolicy::LastToken),
        other => Err(config_invalid(format!("unsupported ONNX pooling {other}"))),
    }
}

#[cfg(test)]
pub(in crate::runtime::onnx) fn pool_output(
    shape: &[i64],
    values: &[f32],
    mask: &[i64],
    policy: PoolingPolicy,
    dim: u32,
) -> Result<Vec<f32>> {
    let batch = TokenBatch {
        batch: 1,
        seq: mask.len(),
        ids: vec![0; mask.len()],
        mask: mask.to_vec(),
        indices: vec![0],
    };
    let mut rows = pool_output_batch(shape, values, &batch, policy, dim)?;
    rows.pop()
        .ok_or_else(|| CalyxError::lens_dim_mismatch("custom ONNX returned no pooled row"))
}

fn dense_output_batch(
    shape: &[i64],
    values: &[f32],
    batch: &TokenBatch,
    policy: PoolingPolicy,
    dim: u32,
    norm_policy: NormPolicy,
) -> Result<Vec<SlotVector>> {
    pool_output_batch(shape, values, batch, policy, dim)?
        .into_iter()
        .map(|mut data| {
            apply_norm(norm_policy, &mut data)?;
            Ok(SlotVector::Dense { dim, data })
        })
        .collect()
}

fn pool_output_batch(
    shape: &[i64],
    values: &[f32],
    batch: &TokenBatch,
    policy: PoolingPolicy,
    dim: u32,
) -> Result<Vec<Vec<f32>>> {
    let dim = dim as usize;
    match shape {
        [actual_batch, actual_dim]
            if positive_usize(*actual_batch) == Some(batch.batch)
                && positive_usize(*actual_dim) == Some(dim) =>
        {
            dense_rows(values, batch.batch, dim)
        }
        [actual_batch, seq, actual_dim]
            if positive_usize(*actual_batch) == Some(batch.batch)
                && positive_usize(*seq) == Some(batch.seq)
                && positive_usize(*actual_dim) == Some(dim) =>
        {
            token_rows(values, batch, dim, policy)
        }
        _ => Err(CalyxError::lens_dim_mismatch(format!(
            "custom ONNX output shape {shape:?} is incompatible with batch={} seq={} dim={dim}",
            batch.batch, batch.seq
        ))),
    }
}

fn sparse_output_batch(
    shape: &[i64],
    values: &[f32],
    batch: &TokenBatch,
    dim: u32,
) -> Result<Vec<SlotVector>> {
    let dim_usize = dim as usize;
    match shape {
        [actual_batch, actual_dim]
            if positive_usize(*actual_batch) == Some(batch.batch)
                && positive_usize(*actual_dim) == Some(dim_usize) => {}
        _ => {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "custom ONNX sparse output shape {shape:?} must be [batch={}, dim={dim_usize}]",
                batch.batch
            )));
        }
    }
    let expected = batch.batch * dim_usize;
    if values.len() != expected {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "custom ONNX sparse output has {} floats, expected {expected}",
            values.len()
        )));
    }
    values
        .chunks_exact(dim_usize)
        .map(|row| {
            Ok(SlotVector::Sparse {
                dim,
                entries: positive_sparse_entries(row)?,
            })
        })
        .collect()
}

fn positive_sparse_entries(values: &[f32]) -> Result<Vec<SparseEntry>> {
    let mut entries = Vec::new();
    for (idx, val) in values.iter().copied().enumerate() {
        if !val.is_finite() {
            return Err(CalyxError::lens_numerical_invariant(
                "custom ONNX sparse output emitted NaN or Inf",
            ));
        }
        if val > 0.0 {
            let idx = u32::try_from(idx)
                .map_err(|_| CalyxError::lens_dim_mismatch("sparse output index exceeds u32"))?;
            entries.push(SparseEntry { idx, val });
        }
    }
    Ok(entries)
}

struct OutputMetadata {
    name: String,
    rank: usize,
    dim: u32,
}

fn output_metadata(session: &ort::session::Session) -> Result<OutputMetadata> {
    let output = session
        .outputs()
        .iter()
        .find(|out| matches!(out.dtype(), ValueType::Tensor { .. }))
        .ok_or_else(|| config_invalid("custom ONNX model has no tensor outputs"))?;
    let ValueType::Tensor { shape, .. } = output.dtype() else {
        return Err(config_invalid("custom ONNX output is not a tensor"));
    };
    let Some(dim) = shape.last().copied().filter(|dim| *dim > 0) else {
        return Err(config_invalid(format!(
            "custom ONNX output {} has no static final dimension",
            output.name()
        )));
    };
    Ok(OutputMetadata {
        name: output.name().to_string(),
        rank: shape.len(),
        dim: u32::try_from(dim)
            .map_err(|_| CalyxError::lens_dim_mismatch("custom ONNX dim exceeds u32"))?,
    })
}

fn positive_usize(value: i64) -> Option<usize> {
    usize::try_from(value).ok().filter(|value| *value > 0)
}

fn output_tensor<'a, 'r>(
    outputs: &'a ort::session::SessionOutputs<'r>,
) -> Result<&'a ort::value::DynValue> {
    for name in [
        "splade_embedding",
        "sentence_embedding",
        "last_hidden_state",
        "pooler_output",
    ] {
        if let Some(output) = outputs.get(name) {
            return Ok(output);
        }
    }
    if outputs.len() == 0 {
        return Err(config_invalid("custom ONNX model returned no outputs"));
    }
    Ok(&outputs[0])
}

fn pool_tokens(
    values: &[f32],
    seq: usize,
    dim: usize,
    mask: &[i64],
    policy: PoolingPolicy,
) -> Result<Vec<f32>> {
    if values.len() != seq * dim {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "custom ONNX token output has {} floats, expected {}",
            values.len(),
            seq * dim
        )));
    }
    match policy {
        PoolingPolicy::Cls => Ok(values[..dim].to_vec()),
        PoolingPolicy::LastToken => {
            validate_mask_len(mask, seq)?;
            let index = mask
                .iter()
                .take(seq)
                .rposition(|value| *value > 0)
                .unwrap_or(seq.saturating_sub(1));
            Ok(values[index * dim..(index + 1) * dim].to_vec())
        }
        PoolingPolicy::Mean => {
            validate_mask_len(mask, seq)?;
            let mut out = vec![0.0; dim];
            let mut count = 0usize;
            for token in 0..seq {
                if mask.get(token).copied().unwrap_or(1) <= 0 {
                    continue;
                }
                count += 1;
                for axis in 0..dim {
                    out[axis] += values[token * dim + axis];
                }
            }
            if count == 0 {
                return Err(CalyxError::lens_numerical_invariant(
                    "custom ONNX mean pooling saw no unmasked tokens",
                ));
            }
            for value in &mut out {
                *value /= count as f32;
            }
            Ok(out)
        }
    }
}

fn validate_mask_len(mask: &[i64], seq: usize) -> Result<()> {
    if mask.len() < seq {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "custom ONNX attention mask has {} tokens, expected at least {seq}",
            mask.len()
        )));
    }
    Ok(())
}

fn dense_rows(values: &[f32], batch: usize, dim: usize) -> Result<Vec<Vec<f32>>> {
    let expected = batch * dim;
    if values.len() != expected {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "custom ONNX dense output has {} floats, expected {expected}",
            values.len()
        )));
    }
    Ok(values
        .chunks_exact(dim)
        .map(|row| row.to_vec())
        .collect::<Vec<_>>())
}

fn token_rows(
    values: &[f32],
    batch: &TokenBatch,
    dim: usize,
    policy: PoolingPolicy,
) -> Result<Vec<Vec<f32>>> {
    let expected = batch.batch * batch.seq * dim;
    if values.len() != expected {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "custom ONNX token output has {} floats, expected {expected}",
            values.len()
        )));
    }
    let mut rows = Vec::with_capacity(batch.batch);
    for row in 0..batch.batch {
        let token_start = row * batch.seq * dim;
        let token_end = token_start + batch.seq * dim;
        let mask_start = row * batch.seq;
        let mask_end = mask_start + batch.seq;
        rows.push(pool_tokens(
            &values[token_start..token_end],
            batch.seq,
            dim,
            &batch.mask[mask_start..mask_end],
            policy,
        )?);
    }
    Ok(rows)
}

fn apply_norm(policy: NormPolicy, data: &mut [f32]) -> Result<()> {
    match policy {
        NormPolicy::L2 { .. } | NormPolicy::Unit { .. } => normalize_unit(data),
        NormPolicy::None | NormPolicy::Finite | NormPolicy::DeclaredByModel { .. } => {
            if data.iter().all(|value| value.is_finite()) {
                Ok(())
            } else {
                Err(CalyxError::lens_numerical_invariant(
                    "custom ONNX emitted NaN or Inf",
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use calyx_core::SlotVector;

    use super::*;

    #[test]
    fn sparse_output_emits_only_positive_finite_entries() {
        let batch = TokenBatch {
            batch: 2,
            seq: 3,
            ids: vec![0; 6],
            mask: vec![1; 6],
            indices: vec![0, 1],
        };
        let vectors = sparse_output_batch(
            &[2, 4],
            &[0.0, 1.5, -0.2, 0.25, 2.0, 0.0, 0.0, 3.0],
            &batch,
            4,
        )
        .unwrap();

        let SlotVector::Sparse { dim, entries } = &vectors[0] else {
            panic!("expected sparse row");
        };
        assert_eq!(*dim, 4);
        assert_eq!(
            entries,
            &vec![
                SparseEntry { idx: 1, val: 1.5 },
                SparseEntry { idx: 3, val: 0.25 },
            ]
        );
        let SlotVector::Sparse { entries, .. } = &vectors[1] else {
            panic!("expected sparse row");
        };
        assert_eq!(
            entries,
            &vec![
                SparseEntry { idx: 0, val: 2.0 },
                SparseEntry { idx: 3, val: 3.0 },
            ]
        );
    }

    #[test]
    fn sparse_output_rejects_non_finite_values() {
        let batch = TokenBatch {
            batch: 1,
            seq: 1,
            ids: vec![0],
            mask: vec![1],
            indices: vec![0],
        };
        let error = sparse_output_batch(&[1, 2], &[0.0, f32::NAN], &batch, 2).unwrap_err();

        assert_eq!(error.code, "CALYX_LENS_NUMERICAL_INVARIANT");
    }
}
