use std::fs;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use serde::Serialize;

use super::engine::RecallReport;
use super::request::RecallRequest;
use crate::error::{CliError, CliResult};

const METRIC_KEY: &[u8] = b"ph70/sextant/recall/scifact";

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MetricEvidence {
    pub(crate) single_recall_path: String,
    pub(crate) multi_recall_path: String,
    pub(crate) delta_path: String,
    pub(crate) summary_path: String,
    pub(crate) metric_cf: String,
    pub(crate) metric_cf_key_hex: String,
    pub(crate) metric_cf_value_bytes: usize,
    pub(crate) metric_cf_seq: u64,
    pub(crate) report: RecallReport,
}

pub(crate) fn write_metric_outputs(
    vault: &AsterVault,
    request: &RecallRequest,
    report: RecallReport,
) -> CliResult<MetricEvidence> {
    fs::create_dir_all(&request.metrics_dir)?;
    let single = request.metrics_dir.join("sextant_single_recall.txt");
    let multi = request.metrics_dir.join("sextant_multi_recall.txt");
    let delta = request.metrics_dir.join("sextant_recall_delta.txt");
    let summary = request.metrics_dir.join("sextant_recall_summary.json");
    write_float(&single, report.single_recall_at_10)?;
    write_float(&multi, report.multi_recall_at_10)?;
    write_float(&delta, report.delta)?;
    let value = serde_json::to_vec_pretty(&report)
        .map_err(|error| CliError::runtime(format!("serialize recall summary: {error}")))?;
    fs::write(&summary, &value)?;
    let seq = vault.write_cf(ColumnFamily::Online, METRIC_KEY.to_vec(), value.clone())?;
    vault.flush()?;
    Ok(MetricEvidence {
        single_recall_path: single.display().to_string(),
        multi_recall_path: multi.display().to_string(),
        delta_path: delta.display().to_string(),
        summary_path: summary.display().to_string(),
        metric_cf: "online".to_string(),
        metric_cf_key_hex: hex(METRIC_KEY),
        metric_cf_value_bytes: value.len(),
        metric_cf_seq: seq,
        report,
    })
}

fn write_float(path: &std::path::Path, value: f64) -> CliResult {
    if !value.is_finite() {
        return Err(CliError::runtime(format!(
            "CALYX_FSV_SEXTANT_NONFINITE_METRIC: {}",
            path.display()
        )));
    }
    Ok(fs::write(path, format!("{value:.6}\n"))?)
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
}
