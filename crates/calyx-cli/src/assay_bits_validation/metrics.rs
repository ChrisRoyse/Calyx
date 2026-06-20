use std::fs;
use std::path::Path;

use serde::Serialize;

use super::report::AssayBitsReport;
use super::request::AssayBitsRequest;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MetricEvidence {
    pub(crate) metrics_dir: String,
    pub(crate) abundance_path: String,
    pub(crate) bits_per_lens_path: String,
    pub(crate) rejection_log_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) signal_density_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) packed_panel_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) panel_comparison_path: Option<String>,
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
        let seed_sigma = required_f32(lens.seed_sigma_bits, "lens.seed_sigma_bits")?;
        let power_recovery = required_f32(lens.power_recovery_ratio, "lens.power_recovery_ratio")?;
        let power_status = required_str(
            lens.power_calibration_status.as_deref(),
            "lens.power_calibration_status",
        )?;
        lens_lines.push_str(&format!(
            "lens={} bits={:.6} ci=[{:.6},{:.6}] bound={} seed_sigma={:.6} power_status={} power_recovery={:.6} admitted={}\n",
            lens.name,
            lens.bits_about,
            lens.ci[0],
            lens.ci[1],
            lens.estimate_bound,
            seed_sigma,
            power_status,
            power_recovery,
            lens.admitted
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

    // Signal-density artifact (only when cost was supplied). This is the
    // selection-facing source of truth consumed by the knapsack (#721/#729).
    let signal_density_path = match &report.signal_density {
        Some(density) => {
            let path = request.metrics_dir.join("assay_signal_density.json");
            fs::write(
                &path,
                serde_json::to_vec_pretty(density).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            Some(display(&path))
        }
        None => None,
    };
    let packed_panel_path = match &report.packed_panel {
        Some(panel) => {
            let path = request.metrics_dir.join("assay_packed_panel.json");
            fs::write(
                &path,
                serde_json::to_vec_pretty(panel).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            Some(display(&path))
        }
        None => None,
    };
    let panel_comparison_path = match &report.panel_comparison {
        Some(comparison) => {
            let path = request.metrics_dir.join("assay_panel_comparison.json");
            fs::write(
                &path,
                serde_json::to_vec_pretty(comparison).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            Some(display(&path))
        }
        None => None,
    };

    Ok(MetricEvidence {
        metrics_dir: request.metrics_dir.display().to_string(),
        abundance_path: display(&abundance),
        bits_per_lens_path: display(&bits_per_lens),
        rejection_log_path: display(&rejection_log),
        signal_density_path,
        packed_panel_path,
        panel_comparison_path,
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
        (
            "panel.sufficiency_basis_bits",
            report.panel.sufficiency_basis_bits,
        ),
    ];
    values.push((
        "panel.power_recovery_ratio",
        required_f32(
            report.panel.power_recovery_ratio,
            "panel.power_recovery_ratio",
        )?,
    ));
    values.push((
        "panel.power_recovered_bits",
        required_f32(
            report.panel.power_recovered_bits,
            "panel.power_recovered_bits",
        )?,
    ));
    values.push((
        "panel.power_planted_bits",
        required_f32(report.panel.power_planted_bits, "panel.power_planted_bits")?,
    ));
    required_str(
        report.panel.power_calibration_status.as_deref(),
        "panel.power_calibration_status",
    )?;
    for lens in &report.lenses {
        values.push(("lens.bits_about", lens.bits_about));
        values.push(("lens.ci_low", lens.ci[0]));
        values.push(("lens.ci_high", lens.ci[1]));
        values.push((
            "lens.power_recovery_ratio",
            required_f32(lens.power_recovery_ratio, "lens.power_recovery_ratio")?,
        ));
        values.push((
            "lens.power_recovered_bits",
            required_f32(lens.power_recovered_bits, "lens.power_recovered_bits")?,
        ));
        values.push((
            "lens.power_planted_bits",
            required_f32(lens.power_planted_bits, "lens.power_planted_bits")?,
        ));
        values.push((
            "lens.seed_sigma_bits",
            required_f32(lens.seed_sigma_bits, "lens.seed_sigma_bits")?,
        ));
        required_str(
            lens.power_calibration_status.as_deref(),
            "lens.power_calibration_status",
        )?;
        values.push(("lens.max_pairwise_corr", lens.max_pairwise_corr));
        values.push((
            "lens.max_pairwise_corr_ci_low",
            lens.max_pairwise_corr_ci[0],
        ));
        values.push((
            "lens.max_pairwise_corr_ci_high",
            lens.max_pairwise_corr_ci[1],
        ));
    }
    for stratum in &report.strata {
        values.push(("stratum.bits", stratum.bits));
        values.push(("stratum.frequency", stratum.frequency));
    }
    if let Some(comparison) = &report.panel_comparison {
        values.push((
            "panel_comparison.density_panel.total_signal_bits",
            comparison.density_panel.total_signal_bits,
        ));
        if let Some(control) = &comparison.best_few_lens_control {
            values.push((
                "panel_comparison.best_few_lens_control.total_signal_bits",
                control.total_signal_bits,
            ));
        }
        if let Some(gain) = comparison.signal_gain_bits {
            values.push(("panel_comparison.signal_gain_bits", gain));
        }
        if let Some(ratio) = comparison.signal_gain_ratio {
            values.push(("panel_comparison.signal_gain_ratio", ratio));
        }
    }
    for (name, value) in values {
        if !value.is_finite() {
            return Err(format!("CALYX_FSV_ASSAY_NONFINITE_METRIC: {name}={value}"));
        }
    }
    Ok(())
}

fn required_f32(value: Option<f32>, name: &str) -> Result<f32, String> {
    value.ok_or_else(|| format!("CALYX_FSV_ASSAY_MISSING_VERDICT_METADATA: {name} absent"))
}

fn required_str<'a>(value: Option<&'a str>, name: &str) -> Result<&'a str, String> {
    value.ok_or_else(|| format!("CALYX_FSV_ASSAY_MISSING_VERDICT_METADATA: {name} absent"))
}

fn display(path: &Path) -> String {
    path.display().to_string()
}
