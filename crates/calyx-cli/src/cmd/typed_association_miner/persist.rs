use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::json;
use sha2::Digest;

use super::model::MinerReport;
use super::{SCHEMA_VERSION, hex_lower};
use crate::error::{CliError, CliResult};

pub(super) struct Persisted {
    pub report: PathBuf,
    pub report_sha256: String,
    pub hypotheses: PathBuf,
    pub hypotheses_sha256: String,
    pub blocked_candidates: PathBuf,
    pub blocked_candidates_sha256: String,
    pub blocked_candidate_count: usize,
    pub summary: PathBuf,
    pub summary_sha256: String,
    pub hypothesis_count: usize,
}

pub(super) fn persist(out_dir: &Path, report: &MinerReport) -> CliResult<Persisted> {
    fs::create_dir_all(out_dir)?;
    let report_path = out_dir.join("typed_association_miner_report.json");
    let hypotheses_path = out_dir.join("hypotheses.jsonl");
    let blocked_path = out_dir.join("blocked_candidates.jsonl");
    let summary_path = out_dir.join("score_summary.json");
    let report_bytes = serde_json::to_vec_pretty(report)
        .map_err(|error| CliError::runtime(format!("serialize miner report: {error}")))?;
    let hypotheses_bytes = jsonl(&report.hypotheses)?;
    let blocked_bytes = jsonl(&report.blocked_candidates)?;
    let summary_bytes = serde_json::to_vec_pretty(&json!({
        "schema_version": SCHEMA_VERSION,
        "validation_gate_passed": report.validation_gate_passed,
        "validation_report_sha256": report.validation_report_sha256,
        "input_node_count": report.input_node_count,
        "input_edge_count": report.input_edge_count,
        "scan_limit_reached": report.scan_limit_reached,
        "candidate_pair_count": report.candidate_pair_count,
        "blocked_candidate_count": report.blocked_candidate_count,
        "emitted_hypothesis_count": report.emitted_hypothesis_count,
        "top_score": report.hypotheses.first().map(|row| row.score),
    }))
    .map_err(|error| CliError::runtime(format!("serialize score summary: {error}")))?;
    write_if_same(&report_path, &report_bytes)?;
    write_if_same(&hypotheses_path, &hypotheses_bytes)?;
    write_if_same(&blocked_path, &blocked_bytes)?;
    write_if_same(&summary_path, &summary_bytes)?;
    let report_readback = fs::read(&report_path)?;
    let decoded: MinerReport = serde_json::from_slice(&report_readback)
        .map_err(|error| CliError::runtime(format!("parse miner report readback: {error}")))?;
    Ok(Persisted {
        report: report_path,
        report_sha256: sha256_hex(&report_readback),
        hypotheses: hypotheses_path.clone(),
        hypotheses_sha256: sha256_hex(&fs::read(&hypotheses_path)?),
        blocked_candidates: blocked_path.clone(),
        blocked_candidates_sha256: sha256_hex(&fs::read(&blocked_path)?),
        blocked_candidate_count: decoded.blocked_candidates.len(),
        summary: summary_path.clone(),
        summary_sha256: sha256_hex(&fs::read(&summary_path)?),
        hypothesis_count: decoded.hypotheses.len(),
    })
}

fn jsonl<T: Serialize>(rows: &[T]) -> CliResult<Vec<u8>> {
    let mut out = Vec::new();
    for row in rows {
        serde_json::to_writer(&mut out, row)
            .map_err(|error| CliError::runtime(format!("serialize hypothesis row: {error}")))?;
        out.push(b'\n');
    }
    Ok(out)
}

fn write_if_same(path: &Path, bytes: &[u8]) -> CliResult {
    if path.exists() {
        if fs::read(path)? != bytes {
            return Err(CliError::runtime(format!(
                "refusing to overwrite existing different miner artifact {}",
                path.display()
            )));
        }
        return Ok(());
    }
    fs::write(path, bytes)?;
    if fs::read(path)? != bytes {
        return Err(CliError::runtime(format!(
            "miner artifact readback mismatch at {}",
            path.display()
        )));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&sha2::Sha256::digest(bytes))
}
