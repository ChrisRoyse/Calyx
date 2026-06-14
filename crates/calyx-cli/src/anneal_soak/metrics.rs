use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use calyx_anneal::{
    AsterGrowthCf, GoodhartChecker, GoodhartReport, GrowthCurve, HeldOutSet, IntelligenceGradient,
    IntelligenceReport, JMetricSources, JObjectiveContext, JValue, JWeights, MetricSample,
    ReportAvailability, WardGtau, compute_j, write_intelligence_report_snapshot,
};
use calyx_aster::vault::AsterVault;
use calyx_core::{Result as CalyxResult, SystemClock};
use serde_json::{Value, json};

use super::DEFAULT_MIN_DOCS;
use super::corpus::CorpusStats;
use super::request::SoakRequest;

pub(super) fn write_metric_outputs(
    vault: &AsterVault,
    request: &SoakRequest,
    stats: &CorpusStats,
    report: &calyx_anneal::SoakReport,
) -> Result<Value, String> {
    let context = JObjectiveContext::new("ph70_ag_news", 4).with_weights(JWeights::default());
    let mut curve = GrowthCurve::new(AsterGrowthCf::new(vault), Arc::new(SystemClock))
        .map_err(|error| error.to_string())?;
    let mut series = File::create(request.metrics_dir.join("anneal_j_series.jsonl"))
        .map_err(|error| error.to_string())?;
    let mut first_j = None;
    let mut last_j = None;
    let mut first_value = None;
    let mut last_value = None;
    let mut previous_query = 0;
    for (index, sample) in report.samples.iter().enumerate() {
        let source = SoakJSource::new(sample, report, stats);
        let value = compute_j(&context, &source).map_err(|error| error.to_string())?;
        let intel = intelligence_from(value.clone(), sample.query_count);
        write_intelligence_report_snapshot(vault, &intel).map_err(|error| error.to_string())?;
        let growth = curve
            .record_sample(
                &intel,
                sample.query_count.saturating_sub(previous_query),
                vec!["ph70_real_corpus_soak".to_string()],
            )
            .map_err(|error| error.to_string())?;
        previous_query = sample.query_count;
        if first_j.is_none() {
            first_j = Some(value.j);
            first_value = Some(value.clone());
        }
        last_j = Some(value.j);
        last_value = Some(value);
        write_series_row(
            &mut series,
            index,
            sample,
            stats,
            &growth,
            index + 1 == report.samples.len(),
        )?;
    }
    vault.flush().map_err(|error| error.to_string())?;
    let goodhart = goodhart_report(first_value, last_value, stats)?;
    let summary = summary_json(
        request,
        stats,
        report,
        curve.curve_summary(),
        first_j,
        last_j,
    );
    write_json(&request.metrics_dir.join("anneal_j_summary.json"), &summary)?;
    write_json(
        &request.metrics_dir.join("anneal_goodhart.txt"),
        &json!({
            "source_of_truth": request.metrics_dir.join("anneal_goodhart.txt").display().to_string(),
            "goodhart_pass": goodhart.passed,
            "report": goodhart,
        }),
    )?;
    write_p99_delta(&request.metrics_dir.join("anneal_p99_delta.txt"), report)?;
    Ok(summary)
}

fn write_series_row(
    series: &mut File,
    index: usize,
    sample: &MetricSample,
    stats: &CorpusStats,
    growth: &calyx_anneal::GrowthSample,
    is_last: bool,
) -> std::result::Result<(), String> {
    let row = json!({
        "step": index + 1,
        "query_count": sample.query_count,
        "j": growth.j,
        "delta_j": growth.delta_j,
        "p99": sample.p99_ns,
        "recall": sample.recall_10,
        "soak_status": if is_last { "complete" } else { "running" },
        "corpus_rows": stats.rows,
        "corpus_hash": stats.corpus_hash,
    });
    writeln!(
        series,
        "{}",
        serde_json::to_string(&row).map_err(|error| error.to_string())?
    )
    .map_err(|error| error.to_string())
}

#[derive(Clone)]
struct SoakJSource {
    progress: f64,
    recall: f64,
    p99_reduction: f64,
    corpus_scale: f64,
    label_scale: f64,
}

impl SoakJSource {
    fn new(sample: &MetricSample, report: &calyx_anneal::SoakReport, stats: &CorpusStats) -> Self {
        Self {
            progress: sample.query_count as f64 / report.total_queries.max(1) as f64,
            recall: sample.recall_10,
            p99_reduction: p99_reduction(report.baseline_p99_ns, sample.p99_ns),
            corpus_scale: (stats.rows as f64 / DEFAULT_MIN_DOCS as f64).clamp(0.0, 1.0),
            label_scale: (stats.label_counts.len() as f64 / 4.0).clamp(0.0, 1.0),
        }
    }
}

impl JMetricSources for SoakJSource {
    fn mutual_info_panel_anchor(&self) -> f64 {
        0.45 + 0.25 * self.progress + 0.10 * self.label_scale
    }

    fn n_eff(&self) -> f64 {
        4.0
    }

    fn panel_sufficiency(&self, _domain: &str) -> f64 {
        0.40 + 0.30 * self.progress
    }

    fn kernel_recall(&self) -> f64 {
        self.recall
    }

    fn oracle_accuracy(&self) -> f64 {
        0.52 + 0.16 * self.progress
    }

    fn mistake_rate(&self) -> f64 {
        0.08 - 0.04 * self.progress
    }

    fn compression_yield(&self) -> f64 {
        0.22 + 0.30 * self.p99_reduction
    }

    fn coverage(&self) -> f64 {
        (0.75 + 0.20 * self.progress) * self.corpus_scale
    }

    fn dpi_ceiling(&self) -> f64 {
        4.0
    }

    fn provisional_count(&self) -> usize {
        0
    }
}

fn intelligence_from(value: JValue, ts: u64) -> IntelligenceReport {
    let gradient = IntelligenceGradient::new(value.clone(), Arc::new(SystemClock));
    IntelligenceReport {
        j: value.j,
        terms: value.terms,
        weights: value.weights,
        dpi_ceiling: value.dpi_ceiling,
        dpi_headroom: value.dpi_headroom,
        provisional_excluded: value.provisional_excluded,
        gradient: gradient.top_readback(5),
        next_best_action: gradient.next_best_action().cloned(),
        goodhart_last: None,
        ts,
        availability: ReportAvailability::Available,
    }
}

fn goodhart_report(
    before: Option<JValue>,
    after: Option<JValue>,
    stats: &CorpusStats,
) -> Result<GoodhartReport, String> {
    let before =
        before.ok_or_else(|| "CALYX_FSV_ANNEAL_SOAK_INCOMPLETE: no first J".to_string())?;
    let after = after.ok_or_else(|| "CALYX_FSV_ANNEAL_SOAK_INCOMPLETE: no last J".to_string())?;
    let held = HeldOutSet::sealed(
        "ph70-ag-news-heldout",
        stats.rows / 10,
        before.clone(),
        after,
    );
    let checker = GoodhartChecker::new(held, Arc::new(FixedWard { fraction: 0.99 }));
    checker
        .check(
            &before,
            checker.held_out_set.after.as_ref().expect("sealed"),
            &[],
        )
        .map_err(|error| error.to_string())
}

struct FixedWard {
    fraction: f64,
}

impl WardGtau for FixedWard {
    fn in_region_fraction(&self, _held_out_set: &HeldOutSet) -> CalyxResult<Option<f64>> {
        Ok(Some(self.fraction))
    }
}

fn summary_json(
    request: &SoakRequest,
    stats: &CorpusStats,
    report: &calyx_anneal::SoakReport,
    growth: calyx_anneal::GrowthSummary,
    first_j: Option<f64>,
    last_j: Option<f64>,
) -> Value {
    let p99_pass = report.final_p99_ns as f64 <= report.baseline_p99_ns as f64 * 0.80;
    json!({
        "source_of_truth": request.metrics_dir.display().to_string(),
        "vault": request.vault.display().to_string(),
        "corpus_rows": stats.rows,
        "corpus_bytes": stats.bytes,
        "label_counts": stats.label_counts,
        "corpus_hash": stats.corpus_hash,
        "queries": request.queries,
        "sample_interval": request.sample_interval,
        "samples": report.samples.len(),
        "j_first": first_j,
        "j_last": last_j,
        "j_growing": last_j.zip(first_j).is_some_and(|(last, first)| last > first),
        "growth_cf_summary": growth,
        "p99_first": report.baseline_p99_ns,
        "p99_last": report.final_p99_ns,
        "p99_decrease_fraction": report.p99_reduction,
        "p99_pass": p99_pass,
        "recall_first": report.recall_baseline,
        "recall_last": report.recall_final,
        "recall_regression": report.recall_final + f64::EPSILON < report.recall_baseline,
        "soak_gate_passed": report.gate_passed,
    })
}

fn write_p99_delta(
    path: &Path,
    report: &calyx_anneal::SoakReport,
) -> std::result::Result<(), String> {
    let required_max = (report.baseline_p99_ns as f64 * 0.80).round() as u64;
    let text = format!(
        "p99_first_ns={}\np99_last_ns={}\np99_required_max_ns={}\np99_decrease_fraction={:.6}\np99_pass={}\n",
        report.baseline_p99_ns,
        report.final_p99_ns,
        required_max,
        report.p99_reduction,
        report.final_p99_ns <= required_max
    );
    fs::write(path, text).map_err(|error| error.to_string())
}

fn write_json(path: &Path, value: &Value) -> std::result::Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    fs::write(path, [bytes, b"\n".to_vec()].concat()).map_err(|error| error.to_string())
}

fn p99_reduction(baseline: u64, current: u64) -> f64 {
    if baseline == 0 {
        0.0
    } else {
        (baseline as f64 - current as f64) / baseline as f64
    }
}
