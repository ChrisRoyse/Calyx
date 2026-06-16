use std::fs;
use std::path::Path;

use serde::Serialize;

use super::engine::WardGuardReport;
use super::request::WardGuardRequest;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MetricEvidence {
    pub(crate) metrics_dir: String,
    pub(crate) tau_path: String,
    pub(crate) block_rate_path: String,
    pub(crate) far_path: String,
    pub(crate) frr_path: String,
    pub(crate) novelty_routed_path: String,
    pub(crate) report_path: String,
    pub(crate) verdicts_path: String,
    pub(crate) report: WardGuardReport,
}

pub(crate) fn write_metric_outputs(
    request: &WardGuardRequest,
    report: &WardGuardReport,
) -> Result<MetricEvidence, String> {
    check_finite(report)?;
    fs::create_dir_all(&request.metrics_dir).map_err(|error| error.to_string())?;

    let tau_path = request.metrics_dir.join("ward_tau.txt");
    write_float(&tau_path, report.tau)?;

    let block_rate_path = request.metrics_dir.join("ward_block_rate.txt");
    write_float(&block_rate_path, report.heldout.block_rate)?;

    let far_path = request.metrics_dir.join("ward_far.txt");
    write_float(&far_path, report.heldout.heldout_far)?;

    let frr_path = request.metrics_dir.join("ward_frr.txt");
    write_float(&frr_path, report.heldout.benign_frr)?;

    let novelty_routed_path = request.metrics_dir.join("ward_novelty_routed.txt");
    fs::write(
        &novelty_routed_path,
        format!(
            "routed={} action=new_region novel_regions={}\n",
            report.novelty.routed, report.novelty.novel_regions
        ),
    )
    .map_err(|error| error.to_string())?;

    let report_path = request.metrics_dir.join("ward_guard_validate.json");
    fs::write(
        &report_path,
        serde_json::to_vec_pretty(report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;

    Ok(MetricEvidence {
        metrics_dir: request.metrics_dir.display().to_string(),
        tau_path: display(&tau_path),
        block_rate_path: display(&block_rate_path),
        far_path: display(&far_path),
        frr_path: display(&frr_path),
        novelty_routed_path: display(&novelty_routed_path),
        report_path: display(&report_path),
        verdicts_path: report.verdicts_path.clone(),
        report: report.clone(),
    })
}

fn write_float(path: &Path, value: f32) -> Result<(), String> {
    fs::write(path, format!("{value:.6}\n")).map_err(|error| error.to_string())
}

fn check_finite(report: &WardGuardReport) -> Result<(), String> {
    let values = [
        ("tau", report.tau),
        ("calibration.meta_far", report.calibration.meta_far),
        ("calibration.meta_frr", report.calibration.meta_frr),
        ("heldout.block_rate", report.heldout.block_rate),
        ("heldout.benign_frr", report.heldout.benign_frr),
        ("heldout.benign_acc", report.heldout.benign_acc),
        ("heldout.heldout_far", report.heldout.heldout_far),
    ];
    for (name, value) in values {
        if !value.is_finite() {
            return Err(format!("CALYX_FSV_WARD_NONFINITE_METRIC: {name}={value}"));
        }
    }
    Ok(())
}

fn display(path: &Path) -> String {
    path.display().to_string()
}
