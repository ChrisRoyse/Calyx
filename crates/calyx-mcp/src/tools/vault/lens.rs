use std::fs;
use std::path::PathBuf;

use calyx_core::{
    Asymmetry, CalyxError, Input, Lens, LensId, Modality, QuantPolicy, SlotShape, SlotVector,
};
use calyx_registry::frozen::sha256_digest;
use calyx_registry::{
    AlgorithmicLens, CapabilityCard, FrozenLensContract, LensDType, LensRuntime, LensSpec,
    NormPolicy, ProfileProbe, Registry, TeiHttpLens, profile_lens,
};
use serde_json::Value;

use crate::server::{ToolError, ToolResult};

use super::validate_path_safe;

const DEFAULT_PROFILE_NAME: &str = "profile-lens";
const DEFAULT_ALGORITHMIC_KIND: &str = "byte-features";

#[derive(Debug)]
pub(super) struct BuiltLens {
    pub(super) lens_id: LensId,
    pub(super) spec: LensSpec,
    runtime: BuiltRuntime,
}

#[derive(Debug)]
enum BuiltRuntime {
    Algorithmic(AlgorithmicLens, FrozenLensContract),
    Tei(TeiHttpLens, FrozenLensContract),
    Declared(DeclaredLens, FrozenLensContract),
}

impl BuiltLens {
    pub(super) fn register(self, registry: &mut Registry) -> Result<LensId, CalyxError> {
        match self.runtime {
            BuiltRuntime::Algorithmic(lens, contract) => {
                registry.register_frozen_with_spec(lens, contract, self.spec)
            }
            BuiltRuntime::Tei(lens, contract) => {
                registry.register_frozen_with_spec(lens, contract, self.spec)
            }
            BuiltRuntime::Declared(lens, contract) => {
                registry.register_frozen_with_spec(lens, contract, self.spec)
            }
        }
    }
}

pub(super) fn build_lens(
    name: &str,
    runtime: &str,
    endpoint: Option<&str>,
    weights: Option<&str>,
    shape: Option<&str>,
    modality: Option<&str>,
) -> ToolResult<BuiltLens> {
    validate_path_safe("lens name", name)?;
    let modality = parse_modality(modality)?;
    match runtime.replace('_', "-").as_str() {
        "algorithmic" => build_algorithmic_lens(name, DEFAULT_ALGORITHMIC_KIND, shape, modality),
        "tei-http" => build_tei_lens(name, endpoint, shape, modality),
        "onnx" => build_declared_lens(name, "onnx", endpoint, weights, shape, modality),
        "candle" => build_declared_lens(name, "candle", endpoint, weights, shape, modality),
        other => Err(ToolError::invalid_params(format!(
            "unknown runtime {other}; expected tei-http, onnx, candle, or algorithmic"
        ))),
    }
}

pub(super) fn profile_candidate(
    runtime: &str,
    endpoint: Option<&str>,
    weights: Option<&str>,
    probe: Option<&str>,
    modality: Option<&str>,
) -> ToolResult<CapabilityCard> {
    let built = build_lens(
        DEFAULT_PROFILE_NAME,
        runtime,
        endpoint,
        weights,
        None,
        modality,
    )?;
    let lens_id = built.lens_id;
    let modality = built.spec.modality;
    let mut registry = Registry::new();
    built.register(&mut registry)?;
    let probes = profile_probes(probe, modality)?;
    Ok(profile_lens(&registry, lens_id, &probes)?)
}

fn build_algorithmic_lens(
    name: &str,
    kind: &str,
    shape: Option<&str>,
    modality: Modality,
) -> ToolResult<BuiltLens> {
    let lens = AlgorithmicLens::byte_features(name, modality);
    if let Some(shape) = shape {
        let requested = parse_shape(shape)?;
        if requested != lens.shape() {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "requested shape {requested:?} does not match algorithmic {kind} shape {:?}",
                lens.shape()
            ))
            .into());
        }
    }
    let contract = lens.contract().clone();
    let spec = spec_from_contract(
        name,
        LensRuntime::Algorithmic {
            kind: kind.to_string(),
        },
        &contract,
    );
    Ok(BuiltLens {
        lens_id: contract.lens_id(),
        spec,
        runtime: BuiltRuntime::Algorithmic(lens, contract),
    })
}

fn build_tei_lens(
    name: &str,
    endpoint: Option<&str>,
    shape: Option<&str>,
    modality: Modality,
) -> ToolResult<BuiltLens> {
    let endpoint = endpoint.ok_or_else(|| {
        ToolError::Calyx(CalyxError::lens_unreachable(
            "tei-http runtime requires endpoint",
        ))
    })?;
    let output = shape
        .map(parse_shape)
        .transpose()?
        .unwrap_or(SlotShape::Dense(768));
    let dim = dense_dim(output)?;
    let lens = TeiHttpLens::new(name, endpoint, modality, dim);
    let contract = FrozenLensContract::tei_http(name, endpoint, modality, dim);
    let spec = spec_from_contract(
        name,
        LensRuntime::TeiHttp {
            endpoint: endpoint.to_string(),
        },
        &contract,
    );
    Ok(BuiltLens {
        lens_id: contract.lens_id(),
        spec,
        runtime: BuiltRuntime::Tei(lens, contract),
    })
}

fn build_declared_lens(
    name: &str,
    runtime: &str,
    endpoint: Option<&str>,
    weights: Option<&str>,
    shape: Option<&str>,
    modality: Modality,
) -> ToolResult<BuiltLens> {
    let output = shape
        .map(parse_shape)
        .transpose()?
        .unwrap_or(SlotShape::Dense(768));
    let weights_hash = weights_hash(weights, runtime, endpoint)?;
    let contract = FrozenLensContract::new(
        name,
        weights_hash,
        sha256_digest(&[runtime.as_bytes(), endpoint.unwrap_or("").as_bytes()]),
        output,
        modality,
        LensDType::F32,
        NormPolicy::finite_only(),
    );
    let spec = spec_from_contract(
        name,
        declared_runtime(runtime, endpoint, weights)?,
        &contract,
    );
    let lens = DeclaredLens {
        id: contract.lens_id(),
        shape: output,
        modality,
    };
    Ok(BuiltLens {
        lens_id: contract.lens_id(),
        spec,
        runtime: BuiltRuntime::Declared(lens, contract),
    })
}

fn parse_modality(value: Option<&str>) -> ToolResult<Modality> {
    match value.unwrap_or("text").trim().to_ascii_lowercase().as_str() {
        "text" => Ok(Modality::Text),
        "code" => Ok(Modality::Code),
        "image" => Ok(Modality::Image),
        "audio" => Ok(Modality::Audio),
        "video" => Ok(Modality::Video),
        "structured" => Ok(Modality::Structured),
        "mixed" => Ok(Modality::Mixed),
        other => Err(ToolError::invalid_params(format!(
            "unknown modality {other}; expected text, code, image, audio, video, structured, or mixed"
        ))),
    }
}

fn declared_runtime(
    runtime: &str,
    endpoint: Option<&str>,
    weights: Option<&str>,
) -> ToolResult<LensRuntime> {
    let files = weights.into_iter().map(PathBuf::from).collect();
    match runtime {
        "candle" => Ok(LensRuntime::CandleLocal {
            model_id: endpoint.unwrap_or("declared-candle").to_string(),
            files,
            dtype: "f32".to_string(),
            pooling: "mean".to_string(),
        }),
        "onnx" => Ok(LensRuntime::Onnx {
            model_id: endpoint.unwrap_or("declared-onnx").to_string(),
            files,
        }),
        _ => unreachable!("runtime already validated"),
    }
}

fn spec_from_contract(name: &str, runtime: LensRuntime, contract: &FrozenLensContract) -> LensSpec {
    LensSpec {
        name: name.to_string(),
        runtime,
        output: contract.shape(),
        modality: contract.modality(),
        weights_sha256: contract.weights_sha256(),
        corpus_hash: contract.corpus_hash(),
        norm_policy: contract.norm_policy(),
        max_batch: None,
        axis: Some(name.to_string()),
        asymmetry: Asymmetry::None,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: calyx_registry::spec::default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    }
}

fn parse_shape(value: &str) -> ToolResult<SlotShape> {
    let Some((kind, dim)) = value.trim().split_once('(') else {
        return Err(ToolError::invalid_params(
            "shape must be Dense(<dim>) or Sparse(<dim>)",
        ));
    };
    let dim = dim
        .trim_end_matches(')')
        .parse::<u32>()
        .map_err(|err| ToolError::invalid_params(format!("parse shape dimension: {err}")))?;
    if dim == 0 {
        return Err(ToolError::invalid_params("shape dimension must be > 0"));
    }
    match kind.to_ascii_lowercase().as_str() {
        "dense" => Ok(SlotShape::Dense(dim)),
        "sparse" => Ok(SlotShape::Sparse(dim)),
        _ => Err(ToolError::invalid_params(
            "shape must be Dense(<dim>) or Sparse(<dim>)",
        )),
    }
}

fn dense_dim(shape: SlotShape) -> ToolResult<u32> {
    match shape {
        SlotShape::Dense(dim) => Ok(dim),
        other => Err(CalyxError::lens_dim_mismatch(format!(
            "runtime requires dense output, got {other:?}"
        ))
        .into()),
    }
}

fn weights_hash(
    weights: Option<&str>,
    runtime: &str,
    endpoint: Option<&str>,
) -> ToolResult<[u8; 32]> {
    if let Some(path) = weights {
        let bytes = fs::read(path)
            .map_err(|err| CalyxError::lens_unreachable(format!("read weights failed: {err}")))?;
        return Ok(sha256_digest(&[&bytes]));
    }
    Ok(sha256_digest(&[
        runtime.as_bytes(),
        endpoint.unwrap_or("").as_bytes(),
    ]))
}

#[derive(Debug)]
struct DeclaredLens {
    id: LensId,
    shape: SlotShape,
    modality: Modality,
}

impl Lens for DeclaredLens {
    fn id(&self) -> LensId {
        self.id
    }

    fn shape(&self) -> SlotShape {
        self.shape
    }

    fn modality(&self) -> Modality {
        self.modality
    }

    fn measure(&self, _input: &Input) -> calyx_core::Result<SlotVector> {
        Err(CalyxError::lens_unreachable(format!(
            "lens {} is declared but its runtime is unavailable in this process",
            self.id
        )))
    }
}

fn profile_probes(path: Option<&str>, modality: Modality) -> ToolResult<Vec<ProfileProbe>> {
    let values = if let Some(path) = path {
        let text = fs::read_to_string(path)
            .map_err(|err| CalyxError::lens_unreachable(format!("read probe set failed: {err}")))?;
        let mut probes = Vec::new();
        for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
            probes.push(profile_probe_from_line(line, modality)?);
        }
        return if probes.is_empty() {
            Err(ToolError::invalid_params(
                "profile_lens probe set must not be empty",
            ))
        } else {
            Ok(probes)
        };
    } else {
        vec![
            "calyx profile alpha".to_string(),
            "calyx profile beta".to_string(),
            "calyx profile gamma".to_string(),
        ]
    };
    Ok(values
        .into_iter()
        .map(|value| ProfileProbe::new(Input::new(modality, value.into_bytes())))
        .collect())
}

fn profile_probe_from_line(line: &str, modality: Modality) -> ToolResult<ProfileProbe> {
    match serde_json::from_str::<Value>(line) {
        Ok(Value::String(text)) => Ok(ProfileProbe::new(Input::new(modality, text.into_bytes()))),
        Ok(Value::Object(map)) => {
            let text = map
                .get("input")
                .or_else(|| map.get("text"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ToolError::invalid_params("profile probe object requires string input or text")
                })?;
            let label = map
                .get("label")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            Ok(ProfileProbe {
                input: Input::new(modality, text.as_bytes().to_vec()),
                label,
            })
        }
        Ok(_) => Err(ToolError::invalid_params(
            "profile probe JSONL must be a string or object",
        )),
        Err(_) => Ok(ProfileProbe::new(Input::new(
            modality,
            line.as_bytes().to_vec(),
        ))),
    }
}
