use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Input, Lens, Result, SlotShape, SlotVector};
use ort::session::Session;
use ort::value::ValueType;
use serde_json::Value;
use tokenizers::Tokenizer;

pub(in crate::runtime::onnx) mod batch;

use super::cuda_guard::CudaDropGuard;
use super::fastembed_runtime::execution_providers;
use super::{OnnxFileSpec, OnnxLens, OnnxModelFiles, PoolingPolicy, config_invalid};
use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::runtime::common::{hash_files, normalize_unit};
use batch::{TokenBatch, session_inputs, token_batches};

pub struct CustomOnnxRuntime {
    session: Session,
    tokenizer: Tokenizer,
    pooling: PoolingPolicy,
    norm_policy: NormPolicy,
    dim: u32,
    max_tokens: usize,
}

impl CustomOnnxRuntime {
    pub const fn dim(&self) -> u32 {
        self.dim
    }

    pub fn measure_batch(&mut self, lens: &dyn Lens, inputs: &[Input]) -> Result<Vec<SlotVector>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let batches = token_batches(&self.tokenizer, lens, inputs, self.max_tokens)?;
        let mut rows = vec![None; inputs.len()];
        for batch in &batches {
            let pooled = self.run_token_batch(batch)?;
            if pooled.len() != batch.indices.len() {
                return Err(CalyxError::lens_dim_mismatch(format!(
                    "custom ONNX returned {} rows for {} bucketed inputs",
                    pooled.len(),
                    batch.indices.len()
                )));
            }
            for (index, data) in batch.indices.iter().copied().zip(pooled) {
                rows[index] = Some(data);
            }
        }
        rows.into_iter()
            .map(|data| {
                let mut data = data.ok_or_else(|| {
                    CalyxError::lens_dim_mismatch("custom ONNX omitted a bucketed row")
                })?;
                apply_norm(self.norm_policy, &mut data)?;
                Ok(SlotVector::Dense {
                    dim: self.dim,
                    data,
                })
            })
            .collect()
    }
}

pub fn from_files(spec: OnnxFileSpec) -> Result<OnnxLens> {
    let _ort_dylib = super::dynamic_ort::ensure_dynamic_ort()?;
    ensure_file("model", &spec.model_file)?;
    ensure_file("tokenizer", &spec.tokenizer)?;
    ensure_file("config", &spec.config)?;
    let config = validate_config(&spec.config)?;
    let max_tokens = batch::max_tokens_from_config(&config)?;
    let files = model_files(&spec);
    let weights_sha256 = hash_files(&files.artifact_paths())
        .map_err(|err| config_invalid(format!("read custom ONNX artifacts failed: {err}")))?;
    if let Some(expected) = spec.expected_weights_sha256
        && expected != weights_sha256
    {
        return Err(CalyxError::lens_frozen_violation(format!(
            "custom ONNX weights hash drift for {}",
            spec.model_id
        )));
    }
    let session = Session::builder()
        .map_err(|err| config_invalid(format!("ONNX session builder failed: {err}")))?
        .with_intra_threads(1)
        .map_err(|err| config_invalid(format!("ONNX intra-thread config failed: {err}")))?
        .with_execution_providers(execution_providers(spec.provider_policy))
        .map_err(|err| config_invalid(format!("ONNX provider config failed: {err}")))?
        .commit_from_file(&spec.model_file)
        .map_err(|err| config_invalid(format!("load custom ONNX model failed: {err}")))?;
    let session = CudaDropGuard::new(session, spec.provider_policy);
    let dim = output_dim(session.as_ref())?;
    let shape = SlotShape::Dense(dim);
    if let Some(expected) = spec.expected_shape
        && expected != shape
    {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "custom ONNX output shape {shape:?} != declared {expected:?}"
        )));
    }
    let tokenizer = Tokenizer::from_file(&spec.tokenizer)
        .map_err(|err| config_invalid(format!("load tokenizer failed: {err}")))?;
    let corpus_hash = sha256_digest(&[
        b"onnx-custom-v1",
        spec.model_id.as_bytes(),
        spec.pooling.as_str().as_bytes(),
        format!("{:?}", spec.norm_policy).as_bytes(),
    ]);
    let contract = FrozenLensContract::new(
        spec.name,
        weights_sha256,
        corpus_hash,
        shape,
        spec.modality,
        LensDType::F32,
        spec.norm_policy,
    );
    let runtime = CustomOnnxRuntime {
        session: session.into_inner(),
        tokenizer,
        pooling: spec.pooling,
        norm_policy: spec.norm_policy,
        dim,
        max_tokens,
    };
    Ok(OnnxLens::from_custom_parts(
        contract,
        files,
        spec.provider_policy,
        spec.max_batch,
        runtime,
    ))
}

impl CustomOnnxRuntime {
    fn run_token_batch(&mut self, batch: &TokenBatch) -> Result<Vec<Vec<f32>>> {
        let input_values = session_inputs(&self.session, batch)?;
        let outputs = self
            .session
            .run(input_values)
            .map_err(|err| config_invalid(format!("custom ONNX inference failed: {err}")))?;
        let output = output_tensor(&outputs)?;
        let (shape, values) = output.try_extract_tensor::<f32>().map_err(|err| {
            config_invalid(format!("custom ONNX output is not f32 tensor: {err}"))
        })?;
        pool_output_batch(shape, values, batch, self.pooling, self.dim)
    }
}

pub fn pooling_from_config(path: &Path) -> Result<PoolingPolicy> {
    let value = validate_config(path)?;
    let Some(raw) = value
        .get("pooling")
        .or_else(|| value.get("pooling_policy"))
        .and_then(Value::as_str)
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
pub(super) fn pool_output(
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

fn model_files(spec: &OnnxFileSpec) -> OnnxModelFiles {
    let cache_dir = spec
        .model_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    OnnxModelFiles {
        cache_dir,
        model_code: spec.model_id.clone(),
        model_file: spec.model_file.clone(),
        tokenizer: spec.tokenizer.clone(),
        config: spec.config.clone(),
        special_tokens_map: spec.config.clone(),
        tokenizer_config: spec.tokenizer.clone(),
        contract_paths: spec.contract_paths.clone(),
    }
}

fn ensure_file(label: &str, path: &Path) -> Result<()> {
    if path.is_file() {
        return Ok(());
    }
    Err(config_invalid(format!(
        "custom ONNX {label} file {} is missing",
        path.display()
    )))
}

fn validate_config(path: &Path) -> Result<Value> {
    let bytes = fs::read(path).map_err(|err| {
        config_invalid(format!("read ONNX config {} failed: {err}", path.display()))
    })?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|err| {
        config_invalid(format!(
            "parse ONNX config {} failed: {err}",
            path.display()
        ))
    })?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(config_invalid("ONNX config must be a JSON object"))
    }
}

fn output_dim(session: &Session) -> Result<u32> {
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
    u32::try_from(dim).map_err(|_| CalyxError::lens_dim_mismatch("custom ONNX dim exceeds u32"))
}

fn positive_usize(value: i64) -> Option<usize> {
    usize::try_from(value).ok().filter(|value| *value > 0)
}

fn output_tensor<'a, 'r>(
    outputs: &'a ort::session::SessionOutputs<'r>,
) -> Result<&'a ort::value::DynValue> {
    for name in ["sentence_embedding", "last_hidden_state", "pooler_output"] {
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
            let index = mask
                .iter()
                .take(seq)
                .rposition(|value| *value > 0)
                .unwrap_or(seq.saturating_sub(1));
            Ok(values[index * dim..(index + 1) * dim].to_vec())
        }
        PoolingPolicy::Mean => {
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
