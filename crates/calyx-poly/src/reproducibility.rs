//! Bit-for-bit reproduction checks for local forecast-agent artifacts.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    AgentForecastArtifactRequest, AgentForecastManifest, AgentSourceSnapshotRef, PolyError, Result,
    write_agent_forecast_artifacts,
};

pub const AGENT_REPRODUCTION_SCHEMA_VERSION: &str = "poly.agent.reproduction.v1";
pub const AGENT_REPRODUCTION_BIT_FOR_BIT: &str = "CALYX_POLY_AGENT_REPRODUCED_BIT_FOR_BIT";
const LEDGER_KIND_AGENT_FORECAST: &str = "agent_forecast";

/// Inputs for reproducing one persisted forecast-agent artifact bundle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentForecastReproductionRequest {
    pub original_run_dir: PathBuf,
    pub reproduction_root: PathBuf,
    pub ledger_row_path: PathBuf,
    pub expected_source_snapshot_refs: Vec<AgentSourceSnapshotRef>,
    pub expected_schema_version: String,
}

/// Byte comparison for one original/reproduced artifact pair.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileReproductionComparison {
    pub relative_path: String,
    pub original_bytes: u64,
    pub reproduced_bytes: u64,
    pub original_blake3: String,
    pub reproduced_blake3: String,
    pub identical: bool,
}

/// Durable proof that a forecast artifact bundle reproduces bit-for-bit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentForecastReproductionReport {
    pub schema_version: String,
    pub source_of_truth: String,
    pub original_run_dir: String,
    pub reproduced_run_dir: String,
    pub ledger_row_path: String,
    pub run_id: String,
    pub parser_schema_version: String,
    pub source_snapshot_count: usize,
    pub ledger_payload_blake3: String,
    pub ledger_payload_matches_original_manifest: bool,
    pub ledger_payload_matches_reproduced_manifest: bool,
    pub files: Vec<FileReproductionComparison>,
    pub bit_for_bit: bool,
}

/// Rebuild a forecast-agent artifact bundle from persisted local files and compare bytes.
pub fn reproduce_agent_forecast_artifacts(
    request: &AgentForecastReproductionRequest,
) -> Result<AgentForecastReproductionReport> {
    let manifest_path = request.original_run_dir.join("manifest.json");
    let manifest = read_json::<AgentForecastManifest>(
        &manifest_path,
        "POLY_AGENT_REPRO_MANIFEST_READ",
        "POLY_AGENT_REPRO_MANIFEST_DECODE",
    )?;
    validate_manifest_contract(request, &manifest)?;

    let ledger_payload = read_ledger_payload(&request.ledger_row_path)?;
    let original_payload = manifest.provenance_payload();
    if ledger_payload != original_payload {
        return Err(PolyError::agent_reproduction(
            "POLY_AGENT_REPRO_LEDGER_MISMATCH",
            "persisted ledger payload does not match original manifest provenance payload",
        ));
    }

    let rebuild_request = request_from_manifest(&request.original_run_dir, &manifest)?;
    verify_original_hashes(&request.original_run_dir, &manifest)?;
    let reproduction_run_dir = request.reproduction_root.join(&manifest.run_id);
    if reproduction_run_dir.exists() {
        return Err(PolyError::agent_reproduction(
            "POLY_AGENT_REPRO_OUTPUT_EXISTS",
            format!(
                "reproduction output already exists: {}",
                reproduction_run_dir.display()
            ),
        ));
    }

    let reproduced_manifest =
        write_agent_forecast_artifacts(&request.reproduction_root, &rebuild_request).map_err(
            |err| {
                PolyError::agent_reproduction(
                    "POLY_AGENT_REPRO_REBUILD_FAILED",
                    format!("rebuild artifact writer failed: {err}"),
                )
            },
        )?;
    let reproduced_payload = reproduced_manifest.provenance_payload();
    if reproduced_payload != ledger_payload {
        return Err(PolyError::agent_reproduction(
            "POLY_AGENT_REPRO_REPRODUCED_LEDGER_MISMATCH",
            "reproduced manifest provenance payload does not match persisted ledger payload",
        ));
    }

    let files = compare_artifacts(&request.original_run_dir, &reproduction_run_dir, &manifest)?;
    if files.iter().any(|file| !file.identical) {
        return Err(PolyError::agent_reproduction(
            "POLY_AGENT_REPRO_BIT_MISMATCH",
            "one or more reproduced artifact files differ from the original bytes",
        ));
    }

    Ok(AgentForecastReproductionReport {
        schema_version: AGENT_REPRODUCTION_SCHEMA_VERSION.to_string(),
        source_of_truth:
            "persisted local prompt, raw response, manifest, markdown, parsed forecast, and ledger row"
                .to_string(),
        original_run_dir: request.original_run_dir.display().to_string(),
        reproduced_run_dir: reproduction_run_dir.display().to_string(),
        ledger_row_path: request.ledger_row_path.display().to_string(),
        run_id: manifest.run_id,
        parser_schema_version: manifest.schema_version,
        source_snapshot_count: manifest.source_snapshot_refs.len(),
        ledger_payload_blake3: hash_json_value(&ledger_payload)?,
        ledger_payload_matches_original_manifest: true,
        ledger_payload_matches_reproduced_manifest: true,
        files,
        bit_for_bit: true,
    })
}

/// Writes the reproduction report as a durable source-of-truth artifact.
pub fn write_agent_reproduction_report(
    path: &Path,
    report: &AgentForecastReproductionReport,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            PolyError::agent_reproduction(
                "POLY_AGENT_REPRO_REPORT_WRITE",
                format!("create report directory {}: {err}", parent.display()),
            )
        })?;
    }
    let bytes = serde_json::to_vec_pretty(report).map_err(|err| {
        PolyError::agent_reproduction(
            "POLY_AGENT_REPRO_REPORT_ENCODE",
            format!("encode reproduction report: {err}"),
        )
    })?;
    fs::write(path, bytes).map_err(|err| {
        PolyError::agent_reproduction(
            "POLY_AGENT_REPRO_REPORT_WRITE",
            format!("write report {}: {err}", path.display()),
        )
    })
}

/// Reads a durable reproduction report.
pub fn read_agent_reproduction_report(path: &Path) -> Result<AgentForecastReproductionReport> {
    read_json(
        path,
        "POLY_AGENT_REPRO_REPORT_READ",
        "POLY_AGENT_REPRO_REPORT_DECODE",
    )
}

fn validate_manifest_contract(
    request: &AgentForecastReproductionRequest,
    manifest: &AgentForecastManifest,
) -> Result<()> {
    if manifest.schema_version != request.expected_schema_version {
        return Err(PolyError::agent_reproduction(
            "POLY_AGENT_REPRO_PARSER_VERSION_MISMATCH",
            format!(
                "expected parser schema {}, found {}",
                request.expected_schema_version, manifest.schema_version
            ),
        ));
    }
    if manifest.source_snapshot_refs != request.expected_source_snapshot_refs {
        return Err(PolyError::agent_reproduction(
            "POLY_AGENT_REPRO_SOURCE_SNAPSHOT_MISMATCH",
            "expected source snapshot refs differ from persisted manifest refs",
        ));
    }
    Ok(())
}

fn request_from_manifest(
    run_dir: &Path,
    manifest: &AgentForecastManifest,
) -> Result<AgentForecastArtifactRequest> {
    Ok(AgentForecastArtifactRequest {
        run_id: manifest.run_id.clone(),
        created_at: manifest.created_at.clone(),
        source_snapshot_refs: manifest.source_snapshot_refs.clone(),
        prompt_template_id: manifest.prompt.template_id.clone(),
        prompt_template_version: manifest.prompt.template_version.clone(),
        rendered_prompt: read_text(
            &artifact_path(run_dir, &manifest.prompt.rendered_prompt_path)?,
            "POLY_AGENT_REPRO_MISSING_PROMPT",
        )?,
        provider: manifest.provider.clone(),
        raw_response_json: read_text(
            &artifact_path(run_dir, &manifest.response.raw_response_path)?,
            "POLY_AGENT_REPRO_MISSING_RESPONSE",
        )?,
        markdown_prediction: read_text(
            &artifact_path(run_dir, &manifest.markdown_prediction_path)?,
            "POLY_AGENT_REPRO_MISSING_MARKDOWN",
        )?,
    })
}

fn verify_original_hashes(run_dir: &Path, manifest: &AgentForecastManifest) -> Result<()> {
    let checks = [
        (
            manifest.prompt.rendered_prompt_path.as_str(),
            manifest.prompt.rendered_prompt_blake3.as_str(),
        ),
        (
            manifest.response.raw_response_path.as_str(),
            manifest.response.raw_response_blake3.as_str(),
        ),
        (
            manifest.parsed_forecast_path.as_str(),
            manifest.parsed_forecast_blake3.as_str(),
        ),
        (
            manifest.parsed_forecast.rationale_path.as_str(),
            manifest.parsed_forecast.rationale_blake3.as_str(),
        ),
        (
            manifest.markdown_prediction_path.as_str(),
            manifest.markdown_prediction_blake3.as_str(),
        ),
    ];
    for (relative_path, expected_hash) in checks {
        let actual = hash_file(&artifact_path(run_dir, relative_path)?)?;
        if actual != expected_hash {
            return Err(PolyError::agent_reproduction(
                "POLY_AGENT_REPRO_ORIGINAL_HASH_MISMATCH",
                format!(
                    "original artifact {relative_path} hash {actual} did not match manifest {expected_hash}"
                ),
            ));
        }
    }
    Ok(())
}

fn compare_artifacts(
    original_run_dir: &Path,
    reproduction_run_dir: &Path,
    manifest: &AgentForecastManifest,
) -> Result<Vec<FileReproductionComparison>> {
    let relative_paths = [
        "manifest.json",
        manifest.prompt.rendered_prompt_path.as_str(),
        manifest.response.raw_response_path.as_str(),
        manifest.parsed_forecast_path.as_str(),
        manifest.parsed_forecast.rationale_path.as_str(),
        manifest.markdown_prediction_path.as_str(),
    ];
    relative_paths
        .into_iter()
        .map(|relative_path| compare_file(original_run_dir, reproduction_run_dir, relative_path))
        .collect()
}

fn compare_file(
    original_run_dir: &Path,
    reproduction_run_dir: &Path,
    relative_path: &str,
) -> Result<FileReproductionComparison> {
    let original_path = artifact_path(original_run_dir, relative_path)?;
    let reproduced_path = artifact_path(reproduction_run_dir, relative_path)?;
    let original = fs::read(&original_path).map_err(|err| {
        PolyError::agent_reproduction(
            "POLY_AGENT_REPRO_COMPARE_READ",
            format!("read original {}: {err}", original_path.display()),
        )
    })?;
    let reproduced = fs::read(&reproduced_path).map_err(|err| {
        PolyError::agent_reproduction(
            "POLY_AGENT_REPRO_COMPARE_READ",
            format!("read reproduced {}: {err}", reproduced_path.display()),
        )
    })?;
    let original_blake3 = blake3::hash(&original).to_hex().to_string();
    let reproduced_blake3 = blake3::hash(&reproduced).to_hex().to_string();
    Ok(FileReproductionComparison {
        relative_path: relative_path.to_string(),
        original_bytes: original.len() as u64,
        reproduced_bytes: reproduced.len() as u64,
        identical: original == reproduced,
        original_blake3,
        reproduced_blake3,
    })
}

fn read_ledger_payload(path: &Path) -> Result<Value> {
    let row = read_json::<Value>(
        path,
        "POLY_AGENT_REPRO_LEDGER_READ",
        "POLY_AGENT_REPRO_LEDGER_DECODE",
    )?;
    if row.get("kind").and_then(Value::as_str) != Some(LEDGER_KIND_AGENT_FORECAST) {
        return Err(PolyError::agent_reproduction(
            "POLY_AGENT_REPRO_LEDGER_KIND_MISMATCH",
            "ledger row kind must be agent_forecast",
        ));
    }
    row.get("payload").cloned().ok_or_else(|| {
        PolyError::agent_reproduction(
            "POLY_AGENT_REPRO_LEDGER_PAYLOAD_MISSING",
            "ledger row payload is required",
        )
    })
}

fn read_json<T: for<'de> Deserialize<'de>>(
    path: &Path,
    read_code: &'static str,
    decode_code: &'static str,
) -> Result<T> {
    let bytes = fs::read(path).map_err(|err| {
        PolyError::agent_reproduction(read_code, format!("read {}: {err}", path.display()))
    })?;
    serde_json::from_slice(&bytes).map_err(|err| {
        PolyError::agent_reproduction(decode_code, format!("decode {}: {err}", path.display()))
    })
}

fn read_text(path: &Path, code: &'static str) -> Result<String> {
    fs::read_to_string(path).map_err(|err| {
        PolyError::agent_reproduction(code, format!("read {}: {err}", path.display()))
    })
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|err| {
        PolyError::agent_reproduction(
            "POLY_AGENT_REPRO_HASH_READ",
            format!("read {}: {err}", path.display()),
        )
    })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn hash_json_value(value: &Value) -> Result<String> {
    let bytes = serde_json::to_vec(value).map_err(|err| {
        PolyError::agent_reproduction(
            "POLY_AGENT_REPRO_LEDGER_HASH",
            format!("encode ledger payload for hash: {err}"),
        )
    })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn artifact_path(root: &Path, relative_path: &str) -> Result<PathBuf> {
    let rel = Path::new(relative_path);
    if rel.is_absolute()
        || rel
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(PolyError::agent_reproduction(
            "POLY_AGENT_REPRO_UNSAFE_ARTIFACT_PATH",
            format!("artifact path must stay inside run directory: {relative_path}"),
        ));
    }
    Ok(root.join(rel))
}
