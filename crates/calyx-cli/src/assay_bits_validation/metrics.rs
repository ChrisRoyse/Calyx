use std::fs;
use std::path::Path;

use serde::Serialize;

use super::engine::AssayBitsReport;
use super::request::AssayBitsRequest;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MetricEvidence {
    pub(crate) metrics_dir: String,
    pub(crate) abundance_path: String,
    pub(crate) bits_per_lens_path: String,
    pub(crate) rejection_log_path: String,
    pub(crate) cf_root: String,
    pub(crate) assay_cf_rows_persisted: usize,
    pub(crate) assay_cf_rows_readback: usize,
    pub(crate) report: AssayBitsReport,
}

pub(crate) fn write_metric_outputs(
    request: &AssayBitsRequest,
    report: &AssayBitsReport,
) -> Result<MetricEvidence, String> {
    check_finite(report)?;
    fs::create_dir_all(&request.metrics_dir).map_err(|error| error.to_string())?;

    let abundance = request.metrics_dir.join("assay_abundance.json");
    fs::write(
        &abundance,
        serde_json::to_vec_pretty(report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;

    let bits_per_lens = request.metrics_dir.join("assay_bits_per_lens.txt");
    let mut lens_lines = String::new();
    for lens in &report.lenses {
        lens_lines.push_str(&format!(
            "lens={} bits={:.6} admitted={}\n",
            lens.name, lens.bits_about, lens.admitted
        ));
    }
    fs::write(&bits_per_lens, lens_lines).map_err(|error| error.to_string())?;

    let rejection_log = request.metrics_dir.join("assay_rejection_log.txt");
    let mut rejection_lines = String::new();
    for lens in &report.lenses {
        if let Some(reason) = &lens.rejection_reason {
            rejection_lines.push_str(&format!(
                "lens={} reason={} corr={:.6}\n",
                lens.name, reason, lens.max_pairwise_corr
            ));
        }
    }
    if rejection_lines.is_empty() {
        rejection_lines.push_str("no_rejections\n");
    }
    fs::write(&rejection_log, rejection_lines).map_err(|error| error.to_string())?;

    Ok(MetricEvidence {
        metrics_dir: request.metrics_dir.display().to_string(),
        abundance_path: display(&abundance),
        bits_per_lens_path: display(&bits_per_lens),
        rejection_log_path: display(&rejection_log),
        cf_root: report.cf_root.clone(),
        assay_cf_rows_persisted: report.assay_cf_rows_persisted,
        assay_cf_rows_readback: report.assay_cf_rows_readback,
        report: report.clone(),
    })
}

fn check_finite(report: &AssayBitsReport) -> Result<(), String> {
    let mut values = vec![
        ("anchor_entropy_bits", report.anchor_entropy_bits),
        ("panel.i_panel_anchor", report.panel.i_panel_anchor),
        ("panel.ci_low", report.panel.ci_95[0]),
        ("panel.ci_high", report.panel.ci_95[1]),
    ];
    for lens in &report.lenses {
        values.push(("lens.bits_about", lens.bits_about));
        values.push(("lens.ci_low", lens.ci[0]));
        values.push(("lens.ci_high", lens.ci[1]));
        values.push(("lens.max_pairwise_corr", lens.max_pairwise_corr));
    }
    for stratum in &report.strata {
        values.push(("stratum.bits", stratum.bits));
        values.push(("stratum.frequency", stratum.frequency));
    }
    for (name, value) in values {
        if !value.is_finite() {
            return Err(format!("CALYX_FSV_ASSAY_NONFINITE_METRIC: {name}={value}"));
        }
    }
    Ok(())
}

fn display(path: &Path) -> String {
    path.display().to_string()
}
