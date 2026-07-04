use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_lodestar::{
    HypothesisEvaluationInput, HypothesisEvaluationReport, TraceableHypothesisInput,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::super::artifact_hash::sha256_hex;
use super::super::discovery_run_preflight::{
    DiscoveryRunPreflightArgs, DiscoveryRunPreflightReadback,
};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

pub(super) const BRIDGE_SCHEMA_VERSION: u32 = 1;
pub(super) const CLINICAL_BOUNDARY: &str =
    "Research lead only; not clinical actionability, efficacy, safety, dosing, or cure evidence.";

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FalsificationEvaluateBridgeArgs {
    pub miner_report: PathBuf,
    pub falsification_report: PathBuf,
    pub out: PathBuf,
    pub preflight: DiscoveryRunPreflightArgs,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EvaluateRankBridgeArgs {
    pub evaluation_report: PathBuf,
    pub out: PathBuf,
    pub preflight: DiscoveryRunPreflightArgs,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct MinerReport {
    pub hypotheses: Vec<AssociationHypothesis>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct AssociationHypothesis {
    pub hypothesis_id: String,
    pub source_id: String,
    pub source_name: String,
    pub source_type: String,
    pub target_id: String,
    pub target_name: String,
    pub target_type: String,
    pub path_count: usize,
    pub support_count: usize,
    pub score: f64,
    pub novelty_score: f64,
    pub clinical_boundary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct FalsificationReport {
    pub hypothesis_flags: Vec<HypothesisFlag>,
    pub support_evidence: Vec<EvidenceRow>,
    pub counter_evidence: Vec<EvidenceRow>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct HypothesisFlag {
    pub hypothesis_id: String,
    pub support_evidence_count: usize,
    pub counter_evidence_count: usize,
    pub support_weight: f64,
    pub counter_weight: f64,
    pub falsification_score: f64,
    pub reason_codes: Vec<String>,
    pub sweep_status: String,
    pub human_review_atlas_status: String,
    pub clinical_boundary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct EvidenceRow {
    pub hypothesis_id: String,
    pub evidence_kind: String,
    pub source_system: String,
    pub reason_code: String,
    pub source_path: String,
    pub source_sha256: String,
    pub source_row_index: usize,
    pub weight: f64,
    pub summary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct EvaluationArtifact {
    pub report: HypothesisEvaluationReport,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct EvaluationBridgeOutput {
    pub schema_version: u32,
    pub bridge_metadata: BridgeMetadata,
    pub inputs: Vec<HypothesisEvaluationInput>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RankBridgeOutput {
    pub schema_version: u32,
    pub bridge_metadata: BridgeMetadata,
    pub inputs: Vec<TraceableHypothesisInput>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct BridgeMetadata {
    pub bridge_kind: String,
    pub source_artifacts: Vec<SourceArtifact>,
    pub counts: BTreeMap<String, usize>,
    pub research_lead_only: bool,
    pub clinical_boundary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct SourceArtifact {
    pub role: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct BridgeReadback {
    pub schema_version: u32,
    pub bridge_kind: String,
    pub out: String,
    pub out_bytes: u64,
    pub out_sha256: String,
    pub readback_input_count: usize,
    pub counts: BTreeMap<String, usize>,
    pub source_artifacts: Vec<SourceArtifact>,
    pub preflight: Option<DiscoveryRunPreflightReadback>,
    pub research_lead_only: bool,
    pub clinical_boundary: String,
}

pub(super) struct Source<T> {
    pub value: T,
    pub bytes: Vec<u8>,
    pub artifact: SourceArtifact,
}

pub(super) fn read_source<T: for<'de> Deserialize<'de>>(
    role: &str,
    path: &Path,
) -> CliResult<Source<T>> {
    let bytes = fs::read(path)
        .map_err(|error| CliError::io(format!("read {role} {}: {error}", path.display())))?;
    let value = serde_json::from_slice(&bytes)
        .map_err(|error| CliError::runtime(format!("parse {role} {}: {error}", path.display())))?;
    Ok(Source {
        value,
        artifact: SourceArtifact {
            role: role.to_string(),
            path: path.display().to_string(),
            bytes: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        },
        bytes,
    })
}

pub(super) fn metadata(
    bridge_kind: &str,
    source_artifacts: Vec<SourceArtifact>,
    counts: BTreeMap<String, usize>,
) -> BridgeMetadata {
    BridgeMetadata {
        bridge_kind: bridge_kind.to_string(),
        source_artifacts,
        counts,
        research_lead_only: true,
        clinical_boundary: CLINICAL_BOUNDARY.to_string(),
    }
}

pub(super) fn persist_bridge_output<T: Serialize>(
    out: &Path,
    output: &T,
    metadata: &BridgeMetadata,
    preflight: Option<DiscoveryRunPreflightReadback>,
    readback_count: fn(&[u8]) -> CliResult<usize>,
) -> CliResult<BridgeReadback> {
    let bytes = serde_json::to_vec_pretty(output).map_err(|error| {
        CliError::runtime(format!("serialize discovery bridge output: {error}"))
    })?;
    write_if_same(out, &bytes)?;
    let readback_bytes = fs::read(out)?;
    if readback_bytes != bytes {
        return Err(CliError::runtime(format!(
            "discovery bridge output readback mismatch at {}",
            out.display()
        )));
    }
    let readback = BridgeReadback {
        schema_version: BRIDGE_SCHEMA_VERSION,
        bridge_kind: metadata.bridge_kind.clone(),
        out: out.display().to_string(),
        out_bytes: readback_bytes.len() as u64,
        out_sha256: sha256_hex(&readback_bytes),
        readback_input_count: readback_count(&readback_bytes)?,
        counts: metadata.counts.clone(),
        source_artifacts: metadata.source_artifacts.clone(),
        preflight,
        research_lead_only: true,
        clinical_boundary: CLINICAL_BOUNDARY.to_string(),
    };
    let sidecar = serde_json::to_vec_pretty(&readback)
        .map_err(|error| CliError::runtime(format!("serialize bridge readback: {error}")))?;
    write_if_same(&readback_path(out), &sidecar)?;
    Ok(readback)
}

pub(super) fn eval_count(bytes: &[u8]) -> CliResult<usize> {
    serde_json::from_slice::<EvaluationBridgeOutput>(bytes)
        .map(|output| output.inputs.len())
        .map_err(|error| CliError::runtime(format!("parse evaluation bridge readback: {error}")))
}

pub(super) fn rank_count(bytes: &[u8]) -> CliResult<usize> {
    serde_json::from_slice::<RankBridgeOutput>(bytes)
        .map(|output| output.inputs.len())
        .map_err(|error| CliError::runtime(format!("parse rank bridge readback: {error}")))
}

pub(super) fn print_bridge_summary(readback: &BridgeReadback) -> CliResult {
    print_json(&json!({
        "status": "ok",
        "bridge_kind": readback.bridge_kind,
        "out": readback.out,
        "out_bytes": readback.out_bytes,
        "out_sha256": readback.out_sha256,
        "readback": readback_path(Path::new(&readback.out)),
        "readback_input_count": readback.readback_input_count,
        "counts": readback.counts,
        "preflight": readback.preflight,
        "research_lead_only": true,
        "clinical_boundary": CLINICAL_BOUNDARY,
    }))
}

pub(super) fn require_path(path: &Path, command: &str, flag: &str) -> CliResult {
    if path.as_os_str().is_empty() {
        return Err(CliError::usage(format!("{command} requires {flag} <path>")));
    }
    Ok(())
}

pub(super) fn bridge_error(detail: impl Into<String>) -> CliError {
    CliError::runtime(format!("discovery bridge invalid: {}", detail.into()))
}

fn write_if_same(path: &Path, bytes: &[u8]) -> CliResult {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        if fs::read(path)? != bytes {
            return Err(CliError::runtime(format!(
                "refusing to overwrite existing different discovery bridge artifact {}",
                path.display()
            )));
        }
        return Ok(());
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn readback_path(out: &Path) -> PathBuf {
    out.with_extension("readback.json")
}
