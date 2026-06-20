use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use calyx_core::{CalyxError, Input, Lens, LensId, Result, SlotShape, SlotVector, SparseEntry};
use fastembed::SparseEmbedding;

use super::super::{OnnxModelFiles, OnnxProviderPolicy};
use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::runtime::common::{fastembed_cache_root, hash_files, normalize_unit, text_from_input};
use crate::spec::LensSpec;

pub(super) fn special_files(
    cache_dir: &Path,
    model_code: &str,
    model_file: &str,
    additional_files: &[String],
) -> Result<OnnxModelFiles> {
    let effective_cache = fastembed_cache_root(cache_dir);
    super::super::fastembed_runtime::resolve_files(
        &effective_cache,
        model_code,
        model_file,
        additional_files,
    )
}

pub(super) fn contract(
    name: String,
    files: &OnnxModelFiles,
    shape: SlotShape,
    norm: NormPolicy,
    corpus_parts: &[&[u8]],
) -> Result<FrozenLensContract> {
    Ok(FrozenLensContract::new(
        name,
        hash_files(&files.artifact_paths())?,
        sha256_digest(corpus_parts),
        shape,
        calyx_core::Modality::Text,
        LensDType::F32,
        norm,
    ))
}

pub(super) fn ensure_spec_match(
    shape: SlotShape,
    weights: [u8; 32],
    spec: &LensSpec,
) -> Result<()> {
    if shape != spec.output {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "fastembed special output shape {shape:?} != declared {:?}",
            spec.output
        )));
    }
    if weights != spec.weights_sha256 {
        return Err(CalyxError::lens_frozen_violation(
            "fastembed special artifact hash does not match LensSpec",
        ));
    }
    Ok(())
}

pub(super) fn input_texts(lens: &dyn Lens, inputs: &[Input]) -> Result<Vec<String>> {
    inputs
        .iter()
        .map(|input| text_from_input(lens, input).map(str::to_string))
        .collect()
}

pub(super) fn single_vector(lens_id: LensId, mut batch: Vec<SlotVector>) -> Result<SlotVector> {
    batch
        .pop()
        .ok_or_else(|| CalyxError::lens_dim_mismatch(format!("lens {lens_id} returned no vector")))
}

pub(super) fn dense_batch(
    rows: Vec<Vec<f32>>,
    dim: u32,
    expected: usize,
) -> Result<Vec<SlotVector>> {
    ensure_count(rows.len(), expected, "dense")?;
    rows.into_iter()
        .map(|mut data| {
            if data.len() != dim as usize {
                return Err(CalyxError::lens_dim_mismatch(format!(
                    "dense dim {} != expected {dim}",
                    data.len()
                )));
            }
            normalize_unit(&mut data)?;
            Ok(SlotVector::Dense { dim, data })
        })
        .collect()
}

pub(super) fn sparse_batch(
    rows: Vec<SparseEmbedding>,
    dim: u32,
    expected: usize,
) -> Result<Vec<SlotVector>> {
    ensure_count(rows.len(), expected, "sparse")?;
    rows.into_iter()
        .map(|row| {
            Ok(SlotVector::Sparse {
                dim,
                entries: sparse_entries(row, dim)?,
            })
        })
        .collect()
}

pub(super) fn multi_batch(
    rows: Vec<Vec<Vec<f32>>>,
    token_dim: u32,
    expected: usize,
) -> Result<Vec<SlotVector>> {
    ensure_count(rows.len(), expected, "ColBERT")?;
    rows.into_iter()
        .map(|tokens| {
            if tokens.is_empty() {
                return Err(CalyxError::lens_dim_mismatch(
                    "ColBERT returned no token vectors",
                ));
            }
            for token in &tokens {
                if token.len() != token_dim as usize {
                    return Err(CalyxError::lens_dim_mismatch(format!(
                        "ColBERT token dim {} != expected {token_dim}",
                        token.len()
                    )));
                }
                ensure_finite("ColBERT token", token)?;
            }
            Ok(SlotVector::Multi { token_dim, tokens })
        })
        .collect()
}

fn sparse_entries(row: SparseEmbedding, dim: u32) -> Result<Vec<SparseEntry>> {
    if row.indices.len() != row.values.len() {
        return Err(CalyxError::lens_dim_mismatch(
            "sparse index/value count mismatch",
        ));
    }
    row.indices
        .into_iter()
        .zip(row.values)
        .map(|(idx, val)| {
            let idx = u32::try_from(idx)
                .map_err(|_| CalyxError::lens_dim_mismatch("sparse index exceeds u32"))?;
            if idx >= dim {
                return Err(CalyxError::lens_dim_mismatch(format!(
                    "sparse index {idx} outside dim {dim}"
                )));
            }
            ensure_finite("sparse value", &[val])?;
            Ok(SparseEntry { idx, val })
        })
        .collect()
}

fn ensure_count(actual: usize, expected: usize, label: &str) -> Result<()> {
    if actual == expected {
        return Ok(());
    }
    Err(CalyxError::lens_dim_mismatch(format!(
        "{label} returned {actual} vectors for {expected} inputs"
    )))
}

pub(super) fn ensure_finite(label: &str, values: &[f32]) -> Result<()> {
    if values.iter().all(|value| value.is_finite()) {
        return Ok(());
    }
    Err(CalyxError::lens_numerical_invariant(format!(
        "{label} is NaN or Inf"
    )))
}

pub(super) fn sparse_shape_dim(shape: SlotShape) -> u32 {
    match shape {
        SlotShape::Sparse(dim) => dim,
        _ => unreachable!("FastembedSparseLens shape is sparse"),
    }
}

pub(super) fn rerank_pair(text: &str) -> (&str, &str) {
    text.split_once("\n---\n")
        .or_else(|| text.split_once('\n'))
        .unwrap_or((text, text))
}

pub(super) fn lock_model<'a, T>(
    model: &'a Option<Mutex<T>>,
    label: &str,
) -> Result<MutexGuard<'a, T>> {
    model
        .as_ref()
        .expect("fastembed model is present until drop")
        .lock()
        .map_err(|_| CalyxError::lens_unreachable(format!("{label} model mutex was poisoned")))
}

pub(super) fn leak_cuda_model<T>(
    model: &mut Option<Mutex<T>>,
    provider_policy: OnnxProviderPolicy,
) {
    if provider_policy == OnnxProviderPolicy::CudaFailLoud
        && let Some(model) = model.take()
    {
        std::mem::forget(model);
    }
}
