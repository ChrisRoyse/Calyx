use std::fs;
use std::path::Path;

use calyx_poly::{
    ERR_METRICS_ALERT, ERR_METRICS_MALFORMED, ERR_METRICS_MISSING_SINK,
    ERR_OBSERVABILITY_STALE_INGEST, FORECAST_OBSERVABILITY_METRICS_FILE,
    FORECAST_OBSERVABILITY_REPORT_FILE, ForecastObservabilityReport, ForecastObservabilityRequest,
    ForecastObservabilityStatus, ForecastObservabilityThresholds, ForecastQualityMetrics,
    ForecastRefusalMetric, read_forecast_observability_metrics, read_forecast_observability_report,
    require_forecast_observability_healthy, run_forecast_observability_report,
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const LAST_INGEST_MS: u64 = 1_785_400_000_000;
const EVALUATED_MS: u64 = LAST_INGEST_MS + 2_000;
const MAX_INGEST_AGE_MS: u64 = 60_000;
const HEALTHY_CODE: &str = "CALYX_POLY_OBSERVABILITY_HEALTHY";

#[test]
fn issue125_forecast_observability_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE125_FSV_ROOT", "poly-issue125-observability");
    reset_dir(&root);

    let happy = run_observability_case(&root, "happy", healthy_request, HEALTHY_CODE);
    let alert = run_observability_case(&root, "alert", alert_request, ERR_METRICS_ALERT);
    let stale = run_observability_case(
        &root,
        "stale-ingest",
        stale_request,
        ERR_OBSERVABILITY_STALE_INGEST,
    );
    let missing =
        run_metrics_read_case(&root, "missing-metric-sink", None, ERR_METRICS_MISSING_SINK);
    let malformed = run_metrics_read_case(
        &root,
        "malformed-metric-payload",
        Some("poly_forecast_brier_mean{source=\"local\" nope\n"),
        ERR_METRICS_MALFORMED,
    );

    for case in [&happy, &alert, &stale, &missing, &malformed] {
        assert_eq!(case["ok"], true, "case did not meet expected proof: {case}");
    }

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 125,
        "proof_claim": "Poly emits local forecast-quality, refusal, agent-failure, ingest-freshness, and association-coverage metrics; reads the emitted metrics back from disk; persists an alert report; and fails closed on missing, stale, or malformed observability evidence.",
        "minimum_sufficient_proof_corpus": {
            "selected": "one healthy window, one alerting window, one stale-ingest request, one missing metrics file, and one malformed metrics file",
            "why_smaller_would_not_prove": "the happy window proves emission/readback, the alerting window proves structured alert evidence, and missing/stale/malformed are three separate fail-closed paths named in #125",
            "why_larger_would_be_wasteful": "more forecast rows or more metric samples would repeat the same aggregate metric, parser, readback, and alert code paths without adding proof"
        },
        "source_of_truth": [
            "per-case forecast_observability.metrics files read from disk",
            "per-case forecast_observability_report.json artifacts read back from disk",
            "per-case before.json and after.json file-state readbacks"
        ],
        "happy_path": happy,
        "edge_cases": {
            "alert": alert,
            "stale_ingest": stale,
            "missing_metric_sink": missing,
            "malformed_metric_payload": malformed
        },
        "physical_files": files
    });
    write_json(&root.join("readback.json"), &summary);
    write_blake3sums(&root);
    println!(
        "issue125_fsv_summary={}",
        serde_json::to_string_pretty(&summary).expect("encode summary")
    );
    if keep_root {
        println!("poly_issue125_fsv_root={}", root.display());
    }
}

fn run_observability_case(
    root: &Path,
    name: &str,
    builder: fn(&Path) -> ForecastObservabilityRequest<'_>,
    expected_code: &str,
) -> Value {
    let case_dir = root.join(name);
    reset_dir(&case_dir);
    let metrics_path = case_dir.join(FORECAST_OBSERVABILITY_METRICS_FILE);
    let report_path = case_dir.join(FORECAST_OBSERVABILITY_REPORT_FILE);
    let before = state(&case_dir, &metrics_path, &report_path);
    write_json(&case_dir.join("before.json"), &before);

    let request = builder(&case_dir);
    let observed = match run_forecast_observability_report(&request) {
        Ok(run) => {
            let readback =
                read_forecast_observability_report(&run.report_path).expect("read report");
            assert_eq!(readback, run.report, "report must round-trip exactly");
            assert!(
                read_forecast_observability_metrics(&run.metrics_path)
                    .expect("read emitted metrics")
                    .len()
                    >= 7,
                "emitted metrics must read back"
            );
            match require_forecast_observability_healthy(&readback) {
                Ok(()) => HEALTHY_CODE.to_string(),
                Err(err) => err.code().to_string(),
            }
        }
        Err(err) => err.code().to_string(),
    };

    let after = state(&case_dir, &metrics_path, &report_path);
    write_json(&case_dir.join("after.json"), &after);
    let outcome = json!({
        "case": name,
        "expected_code": expected_code,
        "observed_code": observed,
        "ok": observed == expected_code,
        "before": before,
        "after": after
    });
    write_json(&case_dir.join("outcome.json"), &outcome);
    outcome
}

fn run_metrics_read_case(
    root: &Path,
    name: &str,
    metrics_text: Option<&str>,
    expected_code: &str,
) -> Value {
    let case_dir = root.join(name);
    reset_dir(&case_dir);
    let metrics_path = case_dir.join(FORECAST_OBSERVABILITY_METRICS_FILE);
    let report_path = case_dir.join(FORECAST_OBSERVABILITY_REPORT_FILE);
    if let Some(text) = metrics_text {
        fs::write(&metrics_path, text).expect("write malformed metrics fixture");
    }
    let before = state(&case_dir, &metrics_path, &report_path);
    write_json(&case_dir.join("before.json"), &before);
    let observed = match read_forecast_observability_metrics(&metrics_path) {
        Ok(_) => HEALTHY_CODE.to_string(),
        Err(err) => err.code().to_string(),
    };
    let after = state(&case_dir, &metrics_path, &report_path);
    write_json(&case_dir.join("after.json"), &after);
    let outcome = json!({
        "case": name,
        "expected_code": expected_code,
        "observed_code": observed,
        "ok": observed == expected_code,
        "before": before,
        "after": after
    });
    write_json(&case_dir.join("outcome.json"), &outcome);
    outcome
}

fn healthy_request(out_dir: &Path) -> ForecastObservabilityRequest<'_> {
    base_request(
        out_dir,
        EVALUATED_MS,
        ForecastQualityMetrics {
            scored_forecasts: 6,
            mean_brier: 0.12,
            mean_calibration_abs_error: 0.04,
            direction_accuracy: 0.83,
        },
        vec![
            ForecastRefusalMetric {
                code: "kernel_recall".to_string(),
                count: 2,
            },
            ForecastRefusalMetric {
                code: "ward_guard".to_string(),
                count: 1,
            },
        ],
        0,
        0.91,
    )
}

fn alert_request(out_dir: &Path) -> ForecastObservabilityRequest<'_> {
    base_request(
        out_dir,
        EVALUATED_MS,
        ForecastQualityMetrics {
            scored_forecasts: 6,
            mean_brier: 0.41,
            mean_calibration_abs_error: 0.22,
            direction_accuracy: 0.40,
        },
        vec![ForecastRefusalMetric {
            code: "kernel_recall".to_string(),
            count: 8,
        }],
        1,
        0.60,
    )
}

fn stale_request(out_dir: &Path) -> ForecastObservabilityRequest<'_> {
    let mut req = healthy_request(out_dir);
    req.evaluated_at_millis = LAST_INGEST_MS + MAX_INGEST_AGE_MS + 1;
    req
}

fn base_request(
    out_dir: &Path,
    evaluated_at_millis: u64,
    quality: ForecastQualityMetrics,
    refusals: Vec<ForecastRefusalMetric>,
    deepseek_agent_failures_total: u64,
    association_coverage_ratio: f64,
) -> ForecastObservabilityRequest<'_> {
    ForecastObservabilityRequest {
        source: "local-forecast-window",
        out_dir,
        evaluated_at_millis,
        ingest_last_observed_millis: LAST_INGEST_MS,
        max_ingest_age_millis: MAX_INGEST_AGE_MS,
        quality,
        refusals,
        deepseek_agent_failures_total,
        association_coverage_ratio,
        thresholds: ForecastObservabilityThresholds {
            max_mean_brier: 0.25,
            max_calibration_abs_error: 0.10,
            max_refusals_total: 5,
            max_agent_failures_total: 0,
            min_association_coverage_ratio: 0.80,
        },
    }
}

fn state(case_dir: &Path, metrics_path: &Path, report_path: &Path) -> Value {
    json!({
        "case_dir_exists": case_dir.exists(),
        "metrics": file_state(metrics_path),
        "report": report_state(report_path)
    })
}

fn file_state(path: &Path) -> Value {
    let bytes = fs::read(path).ok();
    json!({
        "path": path.display().to_string(),
        "exists": path.exists(),
        "bytes": bytes.as_ref().map(Vec::len).unwrap_or(0),
        "blake3": bytes.as_ref().map(|b| hex(blake3::hash(b).as_bytes()))
    })
}

fn report_state(path: &Path) -> Value {
    let bytes = fs::read(path).ok();
    let readback = bytes
        .as_ref()
        .and_then(|b| serde_json::from_slice::<ForecastObservabilityReport>(b).ok());
    json!({
        "path": path.display().to_string(),
        "exists": path.exists(),
        "bytes": bytes.as_ref().map(Vec::len).unwrap_or(0),
        "blake3": bytes.as_ref().map(|b| hex(blake3::hash(b).as_bytes())),
        "readback": readback.as_ref().map(report_summary)
    })
}

fn report_summary(report: &ForecastObservabilityReport) -> Value {
    json!({
        "status": match report.status {
            ForecastObservabilityStatus::Healthy => "healthy",
            ForecastObservabilityStatus::Alert => "alert",
        },
        "sample_count": report.samples.len(),
        "alert_count": report.alert_count,
        "quality": report.quality,
        "refusals": report.refusals,
        "checks": report.checks.iter().map(|check| {
            json!({
                "metric": &check.metric,
                "value": check.value,
                "comparator": &check.comparator,
                "threshold": check.threshold,
                "passed": check.passed,
                "code": &check.code
            })
        }).collect::<Vec<_>>()
    })
}
