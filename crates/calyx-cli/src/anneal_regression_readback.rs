use std::fs;
use std::path::Path;

use calyx_anneal::{RegressionReport, regression_rate};
use serde_json::json;

pub fn regression_report(path: &Path) -> crate::error::CliResult {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let report: RegressionReport =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let rate = regression_rate(&report).map_err(|error| error.to_string())?;
    let rows = report
        .results
        .iter()
        .map(|row| {
            json!({
                "cx_id": row.cx_id.to_string(),
                "old_prediction": row.old_prediction,
                "observed": row.observed,
                "old_surprise": row.old_surprise,
                "new_prediction": row.new_prediction,
                "new_surprise": row.new_surprise,
                "recurred": row.recurred,
                "anchor": row.anchor,
                "prediction_error": row.prediction_error,
            })
        })
        .collect::<Vec<_>>();
    let readback = json!({
        "artifact": path.display().to_string(),
        "passed": report.passed,
        "batch_len": report.results.len(),
        "regression_count": report.regression_count,
        "regression_rate": rate,
        "rows": rows,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&readback).map_err(|error| error.to_string())?
    );
    Ok(())
}
