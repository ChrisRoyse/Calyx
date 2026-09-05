use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Modality, Result, SlotShape};
use serde_json::Value;

mod fastembed_contract;

use fastembed_contract::{
    fastembed_bgem3_contract, fastembed_reranker_contract, fastembed_sparse_contract,
};

use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::runtime::candle::{self, CandlePoolingPolicy, CandlePrecision};
use crate::runtime::common::DEFAULT_MAX_TOKENS;
use crate::runtime::onnx::{custom_contract_corpus_hash, custom_pooling_from_config};
use crate::runtime::static_lookup::STATIC_LOOKUP_FULL_DOCUMENT_POLICY;
use crate::{AlgorithmicLens, LensRuntime, LensSpec, MultimodalAdapterLens, Qwen3ModelFiles};

const DEFAULT_COLBERT_ONNX: &str = "onnx/model_fp16.onnx";
const DEFAULT_QWEN3_MODEL: &str = "Qwen/Qwen3-Embedding-0.6B";
const STATIC_LOOKUP_MAGIC: &[u8; 8] = b"CXLKUP1\0";
const STATIC_LOOKUP_HEADER_LEN: usize = 24;

/// Derive the exact frozen runtime contract without constructing a model session.
///
/// This is intentionally public so content-addressed deployment records can
/// snapshot and later compare the runtime identity before allocating a GPU or
/// mutating a registry. Runtime-specific configuration files are still read and
/// validated; callers must treat an error as a hard deployment failure.
pub fn derive_runtime_contract_from_spec(spec: &LensSpec) -> Result<FrozenLensContract> {
    match &spec.runtime {
        LensRuntime::Algorithmic { kind } => algorithmic_contract(spec, kind),
        LensRuntime::TeiHttp { endpoint } => tei_contract(spec, endpoint),
        LensRuntime::TeiHttpSegmented { endpoint, policy } => {
            tei_segmented_contract(spec, endpoint, *policy)
        }
        LensRuntime::ExternalCmd { cmd, args } => external_contract(spec, cmd, args),
        LensRuntime::CandleLocal {
            model_id,
            files,
            dtype,
            pooling,
        } => candle_contract(spec, model_id, files, dtype, pooling),
        LensRuntime::Onnx { model_id, files } => onnx_contract(spec, model_id, files),
        LensRuntime::OnnxColbert { model_id, files } => {
            onnx_colbert_contract(spec, model_id, files)
        }
        LensRuntime::FastembedSparse { model_id, files } => {
            fastembed_sparse_contract(spec, model_id, files)
        }
        LensRuntime::FastembedBgem3 {
            model_id,
            files,
            output,
            engine,
        } => fastembed_bgem3_contract(spec, model_id, files, *output, *engine),
        LensRuntime::FastembedReranker { model_id, files } => {
            fastembed_reranker_contract(spec, model_id, files)
        }
        LensRuntime::FastembedQwen3 {
            model_id,
            files,
            dtype,
            max_tokens,
        } => qwen3_contract(spec, model_id, files, dtype, *max_tokens),
        LensRuntime::StaticLookup {
            embeddings_file,
            tokenizer,
            dim,
        } => static_lookup_contract(spec, embeddings_file, tokenizer, *dim),
        LensRuntime::MultimodalAdapter { .. } => {
            Ok(MultimodalAdapterLens::from_lens_spec(spec)?.contract())
        }
    }
}

fn algorithmic_contract(spec: &LensSpec, kind: &str) -> Result<FrozenLensContract> {
    Ok(
        AlgorithmicLens::from_runtime_kind(&spec.name, spec.modality, kind, spec.output)?
            .contract()
            .clone(),
    )
}

fn tei_contract(spec: &LensSpec, endpoint: &str) -> Result<FrozenLensContract> {
    let dim = dense_dim(spec.output).ok_or_else(|| {
        lens_config_invalid(format!(
            "TEI lens {} requires dense output shape, got {:?}",
            spec.name, spec.output
        ))
    })?;
    Ok(FrozenLensContract::tei_http(
        &spec.name,
        endpoint,
        spec.modality,
        dim,
    ))
}

fn tei_segmented_contract(
    spec: &LensSpec,
    endpoint: &str,
    policy: crate::TeiSegmentationPolicy,
) -> Result<FrozenLensContract> {
    policy.validate()?;
    // A pooled lane emits one dense vector per document, so it must declare a
    // dense output shape; a segment-preserving lane must declare multi (#1980).
    let dim = match (policy.is_pooled(), spec.output) {
        (false, SlotShape::Multi { token_dim }) => token_dim,
        (true, SlotShape::Dense(dim)) => dim,
        (pooled, other) => {
            let expected = if pooled { "dense" } else { "multi" };
            return Err(lens_config_invalid(format!(
                "segmented TEI lens {} requires {expected} output shape, got {other:?}",
                spec.name
            )));
        }
    };
    if policy.is_pooled() {
        return Ok(FrozenLensContract::tei_http_segmented_pooled(
            &spec.name,
            endpoint,
            spec.modality,
            dim,
            policy.fingerprint(),
        ));
    }
    Ok(FrozenLensContract::tei_http_segmented(
        &spec.name,
        endpoint,
        spec.modality,
        dim,
        policy.fingerprint(),
    ))
}

fn external_contract(spec: &LensSpec, cmd: &str, args: &[String]) -> Result<FrozenLensContract> {
    let dim = dense_dim(spec.output).ok_or_else(|| {
        lens_config_invalid(format!(
            "external command lens {} requires dense output shape, got {:?}",
            spec.name, spec.output
        ))
    })?;
    let args_text = args.join("\0");
    Ok(FrozenLensContract::new(
        spec.name.clone(),
        sha256_digest(&[cmd.as_bytes(), args_text.as_bytes()]),
        sha256_digest(&[b"external-cmd-runtime-v1"]),
        SlotShape::Dense(dim),
        spec.modality,
        LensDType::F32,
        NormPolicy::None,
    ))
}

fn candle_contract(
    spec: &LensSpec,
    model_id: &str,
    files: &[PathBuf],
    dtype: &str,
    pooling: &str,
) -> Result<FrozenLensContract> {
    let [weights, tokenizer, config, ..] = files else {
        return Err(lens_config_invalid(
            "LensRuntime::CandleLocal requires weights, tokenizer, and config paths",
        ));
    };
    ensure_file("candle weights", weights)?;
    ensure_file("candle tokenizer", tokenizer)?;
    ensure_file("candle config", config)?;
    let dim = dense_hidden_size(config, "candle")?;
    let precision = CandlePrecision::parse(dtype)?;
    let pooling = CandlePoolingPolicy::parse(pooling)?;
    // Same formula and same warm-path device policy as CandleLens::from_lens_spec,
    // so the session-free derivation is byte-identical to the runtime contract.
    let finite_replay =
        candle::contract_finite_replay_precision(candle::LENS_SPEC_DEVICE_POLICY, precision);
    let corpus_hash = candle::contract_corpus_hash(
        model_id,
        DEFAULT_MAX_TOKENS,
        precision,
        pooling,
        spec.norm_policy,
        finite_replay,
    );
    Ok(FrozenLensContract::new(
        spec.name.clone(),
        spec.weights_sha256,
        corpus_hash,
        SlotShape::Dense(dim),
        Modality::Text,
        LensDType::F32,
        spec.norm_policy,
    ))
}

fn onnx_contract(spec: &LensSpec, model_id: &str, files: &[PathBuf]) -> Result<FrozenLensContract> {
    let [_model, _tokenizer, config, ..] = files else {
        return Err(lens_config_invalid(
            "LensRuntime::Onnx requires model, tokenizer, and config paths",
        ));
    };
    for path in files {
        ensure_file("ONNX contract artifact", path)?;
    }
    // Same pooling parser and same corpus-hash formula as the custom ONNX
    // runtime constructor. The sparse decision mirrors `output_from_session`:
    // a custom ONNX lens is sparse if and only if its declared output shape is
    // sparse (SPLADE-style lenses).
    let pooling = custom_pooling_from_config(config)?;
    let sparse = matches!(spec.output, SlotShape::Sparse(_));
    Ok(FrozenLensContract::new(
        spec.name.clone(),
        spec.weights_sha256,
        custom_contract_corpus_hash(model_id, sparse, pooling, spec.norm_policy),
        spec.output,
        spec.modality,
        LensDType::F32,
        spec.norm_policy,
    ))
}

fn onnx_colbert_contract(
    spec: &LensSpec,
    model_id: &str,
    files: &[PathBuf],
) -> Result<FrozenLensContract> {
    let [_model, _tokenizer, _config, ..] = files else {
        return Err(lens_config_invalid(
            "LensRuntime::OnnxColbert requires model, tokenizer, and config paths",
        ));
    };
    for path in files {
        ensure_file("ONNX ColBERT contract artifact", path)?;
    }
    Ok(FrozenLensContract::new(
        spec.name.clone(),
        spec.weights_sha256,
        sha256_digest(&[
            b"onnx-colbert-token-v1",
            model_id.as_bytes(),
            DEFAULT_COLBERT_ONNX.as_bytes(),
            b"attention-mask-unpooled-finite",
        ]),
        spec.output,
        Modality::Text,
        LensDType::F32,
        NormPolicy::Finite,
    ))
}

fn qwen3_contract(
    spec: &LensSpec,
    model_id: &str,
    files: &[PathBuf],
    dtype: &str,
    max_tokens: usize,
) -> Result<FrozenLensContract> {
    if max_tokens == 0 {
        return Err(lens_config_invalid(
            "fastembed-qwen3 max_tokens must be > 0",
        ));
    }
    let model_id = qwen3_model_id(model_id)?;
    let files = Qwen3ModelFiles::from_paths(model_id.clone(), files.to_vec())?;
    for path in files.artifact_paths() {
        ensure_file("fastembed-qwen3 contract artifact", &path)?;
    }
    let precision = CandlePrecision::parse(dtype)?;
    let max_tokens = max_tokens.to_string();
    let dim = dense_hidden_size(&files.config, "Qwen3")?;
    Ok(FrozenLensContract::new(
        spec.name.clone(),
        spec.weights_sha256,
        sha256_digest(&[
            b"fastembed-qwen3-text-v1",
            model_id.as_bytes(),
            precision.as_str().as_bytes(),
            max_tokens.as_bytes(),
            b"left-padding,last-token,l2",
        ]),
        SlotShape::Dense(dim),
        Modality::Text,
        LensDType::F32,
        NormPolicy::unit(),
    ))
}

fn static_lookup_contract(
    spec: &LensSpec,
    embeddings_file: &Path,
    tokenizer: &Path,
    expected_dim: u32,
) -> Result<FrozenLensContract> {
    let (dim, dtype) = static_lookup_header(embeddings_file)?;
    if dim != expected_dim {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "static lookup matrix dim {dim} != expected {expected_dim}"
        )));
    }
    ensure_file("static lookup tokenizer", tokenizer)?;
    Ok(FrozenLensContract::new(
        spec.name.clone(),
        spec.weights_sha256,
        sha256_digest(&[
            STATIC_LOOKUP_FULL_DOCUMENT_POLICY.as_bytes(),
            dim.to_string().as_bytes(),
            dtype.as_bytes(),
        ]),
        SlotShape::Dense(dim),
        Modality::Text,
        LensDType::F32,
        spec.norm_policy,
    ))
}

fn static_lookup_header(path: &Path) -> Result<(u32, &'static str)> {
    let mut file = File::open(path).map_err(|err| {
        lens_config_invalid(format!(
            "open static lookup matrix {} failed: {err}",
            path.display()
        ))
    })?;
    let len = file
        .metadata()
        .map_err(|err| {
            lens_config_invalid(format!(
                "stat static lookup matrix {} failed: {err}",
                path.display()
            ))
        })?
        .len() as usize;
    let mut header = [0_u8; STATIC_LOOKUP_HEADER_LEN];
    file.read_exact(&mut header).map_err(|err| {
        lens_config_invalid(format!(
            "read static lookup matrix header {} failed: {err}",
            path.display()
        ))
    })?;
    if len < STATIC_LOOKUP_HEADER_LEN || &header[..8] != STATIC_LOOKUP_MAGIC {
        return Err(lens_config_invalid(format!(
            "static lookup matrix {} has invalid magic/header",
            path.display()
        )));
    }
    let rows = u32::from_le_bytes(header[8..12].try_into().expect("rows"));
    let dim = u32::from_le_bytes(header[12..16].try_into().expect("dim"));
    let (dtype, width) = match header[16] {
        1 => ("int8", 1usize),
        2 => ("f16", 2usize),
        3 => ("f32", 4usize),
        other => {
            return Err(lens_config_invalid(format!(
                "unsupported static lookup dtype {other}"
            )));
        }
    };
    let expected = STATIC_LOOKUP_HEADER_LEN
        .checked_add(rows as usize * dim as usize * width)
        .ok_or_else(|| CalyxError::lens_dim_mismatch("static lookup matrix size overflow"))?;
    if len != expected {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "static lookup matrix byte length {len} != expected {expected}"
        )));
    }
    Ok((dim, dtype))
}

fn dense_hidden_size(path: &Path, label: &str) -> Result<u32> {
    let value = read_json(path, label)?;
    let hidden = value
        .get("hidden_size")
        .and_then(Value::as_u64)
        .ok_or_else(|| lens_config_invalid(format!("{label} config missing hidden_size")))?;
    u32::try_from(hidden).map_err(|_| CalyxError::lens_dim_mismatch("hidden_size exceeds u32"))
}

fn read_json(path: &Path, label: &str) -> Result<Value> {
    let bytes = fs::read(path).map_err(|err| {
        lens_config_invalid(format!(
            "read {label} config {} failed: {err}",
            path.display()
        ))
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|err| lens_config_invalid(format!("parse {label} config failed: {err}")))
}

fn qwen3_model_id(raw: &str) -> Result<String> {
    match normalized(raw).as_str() {
        "qwen/qwen3-embedding-0.6b" | "qwen3-embedding-0.6b" | "qwen3-0.6b" => {
            Ok(DEFAULT_QWEN3_MODEL.to_string())
        }
        other => Err(CalyxError::lens_unreachable(format!(
            "unsupported fastembed-qwen3 model {other}; expected {DEFAULT_QWEN3_MODEL}"
        ))),
    }
}

fn dense_dim(shape: SlotShape) -> Option<u32> {
    match shape {
        SlotShape::Dense(dim) => Some(dim),
        SlotShape::Sparse(_) | SlotShape::Multi { .. } => None,
    }
}

pub(super) fn ensure_file(label: &str, path: &Path) -> Result<()> {
    if path.is_file() {
        return Ok(());
    }
    Err(lens_config_invalid(format!(
        "{label} {} is missing",
        path.display()
    )))
}

fn normalized(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

pub(super) fn lens_config_invalid(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_LENS_CONFIG_INVALID",
        message: message.into(),
        remediation: "fix persisted LensSpec runtime fields or re-register the lens",
    }
}
