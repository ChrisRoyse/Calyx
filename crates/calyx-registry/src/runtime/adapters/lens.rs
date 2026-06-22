use std::env;
use std::path::PathBuf;

use calyx_core::{
    Asymmetry, CalyxError, Input, Lens, LensId, Modality, Result, SlotShape, SlotVector,
};

use super::axis::MultimodalAxis;
use super::bridge;
use super::config::{
    MultimodalAdapterConfig, MultimodalAdapterProvider, config_invalid, load_adapter_config,
};
use super::validate::validate_input;
use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::lens::ensure_input_modality;
use crate::runtime::common::{hash_files, normalize_unit};
use crate::spec::{LensRuntime, LensSpec};

pub const CALYX_LICENSE_DENIED: &str = "CALYX_LICENSE_DENIED";
pub const CALYX_ALLOW_NONCOMMERCIAL_LENSES_ENV: &str = "CALYX_ALLOW_NONCOMMERCIAL_LENSES";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultimodalAdapterSpec {
    pub name: String,
    pub axis: MultimodalAxis,
    pub model_id: String,
    pub dim: u32,
    pub license: Option<String>,
    pub allow_non_commercial: bool,
    pub adapter_config: Option<PathBuf>,
    pub files: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct MultimodalAdapterLens {
    name: String,
    axis: MultimodalAxis,
    model_id: String,
    dim: u32,
    adapter_config: MultimodalAdapterConfig,
    files: Vec<PathBuf>,
    weights_sha256: [u8; 32],
    corpus_hash: [u8; 32],
    id: LensId,
}

impl MultimodalAdapterLens {
    pub fn from_adapter_spec(spec: MultimodalAdapterSpec) -> Result<Self> {
        if spec.dim == 0 {
            return Err(config_invalid("multimodal adapter dim must be > 0"));
        }
        ensure_license_allowed(
            spec.license.as_deref(),
            spec.license
                .as_deref()
                .is_some_and(is_non_commercial_license),
            spec.allow_non_commercial,
        )?;
        let adapter_config_path = spec
            .adapter_config
            .as_deref()
            .ok_or_else(|| config_invalid("multimodal adapter config is required"))?;
        let adapter_config = load_adapter_config(
            adapter_config_path,
            spec.axis,
            &spec.model_id,
            Some(spec.dim),
        )?;
        let contract_paths = if spec.files.is_empty() {
            adapter_config.contract_paths()
        } else {
            spec.files.clone()
        };
        let weights_sha256 = hash_files(&contract_paths).map_err(|err| {
            config_invalid(format!("hash multimodal adapter files failed: {err}"))
        })?;
        let corpus_hash = sha256_digest(&[
            b"multimodal-onnx-adapter-v2",
            spec.name.as_bytes(),
            spec.axis.as_str().as_bytes(),
            spec.model_id.as_bytes(),
        ]);
        Self::from_parts(AdapterParts {
            name: spec.name,
            axis: spec.axis,
            model_id: spec.model_id,
            dim: spec.dim,
            adapter_config,
            files: contract_paths,
            weights_sha256,
            corpus_hash,
        })
    }

    pub fn from_lens_spec(spec: &LensSpec) -> Result<Self> {
        let LensRuntime::MultimodalAdapter {
            axis,
            model_id,
            adapter_config,
            files,
        } = &spec.runtime
        else {
            return Err(config_invalid("LensSpec runtime is not multimodal_adapter"));
        };
        let axis = MultimodalAxis::parse(axis)?;
        if spec.modality != axis.modality() {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "multimodal adapter axis {} expects {:?}, got {:?}",
                axis.as_str(),
                axis.modality(),
                spec.modality
            )));
        }
        let SlotShape::Dense(dim) = spec.output else {
            return Err(CalyxError::lens_dim_mismatch(
                "multimodal adapter requires dense output",
            ));
        };
        let adapter_config_path = adapter_config
            .as_deref()
            .ok_or_else(|| config_invalid("multimodal adapter config is required"))?;
        let adapter_config = load_adapter_config(adapter_config_path, axis, model_id, Some(dim))?;
        Self::from_parts(AdapterParts {
            name: spec.name.clone(),
            axis,
            model_id: model_id.clone(),
            dim,
            adapter_config,
            files: files.clone(),
            weights_sha256: spec.weights_sha256,
            corpus_hash: spec.corpus_hash,
        })
    }

    pub fn contract(&self) -> FrozenLensContract {
        FrozenLensContract::new(
            self.name.clone(),
            self.weights_sha256,
            self.corpus_hash,
            SlotShape::Dense(self.dim),
            self.axis.modality(),
            LensDType::F32,
            NormPolicy::unit(),
        )
    }

    pub fn lens_spec(&self) -> LensSpec {
        LensSpec {
            name: self.name.clone(),
            runtime: LensRuntime::MultimodalAdapter {
                axis: self.axis.as_str().to_string(),
                model_id: self.model_id.clone(),
                adapter_config: Some(self.adapter_config.path.clone()),
                files: self.files.clone(),
            },
            output: SlotShape::Dense(self.dim),
            modality: self.axis.modality(),
            weights_sha256: self.weights_sha256,
            corpus_hash: self.corpus_hash,
            norm_policy: NormPolicy::unit(),
            max_batch: None,
            axis: Some(format!("{}:{}", self.axis.as_str(), self.model_id)),
            asymmetry: Asymmetry::None,
            quant_default: calyx_core::QuantPolicy::turboquant_default(),
            truncate_dim: None,
            recall_delta: crate::spec::default_recall_delta(),
            retrieval_only: false,
            excluded_from_dedup: false,
        }
    }

    pub const fn axis(&self) -> MultimodalAxis {
        self.axis
    }

    pub const fn provider(&self) -> MultimodalAdapterProvider {
        self.adapter_config.provider
    }

    pub fn provider_detail(&self) -> &'static str {
        self.adapter_config.provider.detail()
    }

    fn from_parts(parts: AdapterParts) -> Result<Self> {
        if parts.dim == 0 {
            return Err(config_invalid("multimodal adapter dim must be > 0"));
        }
        let contract = FrozenLensContract::new(
            parts.name.clone(),
            parts.weights_sha256,
            parts.corpus_hash,
            SlotShape::Dense(parts.dim),
            parts.axis.modality(),
            LensDType::F32,
            NormPolicy::unit(),
        );
        Ok(Self {
            name: parts.name,
            axis: parts.axis,
            model_id: parts.model_id,
            dim: parts.dim,
            adapter_config: parts.adapter_config,
            files: parts.files,
            weights_sha256: parts.weights_sha256,
            corpus_hash: parts.corpus_hash,
            id: contract.lens_id(),
        })
    }
}

struct AdapterParts {
    name: String,
    axis: MultimodalAxis,
    model_id: String,
    dim: u32,
    adapter_config: MultimodalAdapterConfig,
    files: Vec<PathBuf>,
    weights_sha256: [u8; 32],
    corpus_hash: [u8; 32],
}

impl Lens for MultimodalAdapterLens {
    fn id(&self) -> LensId {
        self.id
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(self.dim)
    }

    fn modality(&self) -> Modality {
        self.axis.modality()
    }

    fn measure(&self, input: &Input) -> Result<SlotVector> {
        let mut batch = self.measure_batch(std::slice::from_ref(input))?;
        batch.pop().ok_or_else(|| {
            CalyxError::lens_dim_mismatch(format!(
                "multimodal adapter {} returned no vector",
                self.id
            ))
        })
    }

    fn measure_batch(&self, inputs: &[Input]) -> Result<Vec<SlotVector>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        for input in inputs {
            ensure_input_modality(self, input)?;
            validate_input(self.axis, input)?;
        }
        bridge::measure_batch(&self.adapter_config, inputs)?
            .into_iter()
            .map(|data| self.slot_from_row(data))
            .collect()
    }
}

impl MultimodalAdapterLens {
    fn slot_from_row(&self, mut data: Vec<f32>) -> Result<SlotVector> {
        if data.len() != self.dim as usize {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "multimodal adapter dim {} != expected {}",
                data.len(),
                self.dim
            )));
        }
        if data.iter().any(|value| !value.is_finite()) {
            return Err(CalyxError::lens_numerical_invariant(
                "multimodal adapter vector contains NaN or Inf",
            ));
        }
        normalize_unit(&mut data)?;
        Ok(SlotVector::Dense {
            dim: self.dim,
            data,
        })
    }
}

pub fn allow_noncommercial_from_env() -> bool {
    env::var(CALYX_ALLOW_NONCOMMERCIAL_LENSES_ENV)
        .map(|raw| {
            matches!(
                raw.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "allow" | "allowed"
            )
        })
        .unwrap_or(false)
}

pub fn ensure_license_allowed(
    license: Option<&str>,
    non_commercial: bool,
    allow_non_commercial: bool,
) -> Result<()> {
    let denied = non_commercial || license.is_some_and(is_non_commercial_license);
    if !denied || allow_non_commercial {
        return Ok(());
    }
    Err(CalyxError {
        code: CALYX_LICENSE_DENIED,
        message: format!(
            "non-commercial lens license {} requires explicit local allow flag",
            license.unwrap_or("unknown")
        ),
        remediation: "set CALYX_ALLOW_NONCOMMERCIAL_LENSES=true only for approved local experiments",
    })
}

pub fn is_non_commercial_license(raw: &str) -> bool {
    let lowered = raw.to_ascii_lowercase();
    let normalized = lowered.replace(['_', ' '], "-");
    normalized.contains("non-commercial")
        || normalized.contains("noncommercial")
        || normalized.contains("cc-by-nc")
        || normalized
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .any(|token| token == "nc")
}
