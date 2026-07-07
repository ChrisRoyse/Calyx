use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::cmd::mechanistic_direction::MechanisticDirectionEvidence;
use crate::error::{CliError, CliResult};

pub(crate) const REPORT_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct GateParams {
    pub cutoff_year: i32,
    pub score_threshold: f64,
    pub min_auroc: f64,
    pub min_positive_recall: f64,
    pub min_negative_suppression: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SourceManifest {
    pub family: String,
    pub root: String,
    pub file_count: usize,
    pub byte_count: u64,
    pub aggregate_sha256: String,
    pub required_files_present: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct BenchmarkSourceRow {
    pub row_id: String,
    pub benchmark_kind: String,
    pub source_family: String,
    pub source_path: String,
    pub source_line: Option<usize>,
    pub source_row_sha256: String,
    pub seed_id: String,
    pub subject: String,
    pub object: String,
    pub label: Option<bool>,
    pub source_year: Option<i32>,
    pub features: Value,
    pub mechanistic_direction: Option<MechanisticDirectionEvidence>,
    pub raw: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MechanisticDirectionBlockedRow {
    pub source_system: String,
    pub source_path: String,
    pub source_line: Option<usize>,
    pub source_row_sha256: String,
    pub target_name: String,
    pub disease_name: String,
    pub reason_codes: Vec<String>,
    pub mechanistic_direction: MechanisticDirectionEvidence,
    pub raw: Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct MechanisticDirectionCounts {
    pub inferred_required_direction_rows: usize,
    pub blocked_direction_rows: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TimeSplitRow {
    pub seed_id: String,
    pub split: String,
    pub cutoff_year: i32,
    pub early_evidence_count: usize,
    pub early_max_score: f64,
    pub later_evidence_count: usize,
    pub later_positive: bool,
    pub source_row_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ScoredOutput {
    pub row_id: String,
    pub benchmark_kind: String,
    pub seed_id: String,
    pub label: bool,
    pub score: f64,
    pub score_basis: Vec<String>,
    pub source_row_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MetricBlock {
    pub n: usize,
    pub positives: usize,
    pub negatives: usize,
    pub threshold: f64,
    pub tp: usize,
    pub fp: usize,
    pub tn: usize,
    pub fn_: usize,
    pub precision: f64,
    pub precision_ci: [f64; 2],
    pub positive_recall: f64,
    pub positive_recall_ci: [f64; 2],
    pub negative_suppression: f64,
    pub negative_suppression_ci: [f64; 2],
    pub auroc: f64,
    pub auroc_ci: [f64; 2],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ValidationMetrics {
    pub known_positive_negative: MetricBlock,
    pub time_split: MetricBlock,
    pub combined: MetricBlock,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct BenchmarkCounts {
    pub known_positive: usize,
    pub known_negative: usize,
    pub time_split: usize,
    pub source_rows: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct GateDecision {
    pub passed: bool,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct AssociationValidationReport {
    pub schema_version: u32,
    pub status: String,
    pub gate_passed: bool,
    pub params: GateParams,
    pub source_manifests: Vec<SourceManifest>,
    pub benchmark_counts: BenchmarkCounts,
    pub mechanistic_direction_counts: MechanisticDirectionCounts,
    pub benchmark_source_rows: Vec<BenchmarkSourceRow>,
    pub mechanistic_direction_blocked_rows: Vec<MechanisticDirectionBlockedRow>,
    pub train_test_split: Vec<TimeSplitRow>,
    pub scored_outputs: Vec<ScoredOutput>,
    pub metrics: ValidationMetrics,
    pub gate_decision: GateDecision,
    pub clinical_boundary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ReadbackSummary {
    pub report: PathBuf,
    pub report_bytes: u64,
    pub report_sha256: String,
    pub benchmark_source_rows_jsonl: PathBuf,
    pub benchmark_source_rows: usize,
    pub benchmark_source_rows_sha256: String,
    pub mechanistic_direction_blocked_jsonl: PathBuf,
    pub mechanistic_direction_blocked_rows: usize,
    pub mechanistic_direction_blocked_sha256: String,
    pub train_test_split_jsonl: PathBuf,
    pub train_test_split_rows: usize,
    pub train_test_split_sha256: String,
    pub scored_outputs_jsonl: PathBuf,
    pub scored_output_rows: usize,
    pub scored_outputs_sha256: String,
    pub metrics_json: PathBuf,
    pub metrics_sha256: String,
    pub readback_gate_passed: bool,
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

pub(crate) fn persist_report_set(
    out_dir: &Path,
    report: &AssociationValidationReport,
) -> CliResult<ReadbackSummary> {
    fs::create_dir_all(out_dir)?;
    let source_rows_path = out_dir.join("benchmark_source_rows.jsonl");
    let blocked_path = out_dir.join("mechanistic_direction_blocked_rows.jsonl");
    let split_path = out_dir.join("train_test_split.jsonl");
    let scored_path = out_dir.join("scored_outputs.jsonl");
    let metrics_path = out_dir.join("metrics.json");
    let report_path = out_dir.join("association_validation_report.json");

    let source_rows = jsonl_bytes(&report.benchmark_source_rows)?;
    let blocked_rows = jsonl_bytes(&report.mechanistic_direction_blocked_rows)?;
    let split_rows = jsonl_bytes(&report.train_test_split)?;
    let scored_rows = jsonl_bytes(&report.scored_outputs)?;
    let metrics = serde_json::to_vec_pretty(&json!({
        "schema_version": REPORT_SCHEMA_VERSION,
        "gate_passed": report.gate_passed,
        "params": report.params,
        "benchmark_counts": report.benchmark_counts,
        "mechanistic_direction_counts": report.mechanistic_direction_counts,
        "metrics": report.metrics,
        "gate_decision": report.gate_decision,
    }))
    .map_err(|error| CliError::runtime(format!("serialize metrics: {error}")))?;
    let report_bytes = serde_json::to_vec_pretty(report)
        .map_err(|error| CliError::runtime(format!("serialize report: {error}")))?;

    write_if_same(&source_rows_path, &source_rows)?;
    write_if_same(&blocked_path, &blocked_rows)?;
    write_if_same(&split_path, &split_rows)?;
    write_if_same(&scored_path, &scored_rows)?;
    write_if_same(&metrics_path, &metrics)?;
    write_if_same(&report_path, &report_bytes)?;

    let report_readback = fs::read(&report_path)?;
    let decoded: AssociationValidationReport =
        serde_json::from_slice(&report_readback).map_err(|error| {
            CliError::runtime(format!(
                "parse association validation report readback {}: {error}",
                report_path.display()
            ))
        })?;
    let source_readback = fs::read_to_string(&source_rows_path)?;
    let blocked_readback = fs::read_to_string(&blocked_path)?;
    let split_readback = fs::read_to_string(&split_path)?;
    let scored_readback = fs::read_to_string(&scored_path)?;

    Ok(ReadbackSummary {
        report: report_path,
        report_bytes: report_readback.len() as u64,
        report_sha256: sha256_hex(&report_readback),
        benchmark_source_rows_jsonl: source_rows_path,
        benchmark_source_rows: count_jsonl(&source_readback),
        benchmark_source_rows_sha256: sha256_hex(source_readback.as_bytes()),
        mechanistic_direction_blocked_jsonl: blocked_path,
        mechanistic_direction_blocked_rows: count_jsonl(&blocked_readback),
        mechanistic_direction_blocked_sha256: sha256_hex(blocked_readback.as_bytes()),
        train_test_split_jsonl: split_path,
        train_test_split_rows: count_jsonl(&split_readback),
        train_test_split_sha256: sha256_hex(split_readback.as_bytes()),
        scored_outputs_jsonl: scored_path,
        scored_output_rows: count_jsonl(&scored_readback),
        scored_outputs_sha256: sha256_hex(scored_readback.as_bytes()),
        metrics_json: metrics_path.clone(),
        metrics_sha256: sha256_hex(&fs::read(&metrics_path)?),
        readback_gate_passed: decoded.gate_passed,
    })
}

fn jsonl_bytes<T: Serialize>(rows: &[T]) -> CliResult<Vec<u8>> {
    let mut out = Vec::new();
    for row in rows {
        serde_json::to_writer(&mut out, row)
            .map_err(|error| CliError::runtime(format!("serialize jsonl row: {error}")))?;
        out.push(b'\n');
    }
    Ok(out)
}

fn write_if_same(path: &Path, bytes: &[u8]) -> CliResult {
    if path.exists() {
        let existing = fs::read(path)?;
        if existing != bytes {
            return Err(CliError::runtime(format!(
                "refusing to overwrite existing different validation artifact {}",
                path.display()
            )));
        }
        return Ok(());
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    let readback = fs::read(path)?;
    if readback != bytes {
        return Err(CliError::runtime(format!(
            "validation artifact readback mismatch at {}",
            path.display()
        )));
    }
    Ok(())
}

fn count_jsonl(text: &str) -> usize {
    text.lines().filter(|line| !line.trim().is_empty()).count()
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
