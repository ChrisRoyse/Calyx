use std::fs;
use std::path::Path;

use serde::Serialize;

use super::engine::OracleSufficiencyReport;
use super::request::OracleSufficiencyRequest;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MetricEvidence {
    pub(crate) metrics_dir: String,
    pub(crate) i_panel_path: String,
    pub(crate) entropy_path: String,
    pub(crate) deficit_path: String,
    pub(crate) refused_path: String,
    pub(crate) sufficiency_json_path: String,
    pub(crate) cf_root: String,
    pub(crate) rows_persisted: usize,
    pub(crate) rows_readback: usize,
    pub(crate) report: OracleSufficiencyReport,
}

pub(crate) fn write_metric_outputs(
    request: &OracleSufficiencyRequest,
    report: &OracleSufficiencyReport,
) -> Result<MetricEvidence, String> {
    check_finite(report)?;
    fs::create_dir_all(&request.metrics_dir).map_err(|error| error.to_string())?;

    let i_panel = request.metrics_dir.join("oracle_i_panel.txt");
    fs::write(&i_panel, format!("{:.6}\n", report.i_panel_oracle))
        .map_err(|error| error.to_string())?;

    let entropy = request.metrics_dir.join("oracle_entropy.txt");
    fs::write(&entropy, format!("{:.6}\n", report.h_y)).map_err(|error| error.to_string())?;

    let deficit = request.metrics_dir.join("oracle_deficit.txt");
    fs::write(&deficit, format!("{:.6}\n", report.deficit)).map_err(|error| error.to_string())?;

    let refused = request.metrics_dir.join("oracle_refused.txt");
    fs::write(
        &refused,
        format!(
            "refused={} sufficient={}\n",
            report.refused, report.sufficient
        ),
    )
    .map_err(|error| error.to_string())?;

    let sufficiency_json = request.metrics_dir.join("oracle_sufficiency.json");
    fs::write(
        &sufficiency_json,
        serde_json::to_vec_pretty(report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;

    Ok(MetricEvidence {
        metrics_dir: request.metrics_dir.display().to_string(),
        i_panel_path: display(&i_panel),
        entropy_path: display(&entropy),
        deficit_path: display(&deficit),
        refused_path: display(&refused),
        sufficiency_json_path: display(&sufficiency_json),
        cf_root: report.cf_root.clone(),
        rows_persisted: report.rows_persisted,
        rows_readback: report.rows_readback,
        report: report.clone(),
    })
}

fn check_finite(report: &OracleSufficiencyReport) -> Result<(), String> {
    let mut values = vec![
        ("h_y", report.h_y),
        ("i_panel_oracle", report.i_panel_oracle),
        ("i_panel_ci_low", report.i_panel_ci[0]),
        ("i_panel_ci_high", report.i_panel_ci[1]),
        ("deficit", report.deficit),
    ];
    for lens in &report.lenses {
        values.push(("lens.bits", lens.bits));
        values.push(("lens.ci_low", lens.ci[0]));
        values.push(("lens.ci_high", lens.ci[1]));
        values.push(("lens.accuracy", lens.accuracy));
    }
    for sensor in &report.per_sensor_deficit {
        values.push(("per_sensor_deficit", sensor.deficit));
    }
    for (name, value) in values {
        if !value.is_finite() {
            return Err(format!("CALYX_FSV_ORACLE_NONFINITE_METRIC: {name}={value}"));
        }
    }
    Ok(())
}

fn display(path: &Path) -> String {
    path.display().to_string()
}
