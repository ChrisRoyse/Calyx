use std::fs;
use std::path::Path;

use calyx_core::{
    Asymmetry, CalyxError, Input, Lens, LensId, Modality, QuantPolicy, SlotShape, SlotVector,
};
use calyx_registry::frozen::sha256_digest;
use calyx_registry::{
    AlgorithmicLens, ExternalCmdLens, FrozenLensContract, LensDType, LensRuntime, LensSpec,
    NormPolicy, ProfileProbe, Registry, StructuredBinding, TeiHttpLens, TeiSegmentedLens,
};

mod segmented;
mod structured;

use crate::error::{CliError, CliResult};

const DEFAULT_ALGORITHMIC_KIND: &str = "byte-features";

#[derive(Debug)]
pub(super) struct BuiltLens {
    pub lens_id: LensId,
    pub spec: LensSpec,
    runtime: BuiltRuntime,
}

#[derive(Debug)]
enum BuiltRuntime {
    Algorithmic(AlgorithmicLens, FrozenLensContract),
    Tei(TeiHttpLens, FrozenLensContract),
    TeiSegmented(TeiSegmentedLens, FrozenLensContract),
    External(ExternalCmdLens, FrozenLensContract),
    Declared(DeclaredLens, FrozenLensContract),
}

impl BuiltLens {
    pub(super) fn register(self, registry: &mut Registry) -> calyx_core::Result<LensId> {
        match self.runtime {
            BuiltRuntime::Algorithmic(lens, contract) => {
                registry.register_frozen_with_spec(lens, contract, self.spec)
            }
            BuiltRuntime::Tei(lens, contract) => {
                registry.register_frozen_with_spec(lens, contract, self.spec)
            }
            BuiltRuntime::TeiSegmented(lens, contract) => {
                registry.register_frozen_with_spec(lens, contract, self.spec)
            }
            BuiltRuntime::External(lens, contract) => {
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
    weights: Option<&Path>,
    shape: Option<&str>,
    modality: Option<&str>,
) -> CliResult<BuiltLens> {
    validate_lens_name(name)?;
    let runtime_key = runtime.replace('_', "-");
    let default_modality = if runtime_key == "algorithmic:structured" {
        "structured"
    } else {
        "text"
    };
    let modality = parse_modality(modality.unwrap_or(default_modality))?;
    if runtime_key == "tei-http" {
        return build_tei_lens(name, endpoint, shape, modality);
    }
    if runtime_key == "tei-http-segmented" {
        return segmented::build_segmented_tei_lens(name, endpoint, shape, modality);
    }
    if runtime_key == "tei-http-segmented-pooled" {
        return segmented::build_pooled_segmented_tei_lens(name, endpoint, shape, modality);
    }
    if runtime_key == "external-cmd" {
        return build_external_lens(name, endpoint, shape, modality);
    }
    if runtime_key == "algorithmic:structured" {
        return structured::build_structured_lens(name, weights, shape, modality);
    }
    if let Some(kind) = runtime_key
        .strip_prefix("algorithmic:")
        .or_else(|| (runtime_key == "algorithmic").then_some(DEFAULT_ALGORITHMIC_KIND))
    {
        return build_algorithmic_lens(name, kind, shape, modality);
    }
    build_declared_lens(name, runtime, endpoint, weights, shape, modality)
}

pub(super) fn profile_probes(
    path: Option<&Path>,
    registry: &Registry,
    lens_id: LensId,
) -> CliResult<Vec<ProfileProbe>> {
    let spec = registry
        .lens_spec(lens_id)
        .ok_or_else(|| CalyxError::registry_unavailable(format!("lens {lens_id} missing spec")))?;
    if let LensRuntime::Algorithmic { kind } = &spec.runtime
        && StructuredBinding::is_runtime_kind(kind)
    {
        return structured::structured_profile_probes(path, kind);
    }
    let modality = spec.modality;
    let values = if let Some(path) = path {
        fs::read_to_string(path)?
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>()
    } else {
        vec![
            "calyx profile alpha".to_string(),
            "calyx profile beta".to_string(),
            "calyx profile gamma".to_string(),
        ]
    };
    if values.is_empty() {
        return Err(CliError::usage("profile-lens probe set must not be empty"));
    }
    Ok(values
        .into_iter()
        .map(|value| ProfileProbe::new(Input::new(modality, value.into_bytes())))
        .collect())
}

fn build_algorithmic_lens(
    name: &str,
    kind: &str,
    shape: Option<&str>,
    modality: Modality,
) -> CliResult<BuiltLens> {
    let requested = shape.map(parse_shape).transpose()?;
    let declared = requested.unwrap_or(default_algorithmic_shape(kind)?);
    let lens = AlgorithmicLens::from_runtime_kind(name, modality, kind, declared)?;
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

fn default_algorithmic_shape(kind: &str) -> CliResult<SlotShape> {
    if StructuredBinding::is_runtime_kind(kind) {
        return Ok(StructuredBinding::from_runtime_kind(kind)?.shape());
    }
    let normalized = kind.to_ascii_lowercase().replace('_', "-");
    let base = normalized
        .split_once(':')
        .map_or(normalized.as_str(), |(base, _)| base);
    let encoded_dim = normalized
        .split_once(':')
        .map(|(_, value)| {
            value
                .parse::<u32>()
                .map_err(|error| CliError::usage(format!("parse algorithmic dimension: {error}")))
        })
        .transpose()?;
    Ok(match base {
        "byte" | "byte-features" => SlotShape::Dense(16),
        "scalar" => SlotShape::Dense(1),
        "ast-style" => SlotShape::Dense(8),
        "legal-document-structure" => SlotShape::Dense(32),
        "gdelt-cameo" => SlotShape::Dense(16),
        "gdelt-actor-geo"
        | "gdelt-source-domain"
        | "gdelt-event-geo"
        | "gdelt-actor-pair"
        | "gdelt-event-actor"
        | "gdelt-tone-signal"
        | "gdelt-source-event"
        | "gdelt-action-geo"
        | "gdelt-actor-country"
        | "gdelt-source-host"
        | "gdelt-sqldate"
        | "gdelt-event-code" => SlotShape::Sparse(encoded_dim.unwrap_or(512)),
        "sparse" | "sparse-keywords" | "sparse-keywords-raw-tf" => {
            SlotShape::Sparse(encoded_dim.unwrap_or(30_522))
        }
        "token-hash" | "multi-hash" => SlotShape::Multi {
            token_dim: encoded_dim.unwrap_or(16),
        },
        "one-hot" => SlotShape::Dense(encoded_dim.ok_or_else(|| {
            CliError::usage("algorithmic one-hot requires an explicit dimension or shape")
        })?),
        other => {
            return Err(CliError::usage(format!(
                "unknown algorithmic runtime kind {other}"
            )));
        }
    })
}

fn build_tei_lens(
    name: &str,
    endpoint: Option<&str>,
    shape: Option<&str>,
    modality: Modality,
) -> CliResult<BuiltLens> {
    let output = shape
        .map(parse_shape)
        .transpose()?
        .unwrap_or(SlotShape::Dense(768));
    let dim = dense_dim(output)?;
    let endpoint = endpoint.unwrap_or(calyx_registry::DEFAULT_TEI_ENDPOINT);
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

fn build_external_lens(
    name: &str,
    endpoint: Option<&str>,
    shape: Option<&str>,
    modality: Modality,
) -> CliResult<BuiltLens> {
    let output = shape
        .map(parse_shape)
        .transpose()?
        .unwrap_or(SlotShape::Dense(16));
    let dim = dense_dim(output)?;
    let cmd = endpoint
        .ok_or_else(|| CliError::usage("external-cmd runtime requires --endpoint <executable>"))?;
    let lens = ExternalCmdLens::new(name, cmd, Vec::new(), modality, dim);
    let contract = FrozenLensContract::new(
        name,
        sha256_digest(&[cmd.as_bytes(), b""]),
        sha256_digest(&[b"external-cmd-runtime-v1"]),
        SlotShape::Dense(dim),
        modality,
        LensDType::F32,
        NormPolicy::None,
    );
    let spec = spec_from_contract(
        name,
        LensRuntime::ExternalCmd {
            cmd: cmd.to_string(),
            args: Vec::new(),
        },
        &contract,
    );
    Ok(BuiltLens {
        lens_id: contract.lens_id(),
        spec,
        runtime: BuiltRuntime::External(lens, contract),
    })
}

fn build_declared_lens(
    name: &str,
    runtime: &str,
    endpoint: Option<&str>,
    weights: Option<&Path>,
    shape: Option<&str>,
    modality: Modality,
) -> CliResult<BuiltLens> {
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

fn declared_runtime(
    runtime: &str,
    endpoint: Option<&str>,
    weights: Option<&Path>,
) -> CliResult<LensRuntime> {
    match runtime.replace('_', "-").as_str() {
        "candle-local" => Ok(LensRuntime::CandleLocal {
            model_id: endpoint.unwrap_or("declared-candle-local").to_string(),
            files: weights.into_iter().map(Path::to_path_buf).collect(),
            dtype: "f32".to_string(),
            pooling: "mean".to_string(),
        }),
        "onnx" => Ok(LensRuntime::Onnx {
            model_id: endpoint.unwrap_or("declared-onnx").to_string(),
            files: weights.into_iter().map(Path::to_path_buf).collect(),
        }),
        "multimodal-adapter" => Ok(LensRuntime::MultimodalAdapter {
            axis: endpoint.unwrap_or("mixed").to_string(),
            model_id: "declared-multimodal".to_string(),
            adapter_config: weights.map(Path::to_path_buf),
            files: weights.into_iter().map(Path::to_path_buf).collect(),
        }),
        other => Err(CliError::usage(format!(
            "unknown runtime {other}; expected algorithmic, tei-http, tei-http-segmented, tei-http-segmented-pooled, external-cmd, candle-local, onnx, or multimodal-adapter"
        ))),
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

fn parse_shape(value: &str) -> CliResult<SlotShape> {
    let Some((kind, dim)) = value.trim().split_once('(') else {
        return Err(CliError::usage(
            "shape must be Dense(<dim>), Sparse(<dim>), or Multi(<token_dim>)",
        ));
    };
    let dim = dim
        .trim_end_matches(')')
        .parse::<u32>()
        .map_err(|err| CliError::usage(format!("parse shape dimension in {value}: {err}")))?;
    if dim == 0 {
        return Err(CliError::usage("shape dimension must be > 0"));
    }
    match kind.to_ascii_lowercase().as_str() {
        "dense" => Ok(SlotShape::Dense(dim)),
        "sparse" => Ok(SlotShape::Sparse(dim)),
        "multi" => Ok(SlotShape::Multi { token_dim: dim }),
        _ => Err(CliError::usage(
            "shape must be Dense(<dim>), Sparse(<dim>), or Multi(<token_dim>)",
        )),
    }
}

fn parse_modality(value: &str) -> CliResult<Modality> {
    match value.replace('-', "_").to_ascii_lowercase().as_str() {
        "text" => Ok(Modality::Text),
        "code" => Ok(Modality::Code),
        "image" => Ok(Modality::Image),
        "audio" => Ok(Modality::Audio),
        "video" => Ok(Modality::Video),
        "protein" => Ok(Modality::Protein),
        "dna" => Ok(Modality::Dna),
        "molecule" => Ok(Modality::Molecule),
        "structured" => Ok(Modality::Structured),
        "mixed" => Ok(Modality::Mixed),
        other => Err(CliError::usage(format!("unknown modality {other}"))),
    }
}

fn dense_dim(shape: SlotShape) -> CliResult<u32> {
    match shape {
        SlotShape::Dense(dim) => Ok(dim),
        other => Err(CalyxError::lens_dim_mismatch(format!(
            "runtime requires dense output, got {other:?}"
        ))
        .into()),
    }
}

fn weights_hash(
    weights: Option<&Path>,
    runtime: &str,
    endpoint: Option<&str>,
) -> CliResult<[u8; 32]> {
    if let Some(path) = weights {
        return Ok(sha256_digest(&[&fs::read(path)?]));
    }
    Ok(sha256_digest(&[
        runtime.as_bytes(),
        endpoint.unwrap_or("").as_bytes(),
    ]))
}

fn validate_lens_name(name: &str) -> CliResult {
    if name.is_empty() || name.chars().any(char::is_whitespace) || name.contains(['/', '\\']) {
        return Err(CliError::usage("lens name must be non-empty and path-safe"));
    }
    Ok(())
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

#[cfg(test)]
#[path = "lens/tests.rs"]
mod tests;
