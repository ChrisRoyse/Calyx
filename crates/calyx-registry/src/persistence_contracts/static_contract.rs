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
use crate::runtime::candle::{CandlePoolingPolicy, CandlePrecision};
use crate::runtime::common::DEFAULT_MAX_TOKENS;
use crate::{AlgorithmicEncoder, LensRuntime, LensSpec, Qwen3ModelFiles};

const DEFAULT_COLBERT_ONNX: &str = "onnx/model_fp16.onnx";
const DEFAULT_QWEN3_MODEL: &str = "Qwen/Qwen3-Embedding-0.6B";
const STATIC_LOOKUP_MAGIC: &[u8; 8] = b"CXLKUP1\0";
const STATIC_LOOKUP_HEADER_LEN: usize = 24;

pub(crate) fn derive_runtime_contract_from_spec(spec: &LensSpec) -> Result<FrozenLensContract> {
    match &spec.runtime {
        LensRuntime::Algorithmic { kind } => algorithmic_contract(spec, kind),
        LensRuntime::TeiHttp { endpoint } => tei_contract(spec, endpoint),
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
        } => fastembed_bgem3_contract(spec, model_id, files, *output),
        LensRuntime::FastembedReranker { model_id, files } => {
            fastembed_reranker_contract(spec, model_id, files)
        }
        LensRuntime::FastembedQwen3 {
            model_id,
            files,
            dtype,
        } => qwen3_contract(spec, model_id, files, dtype),
        LensRuntime::StaticLookup {
            embeddings_file,
            tokenizer,
            dim,
        } => static_lookup_contract(spec, embeddings_file, tokenizer, *dim),
        LensRuntime::MultimodalAdapter { .. } => Ok(spec.declared_contract()),
    }
}

fn algorithmic_contract(spec: &LensSpec, kind: &str) -> Result<FrozenLensContract> {
    let encoder = algorithmic_encoder(kind, spec.output).ok_or_else(|| {
        lens_config_invalid(format!(
            "unsupported algorithmic lens kind {kind} for persisted lens {}",
            spec.name
        ))
    })?;
    if encoder == AlgorithmicEncoder::ByteFeatures {
        return Ok(FrozenLensContract::algorithmic_byte_features(
            &spec.name,
            spec.modality,
        ));
    }
    let encoder_text = format!("{encoder:?}:{}", encoder.dim());
    Ok(FrozenLensContract::new(
        spec.name.clone(),
        sha256_digest(&[b"algorithmic-runtime-v2", encoder_text.as_bytes()]),
        sha256_digest(&[b"algorithmic-data-oblivious"]),
        encoder.shape(),
        spec.modality,
        LensDType::F32,
        NormPolicy::None,
    ))
}

fn algorithmic_encoder(kind: &str, shape: SlotShape) -> Option<AlgorithmicEncoder> {
    match kind {
        "byte_features" | "byte-features" | "byte" => Some(AlgorithmicEncoder::ByteFeatures),
        "scalar" => Some(AlgorithmicEncoder::Scalar),
        "ast_style" | "ast-style" => Some(AlgorithmicEncoder::AstStyle),
        "gdelt_cameo" | "gdelt-cameo" => Some(AlgorithmicEncoder::GdeltCameo),
        "gdelt_actor_geo" | "gdelt-actor-geo" => Some(AlgorithmicEncoder::GdeltActorGeo {
            dim: sparse_dim(shape)?,
        }),
        "gdelt_source_domain" | "gdelt-source-domain" => {
            Some(AlgorithmicEncoder::GdeltSourceDomain {
                dim: sparse_dim(shape)?,
            })
        }
        "gdelt_event_geo" | "gdelt-event-geo" => Some(AlgorithmicEncoder::GdeltEventGeo {
            dim: sparse_dim(shape)?,
        }),
        "gdelt_actor_pair" | "gdelt-actor-pair" => Some(AlgorithmicEncoder::GdeltActorPair {
            dim: sparse_dim(shape)?,
        }),
        "gdelt_event_actor" | "gdelt-event-actor" => Some(AlgorithmicEncoder::GdeltEventActor {
            dim: sparse_dim(shape)?,
        }),
        "gdelt_tone_signal" | "gdelt-tone-signal" => Some(AlgorithmicEncoder::GdeltToneSignal {
            dim: sparse_dim(shape)?,
        }),
        "gdelt_source_event" | "gdelt-source-event" => Some(AlgorithmicEncoder::GdeltSourceEvent {
            dim: sparse_dim(shape)?,
        }),
        "sparse" | "sparse_keywords" | "sparse-keywords" => {
            Some(AlgorithmicEncoder::SparseKeywords {
                dim: sparse_dim(shape)?,
            })
        }
        "token_hash" | "token-hash" | "multi_hash" | "multi-hash" => {
            Some(AlgorithmicEncoder::TokenHash {
                token_dim: token_dim(shape)?,
            })
        }
        "one_hot" | "one-hot" => Some(AlgorithmicEncoder::OneHot {
            buckets: dense_dim(shape)?,
        }),
        value => {
            if let Some(dim) = value
                .strip_prefix("sparse_keywords:")
                .or_else(|| value.strip_prefix("sparse-keywords:"))
                .and_then(|dim| dim.parse().ok())
            {
                return Some(AlgorithmicEncoder::SparseKeywords { dim });
            }
            if let Some(token_dim) = value
                .strip_prefix("token_hash:")
                .or_else(|| value.strip_prefix("token-hash:"))
                .or_else(|| value.strip_prefix("multi_hash:"))
                .or_else(|| value.strip_prefix("multi-hash:"))
                .and_then(|dim| dim.parse().ok())
            {
                return Some(AlgorithmicEncoder::TokenHash { token_dim });
            }
            value
                .strip_prefix("one_hot:")
                .or_else(|| value.strip_prefix("one-hot:"))
                .and_then(|buckets| buckets.parse().ok())
                .map(|buckets| AlgorithmicEncoder::OneHot { buckets })
        }
    }
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
    let finite_replay_text = match precision {
        CandlePrecision::F16 | CandlePrecision::BF16 => precision.as_str(),
        CandlePrecision::F32 => "none",
    };
    let max_tokens = DEFAULT_MAX_TOKENS.to_string();
    let norm_text = format!("{:?}", spec.norm_policy);
    let corpus_hash = sha256_digest(&[
        b"candle-local-bert-v2",
        model_id.as_bytes(),
        max_tokens.as_bytes(),
        precision.as_str().as_bytes(),
        pooling.as_str().as_bytes(),
        norm_text.as_bytes(),
        finite_replay_text.as_bytes(),
    ]);
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
    let pooling = onnx_pooling_from_config(config)?;
    let norm_text = format!("{:?}", spec.norm_policy);
    Ok(FrozenLensContract::new(
        spec.name.clone(),
        spec.weights_sha256,
        sha256_digest(&[
            b"onnx-custom-v1",
            model_id.as_bytes(),
            pooling.as_bytes(),
            norm_text.as_bytes(),
        ]),
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
) -> Result<FrozenLensContract> {
    let model_id = qwen3_model_id(model_id)?;
    let files = Qwen3ModelFiles::from_paths(model_id.clone(), files.to_vec())?;
    for path in files.artifact_paths() {
        ensure_file("fastembed-qwen3 contract artifact", &path)?;
    }
    let precision = CandlePrecision::parse(dtype)?;
    let max_tokens = "32768".to_string();
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
            b"static-lookup-model2vec-v1",
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

fn onnx_pooling_from_config(path: &Path) -> Result<&'static str> {
    let value = read_json(path, "ONNX")?;
    let Some(raw) = value
        .get("pooling")
        .or_else(|| value.get("pooling_policy"))
        .and_then(Value::as_str)
    else {
        return Ok("mean");
    };
    match raw {
        "mean" => Ok("mean"),
        "cls" => Ok("cls"),
        "last_token" | "last-token" => Ok("last_token"),
        other => Err(lens_config_invalid(format!(
            "unsupported ONNX pooling {other}"
        ))),
    }
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

fn sparse_dim(shape: SlotShape) -> Option<u32> {
    match shape {
        SlotShape::Sparse(dim) => Some(dim),
        SlotShape::Dense(_) | SlotShape::Multi { .. } => None,
    }
}

fn token_dim(shape: SlotShape) -> Option<u32> {
    match shape {
        SlotShape::Multi { token_dim } => Some(token_dim),
        SlotShape::Dense(_) | SlotShape::Sparse(_) => None,
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
