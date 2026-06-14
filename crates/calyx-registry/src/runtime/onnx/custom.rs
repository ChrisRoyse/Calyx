use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Input, Lens, Modality, Result, SlotShape, SlotVector};
use ort::session::{Session, SessionInputValue};
use ort::value::{Tensor, ValueType};
use serde_json::Value;
use tokenizers::Tokenizer;

use super::fastembed_runtime::execution_providers;
use super::{OnnxFileSpec, OnnxLens, OnnxModelFiles, PoolingPolicy, config_invalid};
use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::runtime::common::{DEFAULT_MAX_TOKENS, hash_files, normalize_unit, text_from_input};

pub struct CustomOnnxRuntime {
    session: Session,
    tokenizer: Tokenizer,
    pooling: PoolingPolicy,
    norm_policy: NormPolicy,
    dim: u32,
}

impl CustomOnnxRuntime {
    pub const fn dim(&self) -> u32 {
        self.dim
    }

    pub fn measure(&mut self, lens: &dyn Lens, input: &Input) -> Result<SlotVector> {
        let text = text_from_input(lens, input)?;
        let encoded = self
            .tokenizer
            .encode(text, true)
            .map_err(|err| config_invalid(format!("tokenizer encode failed: {err}")))?;
        let (ids, mask) = token_inputs(&encoded);
        let input_values = session_inputs(&self.session, &ids, &mask)?;
        let outputs = self
            .session
            .run(input_values)
            .map_err(|err| config_invalid(format!("custom ONNX inference failed: {err}")))?;
        let output = output_tensor(&outputs)?;
        let (shape, values) = output.try_extract_tensor::<f32>().map_err(|err| {
            config_invalid(format!("custom ONNX output is not f32 tensor: {err}"))
        })?;
        let mut data = pool_output(shape, values, &mask, self.pooling, self.dim)?;
        apply_norm(self.norm_policy, &mut data)?;
        Ok(SlotVector::Dense {
            dim: self.dim,
            data,
        })
    }
}

pub fn from_files(spec: OnnxFileSpec) -> Result<OnnxLens> {
    ensure_file("model", &spec.model_file)?;
    ensure_file("tokenizer", &spec.tokenizer)?;
    ensure_file("config", &spec.config)?;
    validate_config(&spec.config)?;
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
    let dim = output_dim(&session)?;
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
        Modality::Text,
        LensDType::F32,
        spec.norm_policy,
    );
    let runtime = CustomOnnxRuntime {
        session,
        tokenizer,
        pooling: spec.pooling,
        norm_policy: spec.norm_policy,
        dim,
    };
    Ok(OnnxLens::from_custom_parts(
        contract,
        files,
        spec.provider_policy,
        runtime,
    ))
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

pub(super) fn pool_output(
    shape: &[i64],
    values: &[f32],
    mask: &[i64],
    policy: PoolingPolicy,
    dim: u32,
) -> Result<Vec<f32>> {
    let dim = dim as usize;
    match shape {
        [1, actual_dim] if *actual_dim as usize == dim => Ok(values.to_vec()),
        [1, seq, actual_dim] if *actual_dim as usize == dim => {
            pool_tokens(values, *seq as usize, dim, mask, policy)
        }
        _ => Err(CalyxError::lens_dim_mismatch(format!(
            "custom ONNX output shape {shape:?} is incompatible with dense dim {dim}"
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

fn token_inputs(encoding: &tokenizers::Encoding) -> (Vec<i64>, Vec<i64>) {
    let mut ids = encoding
        .get_ids()
        .iter()
        .take(DEFAULT_MAX_TOKENS)
        .map(|id| i64::from(*id))
        .collect::<Vec<_>>();
    let mut mask = encoding
        .get_attention_mask()
        .iter()
        .take(DEFAULT_MAX_TOKENS)
        .map(|value| i64::from(*value))
        .collect::<Vec<_>>();
    if ids.is_empty() {
        ids.push(0);
        mask.push(0);
    }
    if mask.len() != ids.len() {
        mask.resize(ids.len(), 1);
    }
    (ids, mask)
}

fn session_inputs<'a>(
    session: &Session,
    ids: &[i64],
    mask: &[i64],
) -> Result<Vec<(String, SessionInputValue<'a>)>> {
    let shape = vec![1_i64, ids.len() as i64];
    let mut values = Vec::with_capacity(session.inputs().len());
    for input in session.inputs() {
        let name = input.name();
        let tensor = if name.contains("token_type_ids") || name.contains("segment") {
            Tensor::from_array((shape.clone(), vec![0_i64; ids.len()]))
        } else if name.contains("input_ids") || name.contains("token") {
            Tensor::from_array((shape.clone(), ids.to_vec()))
        } else if name.contains("attention_mask") || name.contains("mask") {
            Tensor::from_array((shape.clone(), mask.to_vec()))
        } else if name.contains("position_ids") || name.contains("position") {
            Tensor::from_array((shape.clone(), (0..ids.len() as i64).collect::<Vec<_>>()))
        } else {
            return Err(config_invalid(format!(
                "unsupported custom ONNX input {}",
                input.name()
            )));
        }
        .map_err(|err| config_invalid(format!("build ONNX tensor {} failed: {err}", name)))?;
        values.push((name.to_string(), SessionInputValue::from(tensor)));
    }
    Ok(values)
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
