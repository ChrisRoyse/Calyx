use std::fs;
use std::path::Path;

use calyx_poly::{
    ERR_METRICS_ALERT, ERR_METRICS_MALFORMED, ERR_METRICS_MISSING_SINK, ERR_METRICS_STALE,
    METRICS_SCRAPE_REPORT_FILE, MetricsScrapeReport, MetricsScrapeRequest, MetricsStatus,
    MetricsThresholds, read_metrics_scrape_report, require_metrics_scrape_healthy,
    run_metrics_scrape_report,
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const SCRAPED_AT: u64 = 1_785_400_000_000;
const EVALUATED_AT: u64 = SCRAPED_AT + 1_000;
const MAX_AGE_MS: u64 = 60_000;
const HEALTHY_CODE: &str = "CALYX_POLY_METRICS_HEALTHY";

#[test]
fn issue126_metrics_scrape_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE126_FSV_ROOT", "poly-issue126-metrics");
    reset_dir(&root);

    let happy = run_case(
        &root,
        "happy",
        Some(healthy_metrics()),
        SCRAPED_AT,
        EVALUATED_AT,
        HEALTHY_CODE,
    );
    let alert = run_case(
        &root,
        "alert-thresholds",
        Some(alert_metrics()),
        SCRAPED_AT,
        EVALUATED_AT,
        ERR_METRICS_ALERT,
    );
    let missing_sink = run_case(
        &root,
        "missing-sink",
        None,
        SCRAPED_AT,
        EVALUATED_AT,
        ERR_METRICS_MISSING_SINK,
    );
    let malformed = run_case(
        &root,
        "malformed-payload",
        Some("poly_kernel_recall_ratio{domain=\"crypto\" 0.982\n"),
        SCRAPED_AT,
        EVALUATED_AT,
        ERR_METRICS_MALFORMED,
    );
    let stale = run_case(
        &root,
        "stale-scrape",
        Some(healthy_metrics()),
        SCRAPED_AT,
        SCRAPED_AT + MAX_AGE_MS + 1,
        ERR_METRICS_STALE,
    );

    for case in [&happy, &alert, &missing_sink, &malformed, &stale] {
        assert_eq!(case["ok"], true, "case did not meet expected proof: {case}");
    }

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 126,
        "proof_claim": "A captured calyxd /metrics text snapshot is parsed, persisted, read back, and evaluated for kernel recall, guard FAR, n_eff, and chain verification; missing sinks, malformed payloads, stale scrapes, and unhealthy metric values fail closed or alert with source-of-truth evidence.",
        "minimum_sufficient_proof_corpus": {
            "selected": "one happy 4-sample metrics snapshot, one 4-sample alert snapshot, one malformed line, one stale 4-sample snapshot, and one missing-sink case",
            "why_smaller_would_not_prove": "four required metric families are the complete #126 surface; fewer than four happy samples would leave a required gate unproven, and the three fail-closed classes plus alert path are distinct code paths",
            "why_larger_would_be_wasteful": "additional metric samples would repeat the same parser, threshold, readback, and alert invariants without proving a new behavior"
        },
        "source_of_truth": [
            "per-case calyxd.metrics text files read from disk",
            "per-case metrics_scrape_report.json artifacts read back from disk",
            "per-case before.json and after.json file-state readbacks"
        ],
        "happy_path": happy,
        "edge_cases": {
            "alert_thresholds": alert,
            "missing_sink": missing_sink,
            "malformed_payload": malformed,
            "stale_scrape": stale
        },
        "physical_files": files
    });
    write_json(&root.join("readback.json"), &summary);
    write_blake3sums(&root);
    println!(
        "issue126_fsv_summary={}",
        serde_json::to_string_pretty(&summary).expect("encode summary")
    );
    if keep_root {
        println!("poly_issue126_fsv_root={}", root.display());
    }
}

fn run_case(
    root: &Path,
    name: &str,
    metrics_text: Option<&str>,
    scraped_at: u64,
    evaluated_at: u64,
    expected_code: &str,
) -> Value {
    let case_dir = root.join(name);
    reset_dir(&case_dir);
    let metrics_path = case_dir.join("calyxd.metrics");
    let report_path = case_dir.join(METRICS_SCRAPE_REPORT_FILE);
    if let Some(text) = metrics_text {
        fs::write(&metrics_path, text).expect("write metrics fixture");
    }

    let before = state(&case_dir, &metrics_path, &report_path);
    write_json(&case_dir.join("before.json"), &before);
    let request = request(&metrics_path, &case_dir, scraped_at, evaluated_at);
    let observed = match run_metrics_scrape_report(&request) {
        Ok(run) => {
            let readback = read_metrics_scrape_report(&run.report_path).expect("readback report");
            assert_eq!(readback, run.report, "report must round-trip exactly");
            match require_metrics_scrape_healthy(&readback) {
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

fn request<'a>(
    metrics_path: &'a Path,
    out_dir: &'a Path,
    scraped_at: u64,
    evaluated_at: u64,
) -> MetricsScrapeRequest<'a> {
    MetricsScrapeRequest {
        source: "local-calyxd-/metrics-file",
        metrics_path,
        out_dir,
        scraped_at_millis: scraped_at,
        evaluated_at_millis: evaluated_at,
        max_age_millis: MAX_AGE_MS,
        thresholds: MetricsThresholds {
            min_kernel_recall_ratio: 0.95,
            max_guard_far_ratio: 0.05,
            min_n_eff: 2.0,
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
        .and_then(|b| serde_json::from_slice::<MetricsScrapeReport>(b).ok());
    json!({
        "path": path.display().to_string(),
        "exists": path.exists(),
        "bytes": bytes.as_ref().map(Vec::len).unwrap_or(0),
        "blake3": bytes.as_ref().map(|b| hex(blake3::hash(b).as_bytes())),
        "readback": readback.as_ref().map(report_summary)
    })
}

fn report_summary(report: &MetricsScrapeReport) -> Value {
    json!({
        "status": match report.status {
            MetricsStatus::Healthy => "healthy",
            MetricsStatus::Alert => "alert",
        },
        "sample_count": report.sample_count,
        "alert_count": report.alert_count,
        "checks": report.checks.iter().map(|check| {
            json!({
                "metric": check.metric,
                "labels": check.labels,
                "value": check.value,
                "comparator": check.comparator,
                "threshold": check.threshold,
                "passed": check.passed,
                "code": check.code
            })
        }).collect::<Vec<_>>()
    })
}

fn healthy_metrics() -> &'static str {
    "# HELP poly_kernel_recall_ratio Last computed kernel recall ratio\n\
poly_kernel_recall_ratio{domain=\"crypto\"} 0.982\n\
poly_guard_far_ratio{guard=\"ward\"} 0.012\n\
poly_panel_n_eff{domain=\"crypto\",panel=\"v1\"} 3.25\n\
poly_chain_verify_passed{chain=\"daily\"} 1\n"
}

fn alert_metrics() -> &'static str {
    "poly_kernel_recall_ratio{domain=\"crypto\"} 0.900\n\
poly_guard_far_ratio{guard=\"ward\"} 0.180\n\
poly_panel_n_eff{domain=\"crypto\",panel=\"v1\"} 1.25\n\
poly_chain_verify_passed{chain=\"daily\"} 0\n"
}
