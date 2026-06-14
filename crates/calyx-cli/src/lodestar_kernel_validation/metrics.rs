use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::engine::{CorpusReport, LodestarKernelValidationReport};
use super::request::LodestarKernelRequest;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MetricEvidence {
    pub(crate) metrics_dir: String,
    pub(crate) summary_path: String,
    pub(crate) corpora_passed: usize,
    pub(crate) min_observed_ratio: f32,
    pub(crate) ratio_files: Vec<String>,
    pub(crate) kernel_recall_files: Vec<String>,
    pub(crate) full_recall_files: Vec<String>,
    pub(crate) gaps_files: Vec<String>,
    pub(crate) report: LodestarKernelValidationReport,
}

pub(crate) fn write_metric_outputs(
    request: &LodestarKernelRequest,
    report: &LodestarKernelValidationReport,
) -> Result<MetricEvidence, String> {
    fs::create_dir_all(&request.metrics_dir).map_err(|error| error.to_string())?;
    let mut ratio_files = Vec::new();
    let mut kernel_recall_files = Vec::new();
    let mut full_recall_files = Vec::new();
    let mut gaps_files = Vec::new();
    for corpus in &report.corpora {
        let prefix = request
            .metrics_dir
            .join(format!("lodestar_{}", corpus.corpus));
        let full = prefix.with_file_name(format!("lodestar_{}_full_recall.txt", corpus.corpus));
        let kernel = prefix.with_file_name(format!("lodestar_{}_kernel_recall.txt", corpus.corpus));
        let ratio = prefix.with_file_name(format!("lodestar_{}_recall_ratio.txt", corpus.corpus));
        let gaps = prefix.with_file_name(format!("lodestar_{}_gaps.txt", corpus.corpus));
        let summary = prefix.with_file_name(format!("lodestar_{}_summary.json", corpus.corpus));
        write_float(&full, corpus.tuned_recall.full)?;
        write_float(&kernel, corpus.tuned_recall.kernel_only)?;
        write_float(&ratio, corpus.tuned_recall.ratio)?;
        write_gaps(&gaps, corpus)?;
        fs::write(
            &summary,
            serde_json::to_vec_pretty(corpus).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        full_recall_files.push(full.display().to_string());
        kernel_recall_files.push(kernel.display().to_string());
        ratio_files.push(ratio.display().to_string());
        gaps_files.push(gaps.display().to_string());
    }
    let summary_path = request.metrics_dir.join("lodestar_kernel_summary.json");
    fs::write(
        &summary_path,
        serde_json::to_vec_pretty(report).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(MetricEvidence {
        metrics_dir: request.metrics_dir.display().to_string(),
        summary_path: summary_path.display().to_string(),
        corpora_passed: report.corpora_passed,
        min_observed_ratio: report.min_observed_ratio,
        ratio_files,
        kernel_recall_files,
        full_recall_files,
        gaps_files,
        report: report.clone(),
    })
}

fn write_float(path: &Path, value: f32) -> std::result::Result<(), String> {
    if !value.is_finite() {
        return Err(format!(
            "CALYX_FSV_LODESTAR_NONFINITE_METRIC: {}",
            path.display()
        ));
    }
    fs::write(path, format!("{value:.6}\n")).map_err(|error| error.to_string())
}

fn write_gaps(path: &PathBuf, corpus: &CorpusReport) -> std::result::Result<(), String> {
    let mut out = String::new();
    out.push_str(&format!("corpus={}\n", corpus.corpus));
    out.push_str(&format!(
        "grounded_fraction={:.6}\n",
        corpus.grounding_gaps.grounded_fraction
    ));
    out.push_str(&format!("gap_count={}\n", corpus.grounding_gaps.gaps.len()));
    for gap in corpus.grounding_gaps.gaps.iter().take(200) {
        out.push_str(&format!("gap={gap}\n"));
    }
    fs::write(path, out).map_err(|error| error.to_string())
}
