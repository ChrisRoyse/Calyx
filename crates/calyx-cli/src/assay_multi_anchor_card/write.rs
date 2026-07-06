use std::fs;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::a37_admission_store::{self, A37AdmissionDbReadback};

use super::CODE_READBACK_MISMATCH;
use super::model::MultiAnchorReport;
use super::request::Request;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Evidence {
    pub(crate) artifact_mode: String,
    pub(crate) out_dir: String,
    pub(crate) cf_root: String,
    pub(crate) association_key: String,
    pub(crate) db_readback: A37AdmissionDbReadback,
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
    let db_readback =
        a37_admission_store::write(&request.cf_root, &request.association_key, report)
            .map_err(|error| format!("{}: {}", error.code, error.message))?;

    let mut report_path = String::new();
    let mut lens_values_path = String::new();
    let mut target_summary_path = String::new();
    let mut report_sha256 = String::new();
    let mut readback_sha256 = String::new();
    let mut readback_matches = db_readback.readback_matches;
    if request.emit_artifacts {
        fs::create_dir_all(&request.out_dir)
            .map_err(|error| format!("create {}: {error}", request.out_dir.display()))?;
        let path = request.out_dir.join("multi_anchor_ensemble_card.json");
        let report_bytes = serde_json::to_vec_pretty(report)
            .map_err(|error| format!("serialize multi-anchor report: {error}"))?;
        fs::write(&path, &report_bytes)
            .map_err(|error| format!("write {}: {error}", path.display()))?;

        let lens_path = request.out_dir.join("multi_anchor_lens_values.txt");
        fs::write(&lens_path, lens_values(report))
            .map_err(|error| format!("write {}: {error}", lens_path.display()))?;
        let target_path = request.out_dir.join("multi_anchor_target_summary.txt");
        fs::write(&target_path, target_values(report))
            .map_err(|error| format!("write {}: {error}", target_path.display()))?;

        let readback =
            fs::read(&path).map_err(|error| format!("read back {}: {error}", path.display()))?;
        report_sha256 = sha256_hex(&report_bytes);
        readback_sha256 = sha256_hex(&readback);
        if report_sha256 != readback_sha256 {
            return Err(format!(
                "{CODE_READBACK_MISMATCH}: wrote {report_sha256} but read back {readback_sha256}"
            ));
        }
        report_path = path.display().to_string();
        lens_values_path = lens_path.display().to_string();
        target_summary_path = target_path.display().to_string();
        readback_matches = true;
    }
    Ok(Evidence {
        artifact_mode: if request.emit_artifacts {
            "diagnostic_files".to_string()
        } else {
            "db_only".to_string()
        },
        out_dir: request.out_dir.display().to_string(),
        cf_root: request.cf_root.display().to_string(),
        association_key: request.association_key.clone(),
        db_readback,
        report_path,
        lens_values_path,
        target_summary_path,
        report_sha256,
        readback_sha256,
        readback_matches,
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

pub(crate) fn format_evidence(evidence: &Evidence) -> String {
    let mut out = String::new();
    out.push_str("a37_multi_anchor_evidence\n");
    out.push_str(&format!("artifact_mode={}\n", evidence.artifact_mode));
    out.push_str(&format!("out_dir={}\n", evidence.out_dir));
    out.push_str(&format!("cf_root={}\n", evidence.cf_root));
    out.push_str(&format!("association_key={}\n", evidence.association_key));
    out.push_str(&format!(
        "row_key_sha256={}\n",
        evidence.db_readback.row_key_sha256
    ));
    out.push_str(&format!(
        "value_bytes={}\n",
        evidence.db_readback.value_bytes
    ));
    out.push_str(&format!(
        "value_sha256={}\n",
        evidence.db_readback.value_sha256
    ));
    out.push_str(&format!(
        "db_readback_matches={}\n",
        evidence.db_readback.readback_matches
    ));
    out.push_str(&format!("readback_matches={}\n", evidence.readback_matches));
    out.push_str(&format!(
        "status={} gate_passed={}\n",
        evidence.status, evidence.gate_passed
    ));
    out.push_str(&format!(
        "report_count={} lens_count={} passing_lens_count={}\n",
        evidence.report_count, evidence.lens_count, evidence.passing_lens_count
    ));
    out.push_str(&format!("weakest_lens={}\n", evidence.weakest_lens));
    out.push_str(&format!(
        "min_best_marginal_bits={:.9} max_best_marginal_bits={:.9}\n",
        evidence.min_best_marginal_bits, evidence.max_best_marginal_bits
    ));
    if !evidence.report_path.is_empty() {
        out.push_str(&format!("report_path={}\n", evidence.report_path));
        out.push_str(&format!("lens_values_path={}\n", evidence.lens_values_path));
        out.push_str(&format!(
            "target_summary_path={}\n",
            evidence.target_summary_path
        ));
        out.push_str(&format!("report_sha256={}\n", evidence.report_sha256));
        out.push_str(&format!("readback_sha256={}\n", evidence.readback_sha256));
    }
    out
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
