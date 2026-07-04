use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::model::FalsificationReport;
use crate::error::{CliError, CliResult};

pub(super) struct Persisted {
    pub report: PathBuf,
    pub report_sha256: String,
    pub support_evidence: PathBuf,
    pub support_evidence_sha256: String,
    pub counter_evidence: PathBuf,
    pub counter_evidence_sha256: String,
    pub skipped_evidence: PathBuf,
    pub skipped_evidence_sha256: String,
    pub hypothesis_flags: PathBuf,
    pub hypothesis_flags_sha256: String,
    pub raw_query_manifest: PathBuf,
    pub raw_query_manifest_sha256: String,
    pub flag_count: usize,
}

pub(super) fn persist(out_dir: &Path, report: &FalsificationReport) -> CliResult<Persisted> {
    fs::create_dir_all(out_dir)?;
    let report_path = out_dir.join("falsification_sweep_report.json");
    let support_path = out_dir.join("support_evidence.jsonl");
    let counter_path = out_dir.join("counter_evidence.jsonl");
    let skipped_path = out_dir.join("skipped_evidence.jsonl");
    let flags_path = out_dir.join("hypothesis_flags.jsonl");
    let raw_manifest_path = out_dir.join("raw_query_manifest.jsonl");
    let report_bytes = serde_json::to_vec_pretty(report)
        .map_err(|error| CliError::runtime(format!("serialize falsification report: {error}")))?;
    write_if_same(&report_path, &report_bytes)?;
    write_if_same(&support_path, &jsonl(&report.support_evidence)?)?;
    write_if_same(&counter_path, &jsonl(&report.counter_evidence)?)?;
    write_if_same(&skipped_path, &jsonl(&report.skipped_evidence)?)?;
    write_if_same(&flags_path, &jsonl(&report.hypothesis_flags)?)?;
    write_if_same(&raw_manifest_path, &jsonl(&report.raw_query_manifest)?)?;
    let report_readback = fs::read(&report_path)?;
    let decoded: FalsificationReport =
        serde_json::from_slice(&report_readback).map_err(|error| {
            CliError::runtime(format!("parse falsification report readback: {error}"))
        })?;
    Ok(Persisted {
        report: report_path,
        report_sha256: sha256_hex(&report_readback),
        support_evidence: support_path.clone(),
        support_evidence_sha256: sha256_hex(&fs::read(&support_path)?),
        counter_evidence: counter_path.clone(),
        counter_evidence_sha256: sha256_hex(&fs::read(&counter_path)?),
        skipped_evidence: skipped_path.clone(),
        skipped_evidence_sha256: sha256_hex(&fs::read(&skipped_path)?),
        hypothesis_flags: flags_path.clone(),
        hypothesis_flags_sha256: sha256_hex(&fs::read(&flags_path)?),
        raw_query_manifest: raw_manifest_path.clone(),
        raw_query_manifest_sha256: sha256_hex(&fs::read(&raw_manifest_path)?),
        flag_count: decoded.hypothesis_flags.len(),
    })
}

fn jsonl<T: Serialize>(rows: &[T]) -> CliResult<Vec<u8>> {
    let mut out = Vec::new();
    for row in rows {
        serde_json::to_writer(&mut out, row)
            .map_err(|error| CliError::runtime(format!("serialize falsification row: {error}")))?;
        out.push(b'\n');
    }
    Ok(out)
}

fn write_if_same(path: &Path, bytes: &[u8]) -> CliResult {
    if path.exists() {
        if fs::read(path)? != bytes {
            return Err(CliError::runtime(format!(
                "refusing to overwrite existing different falsification artifact {}",
                path.display()
            )));
        }
        return Ok(());
    }
    fs::write(path, bytes)?;
    if fs::read(path)? != bytes {
        return Err(CliError::runtime(format!(
            "falsification artifact readback mismatch at {}",
            path.display()
        )));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
