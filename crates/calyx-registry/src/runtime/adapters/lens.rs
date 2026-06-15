use std::env;

use calyx_core::{
    Asymmetry, CalyxError, Input, Lens, LensId, Modality, Result, SlotShape, SlotVector,
};

use super::axis::MultimodalAxis;
use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::lens::ensure_input_modality;
use crate::runtime::common::normalize_unit;
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
}

#[derive(Clone, Debug)]
pub struct MultimodalAdapterLens {
    name: String,
    axis: MultimodalAxis,
    model_id: String,
    dim: u32,
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
        let license = spec.license.as_deref().unwrap_or("unknown");
        let weights_sha256 = sha256_digest(&[
            b"multimodal-adapter-v1",
            spec.axis.as_str().as_bytes(),
            spec.model_id.as_bytes(),
            license.as_bytes(),
        ]);
        let corpus_hash = sha256_digest(&[
            b"ph74-multimodal-pack-v1",
            spec.name.as_bytes(),
            spec.axis.as_str().as_bytes(),
            spec.model_id.as_bytes(),
        ]);
        Self::from_parts(
            spec.name,
            spec.axis,
            spec.model_id,
            spec.dim,
            weights_sha256,
            corpus_hash,
        )
    }

    pub fn from_lens_spec(spec: &LensSpec) -> Result<Self> {
        let LensRuntime::MultimodalAdapter { axis, model_id } = &spec.runtime else {
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
        Self::from_parts(
            spec.name.clone(),
            axis,
            model_id.clone(),
            dim,
            spec.weights_sha256,
            spec.corpus_hash,
        )
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
            },
            output: SlotShape::Dense(self.dim),
            modality: self.axis.modality(),
            weights_sha256: self.weights_sha256,
            corpus_hash: self.corpus_hash,
            norm_policy: NormPolicy::unit(),
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

    fn from_parts(
        name: String,
        axis: MultimodalAxis,
        model_id: String,
        dim: u32,
        weights_sha256: [u8; 32],
        corpus_hash: [u8; 32],
    ) -> Result<Self> {
        if dim == 0 {
            return Err(config_invalid("multimodal adapter dim must be > 0"));
        }
        let contract = FrozenLensContract::new(
            name.clone(),
            weights_sha256,
            corpus_hash,
            SlotShape::Dense(dim),
            axis.modality(),
            LensDType::F32,
            NormPolicy::unit(),
        );
        Ok(Self {
            name,
            axis,
            model_id,
            dim,
            weights_sha256,
            corpus_hash,
            id: contract.lens_id(),
        })
    }
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
        ensure_input_modality(self, input)?;
        validate_input(self.axis, input)?;
        let mut data = deterministic_projection(self.axis, &self.model_id, &input.bytes, self.dim);
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

fn validate_input(axis: MultimodalAxis, input: &Input) -> Result<()> {
    if input.bytes.is_empty() {
        return Err(invalid_input(axis, "input is empty"));
    }
    match axis {
        MultimodalAxis::Image => validate_image(&input.bytes),
        MultimodalAxis::Audio => validate_audio(&input.bytes),
        MultimodalAxis::Protein => validate_alpha(&input.bytes, axis, b"ACDEFGHIKLMNPQRSTVWY"),
        MultimodalAxis::Dna => validate_alpha(&input.bytes, axis, b"ACGTN"),
        MultimodalAxis::Molecule => validate_smiles(&input.bytes),
    }
}

fn validate_image(bytes: &[u8]) -> Result<()> {
    let png = bytes.starts_with(b"\x89PNG\r\n\x1a\n");
    let jpeg = bytes.starts_with(&[0xff, 0xd8, 0xff]);
    if png || jpeg {
        Ok(())
    } else {
        Err(invalid_input(
            MultimodalAxis::Image,
            "expected PNG or JPEG bytes",
        ))
    }
}

fn validate_audio(bytes: &[u8]) -> Result<()> {
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE" {
        Ok(())
    } else {
        Err(invalid_input(
            MultimodalAxis::Audio,
            "expected RIFF/WAVE bytes",
        ))
    }
}

fn validate_alpha(bytes: &[u8], axis: MultimodalAxis, allowed: &[u8]) -> Result<()> {
    let ok = bytes
        .iter()
        .copied()
        .all(|byte| allowed.contains(&byte.to_ascii_uppercase()));
    if ok {
        Ok(())
    } else {
        Err(invalid_input(axis, "contains unsupported sequence symbol"))
    }
}

fn validate_smiles(bytes: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| invalid_input(MultimodalAxis::Molecule, "SMILES input is not UTF-8"))?;
    let allowed = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789[]()=#@+-/\\\\.%";
    if text.chars().any(|ch| ch.is_ascii_alphabetic())
        && text.chars().all(|ch| allowed.contains(ch))
    {
        Ok(())
    } else {
        Err(invalid_input(
            MultimodalAxis::Molecule,
            "SMILES contains unsupported token",
        ))
    }
}

fn deterministic_projection(
    axis: MultimodalAxis,
    model_id: &str,
    bytes: &[u8],
    dim: u32,
) -> Vec<f32> {
    let mut out = Vec::with_capacity(dim as usize);
    let mut counter = 0_u32;
    while out.len() < dim as usize {
        let digest = sha256_digest(&[
            b"multimodal-adapter-vector-v1",
            axis.as_str().as_bytes(),
            model_id.as_bytes(),
            bytes,
            &counter.to_le_bytes(),
        ]);
        for chunk in digest.chunks_exact(4) {
            if out.len() == dim as usize {
                break;
            }
            let raw = u32::from_le_bytes(chunk.try_into().expect("sha256 chunk is 4 bytes"));
            let unit = (raw as f64 / u32::MAX as f64) * 2.0 - 1.0;
            out.push(unit as f32);
        }
        counter = counter.wrapping_add(1);
    }
    out
}

fn invalid_input(axis: MultimodalAxis, message: &str) -> CalyxError {
    CalyxError::lens_dim_mismatch(format!("{} adapter {message}", axis.as_str()))
}

fn config_invalid(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_LENS_CONFIG_INVALID",
        message: message.into(),
        remediation: "fix the multimodal adapter lens spec",
    }
}
