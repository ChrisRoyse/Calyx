use std::fs;

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::CODE_READBACK_MISMATCH;
use super::model::MultiAnchorReport;
use super::request::Request;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Evidence {
    pub(crate) out_dir: String,
    pub(crate) report_path: String,
    pub(crate) lens_values_path: String,
    pub(crate) target_summary_path: String,
    pub(crate) report_sha256: String,
    pub(crate) readback_sha256: String,
    pub(crate) readback_matches: bool,
    pub(crate) status: String,
    pub(crate) gate_passed: bool,
    pub(crate) report_count: usize,
    pub(crate) lens_count: usize,
    pub(crate) passing_lens_count: usize,
    pub(crate) weakest_lens: String,
    pub(crate) min_best_marginal_bits: f32,
    pub(crate) max_best_marginal_bits: f32,
}

pub(crate) fn write_outputs(
    request: &Request,
    report: &MultiAnchorReport,
) -> Result<Evidence, String> {
    request.ensure_fresh_output()?;
    fs::create_dir_all(&request.out_dir).map_err(|error| error.to_string())?;
    let report_path = request.out_dir.join("multi_anchor_ensemble_card.json");
    let report_bytes = serde_json::to_vec_pretty(report).map_err(|error| error.to_string())?;
    fs::write(&report_path, &report_bytes).map_err(|error| error.to_string())?;

    let lens_values_path = request.out_dir.join("multi_anchor_lens_values.txt");
    fs::write(&lens_values_path, lens_values(report)).map_err(|error| error.to_string())?;
    let target_summary_path = request.out_dir.join("multi_anchor_target_summary.txt");
    fs::write(&target_summary_path, target_values(report)).map_err(|error| error.to_string())?;

    let readback = fs::read(&report_path).map_err(|error| error.to_string())?;
    let report_sha256 = sha256_hex(&report_bytes);
    let readback_sha256 = sha256_hex(&readback);
    if report_sha256 != readback_sha256 {
        return Err(format!(
            "{CODE_READBACK_MISMATCH}: wrote {report_sha256} but read back {readback_sha256}"
        ));
    }
    Ok(Evidence {
        out_dir: request.out_dir.display().to_string(),
        report_path: report_path.display().to_string(),
        lens_values_path: lens_values_path.display().to_string(),
        target_summary_path: target_summary_path.display().to_string(),
        report_sha256,
        readback_sha256,
        readback_matches: true,
        status: report.status.clone(),
        gate_passed: report.gate_passed,
        report_count: report.report_count,
        lens_count: report.lens_count,
        passing_lens_count: report.passing_lens_count,
        weakest_lens: report.weakest_lens.clone(),
        min_best_marginal_bits: report.min_best_marginal_bits,
        max_best_marginal_bits: report.max_best_marginal_bits,
    })
}

fn lens_values(report: &MultiAnchorReport) -> String {
    let mut out = String::new();
    for lens in &report.lenses {
        out.push_str(&format!(
            "lens={} slot={} family={} passed={} best_target={} best_marginal={:.6}\n",
            lens.name,
            lens.slot,
            lens.association_family,
            lens.passed,
            lens.best_target_class,
            lens.best_marginal_bits
        ));
    }
    out
}

fn target_values(report: &MultiAnchorReport) -> String {
    let mut out = String::new();
    for target in &report.target_summaries {
        out.push_str(&format!(
            "target={} domain={} status={} no_collapse={} redundancy={} n_eff={:.6} max_marginal={:.6}\n",
            target.target_class,
            target.domain,
            target.status,
            target.no_collapse_pass,
            target.redundancy_bound_pass,
            target.n_eff,
            target.max_marginal_bits
        ));
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
