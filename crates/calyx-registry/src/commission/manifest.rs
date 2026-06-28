use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use calyx_core::{Asymmetry, CalyxError, Modality, QuantPolicy, Result, SlotShape};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::frozen::{LengthDelimitedSha256, NormPolicy, sha256_digest};
use crate::runtime::adapters::{allow_noncommercial_from_env, ensure_license_allowed};
use crate::spec::LensSpec;

use super::algorithmic_manifest::{
    is_algorithmic_runtime, output_shape as algorithmic_output_shape,
};
use super::manifest_runtime::runtime_from_manifest;

const CONFIG_INVALID: &str = "CALYX_LENS_CONFIG_INVALID";
const STREAM_HASH_BUFFER_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LensForgeFile {
    pub role: String,
    pub path: PathBuf,
    pub sha256: String,
    #[serde(default)]
    pub bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LensForgeShape {
    Dense { dim: u32 },
    Sparse { dim: u32 },
    Multi { token_dim: u32 },
}

impl LensForgeShape {
    pub fn from_slot_shape(shape: SlotShape) -> Self {
        match shape {
            SlotShape::Dense(dim) => Self::Dense { dim },
            SlotShape::Sparse(dim) => Self::Sparse { dim },
            SlotShape::Multi { token_dim } => Self::Multi { token_dim },
        }
    }

    pub fn to_slot_shape(self) -> SlotShape {
        match self {
            Self::Dense { dim } => SlotShape::Dense(dim),
            Self::Sparse { dim } => SlotShape::Sparse(dim),
            Self::Multi { token_dim } => SlotShape::Multi { token_dim },
        }
    }

    pub fn dim(self) -> u32 {
        match self {
            Self::Dense { dim } | Self::Sparse { dim } => dim,
            Self::Multi { token_dim } => token_dim,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LensForgeManifest {
    pub name: String,
    pub modality: Modality,
    pub runtime: String,
    pub dim: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<LensForgeShape>,
    pub dtype: String,
    pub weights_sha256: String,
    #[serde(default)]
    pub artifact_set_sha256: Option<String>,
    pub files: Vec<LensForgeFile>,
    pub pooling: String,
    pub norm: String,
    pub source_hf_id: String,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub non_commercial: bool,
    #[serde(default = "crate::spec::default_quant_default")]
    pub quant_default: QuantPolicy,
    #[serde(default)]
    pub truncate_dim: Option<u32>,
    #[serde(default = "crate::spec::default_recall_delta")]
    pub recall_delta: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_batch: Option<usize>,
}

impl LensForgeManifest {
    pub fn output_shape(&self) -> Result<SlotShape> {
        let derived = algorithmic_output_shape(&self.runtime, self.dim)?;
        let Some(shape) = self.shape else {
            return Ok(derived);
        };
        if shape.dim() != self.dim {
            return Err(config_invalid(format!(
                "lensforge manifest shape dim {} != dim {}",
                shape.dim(),
                self.dim
            )));
        }
        let declared = shape.to_slot_shape();
        if declared != derived {
            return Err(config_invalid(format!(
                "lensforge manifest shape {declared:?} does not match runtime {} dim {} ({derived:?})",
                self.runtime, self.dim
            )));
        }
        Ok(declared)
    }
}

pub fn lens_spec_from_manifest_path(path: impl AsRef<Path>) -> Result<LensSpec> {
    let path = path.as_ref();
    let bytes = fs::read(path).map_err(|err| {
        config_invalid(format!(
            "read lensforge manifest {} failed: {err}",
            path.display()
        ))
    })?;
    let manifest: LensForgeManifest = serde_json::from_slice(&bytes).map_err(|err| {
        config_invalid(format!(
            "parse lensforge manifest {} failed: {err}",
            path.display()
        ))
    })?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    lens_spec_from_manifest(&manifest, base)
}

pub fn lens_spec_from_manifest(manifest: &LensForgeManifest, base_dir: &Path) -> Result<LensSpec> {
    lens_spec_from_manifest_with_license_override(
        manifest,
        base_dir,
        allow_noncommercial_from_env(),
    )
}

pub fn lens_spec_from_manifest_with_license_override(
    manifest: &LensForgeManifest,
    base_dir: &Path,
    allow_non_commercial: bool,
) -> Result<LensSpec> {
    validate_required(manifest)?;
    if manifest.max_batch == Some(0) {
        return Err(config_invalid("lensforge manifest max_batch must be > 0"));
    }
    ensure_license_allowed(
        manifest.license.as_deref(),
        manifest.non_commercial,
        allow_non_commercial,
    )?;
    let artifacts = read_and_verify_files(manifest, base_dir)?;
    let output = manifest.output_shape()?;
    let weights_sha256 = spec_weights_sha256(manifest, &artifacts)?;
    let corpus_hash = sha256_digest(&[
        b"lensforge-manifest-v1",
        manifest.name.as_bytes(),
        manifest.source_hf_id.as_bytes(),
        manifest.runtime.as_bytes(),
        modality_token(manifest.modality).as_bytes(),
        manifest.pooling.as_bytes(),
        manifest.norm.as_bytes(),
    ]);
    let retrieval_only = is_retrieval_only_runtime(&manifest.runtime);
    Ok(LensSpec {
        name: manifest.name.clone(),
        runtime: runtime_from_manifest(manifest, &artifacts)?,
        output,
        modality: manifest.modality,
        weights_sha256,
        corpus_hash,
        norm_policy: norm_policy(&manifest.norm)?,
        max_batch: manifest.max_batch,
        axis: Some(manifest.name.clone()),
        asymmetry: Asymmetry::None,
        quant_default: manifest.quant_default,
        truncate_dim: manifest.truncate_dim,
        recall_delta: manifest.recall_delta,
        retrieval_only,
        excluded_from_dedup: retrieval_only,
    })
}

fn is_retrieval_only_runtime(runtime: &str) -> bool {
    matches!(runtime, "fastembed-reranker")
}

fn validate_required(manifest: &LensForgeManifest) -> Result<()> {
    if manifest.name.trim().is_empty() {
        return Err(config_invalid("lensforge manifest name is required"));
    }
    if manifest.source_hf_id.trim().is_empty() {
        return Err(config_invalid(
            "lensforge manifest source_hf_id is required",
        ));
    }
    if manifest.runtime.trim().is_empty() {
        return Err(config_invalid("lensforge manifest runtime is required"));
    }
    if is_tei_runtime(&manifest.runtime)
        && manifest
            .endpoint
            .as_deref()
            .is_none_or(|endpoint| endpoint.trim().is_empty())
    {
        return Err(config_invalid(
            "lensforge TEI manifest endpoint is required",
        ));
    }
    if manifest.dim == 0 {
        return Err(config_invalid("lensforge manifest dim must be > 0"));
    }
    let _ = manifest.output_shape()?;
    if let Some(truncate_dim) = manifest.truncate_dim
        && (truncate_dim == 0 || truncate_dim > manifest.dim)
    {
        return Err(config_invalid(format!(
            "truncate_dim {truncate_dim} must be in 1..={}",
            manifest.dim
        )));
    }
    if !manifest.recall_delta.is_finite() || manifest.recall_delta < 0.0 {
        return Err(config_invalid(
            "recall_delta must be finite and non-negative",
        ));
    }
    if manifest.files.is_empty() && !is_algorithmic_runtime(&manifest.runtime) {
        return Err(config_invalid("lensforge manifest files are required"));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(super) struct VerifiedFile {
    pub(super) role: String,
    pub(super) path: PathBuf,
    sha256: String,
    bytes: u64,
}

mod artifacts;
use artifacts::{is_tei_runtime, read_and_verify_files, spec_weights_sha256};

fn norm_policy(raw: &str) -> Result<NormPolicy> {
    match raw {
        "l2" | "unit" => Ok(NormPolicy::unit()),
        "finite" => Ok(NormPolicy::Finite),
        "none" => Ok(NormPolicy::None),
        other => Err(config_invalid(format!(
            "unsupported lensforge norm {other}"
        ))),
    }
}

pub(super) fn modality_token(modality: Modality) -> &'static str {
    match modality {
        Modality::Text => "text",
        Modality::Code => "code",
        Modality::Image => "image",
        Modality::Audio => "audio",
        Modality::Video => "video",
        Modality::Protein => "protein",
        Modality::Dna => "dna",
        Modality::Molecule => "molecule",
        Modality::Structured => "structured",
        Modality::Mixed => "mixed",
    }
}

fn parse_hex_32(raw: &str) -> Result<[u8; 32]> {
    let value = raw.trim();
    if value.len() != 64 {
        return Err(config_invalid(format!(
            "expected 64 hex chars, got {}",
            value.len()
        )));
    }
    let mut out = [0u8; 32];
    for (idx, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let text = std::str::from_utf8(chunk)
            .map_err(|err| config_invalid(format!("invalid hex utf8: {err}")))?;
        out[idx] = u8::from_str_radix(text, 16)
            .map_err(|err| config_invalid(format!("invalid hex digest: {err}")))?;
    }
    Ok(out)
}

fn hex_from_bytes(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_eq(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right.trim())
}

fn config_invalid(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CONFIG_INVALID,
        message: message.into(),
        remediation: "fix the lensforge manifest or regenerated artifacts",
    }
}
