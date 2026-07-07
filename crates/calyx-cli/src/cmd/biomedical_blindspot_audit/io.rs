use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{CliError, CliResult};

use super::model::{AuditReport, ReadbackSummary, SourceManifest};

pub(super) fn persist(out_dir: &Path, report: &AuditReport) -> CliResult<ReadbackSummary> {
    fs::create_dir_all(out_dir)
        .map_err(|error| CliError::io(format!("create {}: {error}", out_dir.display())))?;
    let report_path = out_dir.join("biomedical_blindspot_audit_report.json");
    let audited_path = out_dir.join("audited_hypotheses.jsonl");
    let ready_path = out_dir.join("ready_hypotheses.jsonl");
    let blocked_path = out_dir.join("blocked_hypotheses.jsonl");
    let benchmark_path = out_dir.join("benchmark_export.jsonl");
    let metrics_path = out_dir.join("metrics.json");
    let report_bytes = serde_json::to_vec_pretty(report)
        .map_err(|error| CliError::runtime(format!("serialize blindspot report: {error}")))?;
    let audited_bytes = jsonl(&report.audited_hypotheses)?;
    let ready = report
        .audited_hypotheses
        .iter()
        .filter(|row| row.final_status == "ready_for_human_review_after_blindspot_audit")
        .cloned()
        .collect::<Vec<_>>();
    let blocked = report
        .audited_hypotheses
        .iter()
        .filter(|row| row.final_status != "ready_for_human_review_after_blindspot_audit")
        .cloned()
        .collect::<Vec<_>>();
    let ready_bytes = jsonl(&ready)?;
    let blocked_bytes = jsonl(&blocked)?;
    let benchmark_bytes = jsonl(&report.benchmark_export)?;
    let metrics_bytes = serde_json::to_vec_pretty(&report.metrics)
        .map_err(|error| CliError::runtime(format!("serialize blindspot metrics: {error}")))?;
    write_if_same(&report_path, &report_bytes)?;
    write_if_same(&audited_path, &audited_bytes)?;
    write_if_same(&ready_path, &ready_bytes)?;
    write_if_same(&blocked_path, &blocked_bytes)?;
    write_if_same(&benchmark_path, &benchmark_bytes)?;
    write_if_same(&metrics_path, &metrics_bytes)?;
    let report_readback = fs::read(&report_path)?;
    let decoded: AuditReport = serde_json::from_slice(&report_readback).map_err(|error| {
        CliError::runtime(format!(
            "parse blindspot report readback {}: {error}",
            report_path.display()
        ))
    })?;
    ReadbackSummary {
        report: report_path.display().to_string(),
        report_sha256: sha256_hex(&report_readback),
        audited_hypotheses: audited_path.display().to_string(),
        audited_hypotheses_rows: count_jsonl(&audited_path)?,
        audited_hypotheses_sha256: sha256_file(&audited_path)?,
        ready_hypotheses: ready_path.display().to_string(),
        ready_hypotheses_rows: count_jsonl(&ready_path)?,
        ready_hypotheses_sha256: sha256_file(&ready_path)?,
        blocked_hypotheses: blocked_path.display().to_string(),
        blocked_hypotheses_rows: count_jsonl(&blocked_path)?,
        blocked_hypotheses_sha256: sha256_file(&blocked_path)?,
        benchmark_export: benchmark_path.display().to_string(),
        benchmark_export_rows: count_jsonl(&benchmark_path)?,
        benchmark_export_sha256: sha256_file(&benchmark_path)?,
        metrics: metrics_path.display().to_string(),
        metrics_sha256: sha256_file(&metrics_path)?,
    }
    .verify(decoded)
}

impl ReadbackSummary {
    fn verify(self, report: AuditReport) -> CliResult<Self> {
        if self.audited_hypotheses_rows != report.audited_count
            || self.ready_hypotheses_rows != report.ready_count
            || self.blocked_hypotheses_rows != report.blocked_count + report.pending_count
            || self.benchmark_export_rows != report.benchmark_export.len()
        {
            return Err(CliError::runtime(
                "biomedical blindspot persisted readback counts did not match report counts",
            ));
        }
        Ok(self)
    }
}

pub(super) fn read_jsonl(label: &str, path: &Path) -> CliResult<(Vec<Value>, SourceManifest)> {
    let file = File::open(path)
        .map_err(|error| CliError::io(format!("read {label} {}: {error}", path.display())))?;
    let bytes = fs::read(path)
        .map_err(|error| CliError::io(format!("read {label} {}: {error}", path.display())))?;
    let mut rows = Vec::new();
    for (idx, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|error| {
            CliError::io(format!(
                "read {label} {} line {}: {error}",
                path.display(),
                idx + 1
            ))
        })?;
        if line.trim().is_empty() {
            continue;
        }
        rows.push(serde_json::from_str(&line).map_err(|error| {
            CliError::runtime(format!(
                "parse {label} {} line {}: {error}",
                path.display(),
                idx + 1
            ))
        })?);
    }
    Ok((
        rows.clone(),
        SourceManifest {
            label: label.to_string(),
            path: path.display().to_string(),
            bytes: bytes.len() as u64,
            rows: Some(rows.len()),
            sha256: sha256_hex(&bytes),
        },
    ))
}

fn jsonl<T: Serialize>(rows: &[T]) -> CliResult<Vec<u8>> {
    let mut out = Vec::new();
    for row in rows {
        serde_json::to_writer(&mut out, row).map_err(|error| {
            CliError::runtime(format!("serialize blindspot JSONL row: {error}"))
        })?;
        out.push(b'\n');
    }
    Ok(out)
}

fn write_if_same(path: &Path, bytes: &[u8]) -> CliResult {
    if path.exists() {
        if fs::read(path)? != bytes {
            return Err(CliError::runtime(format!(
                "refusing to overwrite existing different blindspot artifact {}",
                path.display()
            )));
        }
        return Ok(());
    }
    fs::write(path, bytes)
        .map_err(|error| CliError::io(format!("write {}: {error}", path.display())))?;
    if fs::read(path)? != bytes {
        return Err(CliError::runtime(format!(
            "blindspot artifact readback mismatch at {}",
            path.display()
        )));
    }
    Ok(())
}

fn count_jsonl(path: &Path) -> CliResult<usize> {
    let file = File::open(path)
        .map_err(|error| CliError::io(format!("read {}: {error}", path.display())))?;
    Ok(BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|line| !line.trim().is_empty())
        .count())
}

fn sha256_file(path: &Path) -> CliResult<String> {
    fs::read(path)
        .map(|bytes| sha256_hex(&bytes))
        .map_err(|error| CliError::io(format!("read {}: {error}", path.display())))
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
