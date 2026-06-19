use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{Asymmetry, CalyxError, Modality, QuantPolicy, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::frozen::{NormPolicy, sha256_digest};
use crate::runtime::adapters::{allow_noncommercial_from_env, ensure_license_allowed};
use crate::spec::{LensRuntime, LensSpec};

use super::algorithmic_manifest::{
    algorithmic_kind, is_algorithmic_runtime, output_shape as algorithmic_output_shape,
};

const CONFIG_INVALID: &str = "CALYX_LENS_CONFIG_INVALID";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LensForgeFile {
    pub role: String,
    pub path: PathBuf,
    pub sha256: String,
    #[serde(default)]
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LensForgeManifest {
    pub name: String,
    pub modality: Modality,
    pub runtime: String,
    pub dim: u32,
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
    let output = algorithmic_output_shape(&manifest.runtime, manifest.dim)?;
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
        retrieval_only: false,
        excluded_from_dedup: false,
    })
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
    let _ = algorithmic_output_shape(&manifest.runtime, manifest.dim)?;
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
struct VerifiedFile {
    role: String,
    path: PathBuf,
    bytes: Vec<u8>,
}

fn read_and_verify_files(
    manifest: &LensForgeManifest,
    base_dir: &Path,
) -> Result<Vec<VerifiedFile>> {
    let mut files = Vec::with_capacity(manifest.files.len());
    for file in ordered_manifest_files(&manifest.files) {
        let path = resolve_manifest_path(base_dir, &file.path);
        let bytes = fs::read(&path).map_err(|err| {
            config_invalid(format!(
                "read lensforge artifact {} failed: {err}",
                path.display()
            ))
        })?;
        let actual = plain_sha256_hex(&bytes);
        if !hex_eq(&actual, &file.sha256) {
            return Err(CalyxError::lens_frozen_violation(format!(
                "lensforge artifact {} sha256 {} != manifest {}",
                path.display(),
                actual,
                file.sha256
            )));
        }
        if file.bytes != 0 && file.bytes != bytes.len() as u64 {
            return Err(config_invalid(format!(
                "lensforge artifact {} byte count {} != manifest {}",
                path.display(),
                bytes.len(),
                file.bytes
            )));
        }
        files.push(VerifiedFile {
            role: file.role.clone(),
            path,
            bytes,
        });
    }
    Ok(files)
}

fn spec_weights_sha256(
    manifest: &LensForgeManifest,
    artifacts: &[VerifiedFile],
) -> Result<[u8; 32]> {
    if is_algorithmic_runtime(&manifest.runtime) && artifacts.is_empty() {
        return Ok(sha256_digest(&[
            b"lensforge-algorithmic-v1",
            manifest.name.as_bytes(),
            manifest.runtime.as_bytes(),
            &manifest.dim.to_be_bytes(),
            modality_token(manifest.modality).as_bytes(),
        ]));
    }
    let model = weight_anchor(manifest, artifacts)?;
    let model_sha = plain_sha256_hex(&model.bytes);
    if !hex_eq(&model_sha, &manifest.weights_sha256) {
        return Err(CalyxError::lens_frozen_violation(format!(
            "lensforge model weights sha256 {model_sha} != manifest {}",
            manifest.weights_sha256
        )));
    }
    if let Some(expected) = &manifest.artifact_set_sha256 {
        let contract_artifacts = contract_artifacts(manifest, artifacts)?;
        let parts = contract_artifacts
            .iter()
            .map(|file| file.bytes.as_slice())
            .collect::<Vec<_>>();
        let actual = hex_from_bytes(&sha256_digest(&parts));
        if !hex_eq(&actual, expected) {
            return Err(CalyxError::lens_frozen_violation(format!(
                "lensforge artifact_set_sha256 {actual} != manifest {expected}"
            )));
        }
        parse_hex_32(expected)
    } else {
        parse_hex_32(&manifest.weights_sha256)
    }
}

fn weight_anchor<'a>(
    manifest: &LensForgeManifest,
    artifacts: &'a [VerifiedFile],
) -> Result<&'a VerifiedFile> {
    artifacts
        .iter()
        .find(|file| is_model_role(&file.role))
        .or_else(|| {
            is_adapter_runtime(&manifest.runtime)
                .then(|| artifacts.iter().find(|file| file.role == "adapter"))
                .flatten()
        })
        .ok_or_else(|| config_invalid("lensforge manifest requires a model file"))
}

fn runtime_from_manifest(
    manifest: &LensForgeManifest,
    artifacts: &[VerifiedFile],
) -> Result<LensRuntime> {
    if let Some(kind) = algorithmic_kind(&manifest.runtime) {
        return Ok(LensRuntime::Algorithmic {
            kind: kind.to_string(),
        });
    }
    let files = artifacts
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    match manifest.runtime.as_str() {
        "onnx" | "onnx-int8" | "onnx-custom" | "onnx-fastembed" => Ok(LensRuntime::Onnx {
            model_id: manifest.source_hf_id.clone(),
            files,
        }),
        "candle" | "candle-fp16" | "candle-local" => Ok(LensRuntime::CandleLocal {
            model_id: manifest.source_hf_id.clone(),
            files,
            dtype: manifest.dtype.clone(),
            pooling: manifest.pooling.clone(),
        }),
        "tei" | "tei-http" | "tei_http" => Ok(LensRuntime::TeiHttp {
            endpoint: manifest
                .endpoint
                .clone()
                .ok_or_else(|| config_invalid("lensforge TEI endpoint is required"))?,
        }),
        "model2vec" | "static_lookup" | "static-lookup" => {
            let embeddings_file = artifact_by_role(artifacts, is_model_role)?;
            let tokenizer = artifact_by_role(artifacts, |role| role == "tokenizer")?;
            Ok(LensRuntime::StaticLookup {
                embeddings_file,
                tokenizer,
                dim: manifest.dim,
            })
        }
        "external-cmd" | "external_cmd" => Ok(LensRuntime::ExternalCmd {
            cmd: manifest.source_hf_id.clone(),
            args: artifacts
                .iter()
                .map(|file| file.path.display().to_string())
                .collect::<Vec<_>>(),
        }),
        "adapter" | "multimodal-adapter" | "multimodal_adapter" => {
            let adapter_config = artifact_by_role(artifacts, |role| role == "adapter")?;
            Ok(LensRuntime::MultimodalAdapter {
                axis: modality_token(manifest.modality).to_string(),
                model_id: manifest.source_hf_id.clone(),
                adapter_config: Some(adapter_config),
                files,
            })
        }
        "model2vec-external" => Ok(LensRuntime::ExternalCmd {
            cmd: "model2vec".to_string(),
            args: files
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>(),
        }),
        other => Err(config_invalid(format!(
            "unsupported lensforge runtime {other}"
        ))),
    }
}

fn is_tei_runtime(runtime: &str) -> bool {
    matches!(runtime, "tei" | "tei-http" | "tei_http")
}

fn is_adapter_runtime(runtime: &str) -> bool {
    matches!(
        runtime,
        "adapter" | "multimodal-adapter" | "multimodal_adapter"
    )
}

fn ordered_manifest_files(files: &[LensForgeFile]) -> Vec<&LensForgeFile> {
    let mut ordered = files.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|file| (role_rank(&file.role), file.path.clone()));
    ordered
}

fn role_rank(role: &str) -> u8 {
    match role {
        "model" | "weights" | "embeddings" => 0,
        "tokenizer" => 1,
        "config" => 2,
        "preprocessor" => 3,
        "tokenizer_config" => 4,
        "special_tokens_map" => 5,
        _ => 9,
    }
}

fn contract_artifacts<'a>(
    manifest: &LensForgeManifest,
    artifacts: &'a [VerifiedFile],
) -> Result<Vec<&'a VerifiedFile>> {
    match manifest.runtime.as_str() {
        "model2vec" | "static_lookup" | "static-lookup" => Ok(vec![
            artifact_ref_by_role(artifacts, is_model_role)?,
            artifact_ref_by_role(artifacts, |role| role == "tokenizer")?,
        ]),
        _ => Ok(artifacts.iter().collect()),
    }
}

fn artifact_by_role(
    artifacts: &[VerifiedFile],
    predicate: impl Fn(&str) -> bool,
) -> Result<PathBuf> {
    Ok(artifact_ref_by_role(artifacts, predicate)?.path.clone())
}

fn artifact_ref_by_role(
    artifacts: &[VerifiedFile],
    predicate: impl Fn(&str) -> bool,
) -> Result<&VerifiedFile> {
    artifacts
        .iter()
        .find(|file| predicate(&file.role))
        .ok_or_else(|| config_invalid("lensforge manifest missing static lookup artifact"))
}

fn is_model_role(role: &str) -> bool {
    matches!(role, "model" | "weights" | "embeddings")
}

fn resolve_manifest_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

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

fn modality_token(modality: Modality) -> &'static str {
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

fn plain_sha256_hex(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    hex_from_bytes(&digest)
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
