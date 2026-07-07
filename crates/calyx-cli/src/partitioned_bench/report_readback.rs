use std::path::PathBuf;

use serde_json::Value;

use crate::error::{CliError, CliResult};
use crate::output;
use crate::partitioned_rrf_report_store::{self, PartitionedRrfReportDbReadback};

const DEFAULT_LIMIT_SLOTS: usize = 12;

pub(crate) fn run(raw: &[String]) -> CliResult {
    let args = Args::parse(raw)?;
    let (record, readback) = partitioned_rrf_report_store::read(&args.cf_root, &args.report_key)
        .map_err(CliError::from)?;
    output::print_lines(&render(
        &record.report,
        &record.format,
        &record.mode,
        &readback,
        &args,
    ))?;
    Ok(())
}

#[derive(Clone, Debug)]
struct Args {
    cf_root: PathBuf,
    report_key: String,
    limit_slots: usize,
}

impl Args {
    fn parse(raw: &[String]) -> CliResult<Self> {
        let mut cf_root = None;
        let mut report_key = partitioned_rrf_report_store::DEFAULT_ASSOCIATION_KEY.to_string();
        let mut limit_slots = DEFAULT_LIMIT_SLOTS;
        let mut it = raw.iter();
        while let Some(flag) = it.next() {
            let mut next = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
            };
            match flag.as_str() {
                "--cf-root" => cf_root = Some(PathBuf::from(next()?)),
                "--association-key" | "--report-key" => report_key = next()?,
                "--limit-slots" => {
                    limit_slots = parse_positive(&next()?, "--limit-slots")?;
                }
                other => return Err(CliError::usage(format!("unknown flag: {other}"))),
            }
        }
        if report_key.trim().is_empty() {
            return Err(CliError::usage("--report-key must be non-empty"));
        }
        Ok(Self {
            cf_root: cf_root.ok_or_else(|| CliError::usage("--cf-root <aster-dir> is required"))?,
            report_key,
            limit_slots,
        })
    }
}

fn render(
    report: &Value,
    record_format: &str,
    record_mode: &str,
    readback: &PartitionedRrfReportDbReadback,
    args: &Args,
) -> Vec<String> {
    let mut out = Vec::new();
    out.push("partitioned_rrf_report_readback".to_string());
    push(&mut out, "cf_root", &readback.cf_root);
    push(&mut out, "association_key", &readback.association_key);
    push(&mut out, "row_key_sha256", &readback.row_key_sha256);
    push(&mut out, "value_bytes", readback.value_bytes);
    push(&mut out, "value_sha256", &readback.value_sha256);
    push(&mut out, "readback_matches", readback.readback_matches);
    push(&mut out, "record_format", record_format);
    push(&mut out, "record_mode", record_mode);
    push_value(&mut out, report, "trigger", "trigger");
    push_value(&mut out, report, "mode", "report_mode");
    push_value(&mut out, report, "metric_class", "metric_class");
    push_value(&mut out, report, "metric_scope", "metric_scope");
    push_value(&mut out, report, "queries", "queries");
    push_value(&mut out, report, "k", "k");
    push_value(&mut out, report, "n_probe", "n_probe");
    push_value(&mut out, report, "region_beam", "region_beam");
    push_value(&mut out, report, "truth_depth", "truth_depth");
    push_value(
        &mut out,
        report,
        "ground_truth_queries",
        "ground_truth_queries",
    );
    push_value(
        &mut out,
        report,
        "fused_ground_truth_recall_at_k",
        "recall_at_k",
    );
    push_value(&mut out, report, "recall_floor", "recall_floor");
    render_latency(&mut out, report);
    render_counts(&mut out, report, args.limit_slots);
    render_plan(&mut out, report);
    render_ground_truth(&mut out, report);
    render_a37(&mut out, report);
    render_temporal(&mut out, report);
    push_value(
        &mut out,
        report,
        "best_single_lens_recall_vs_fused_truth",
        "best_single_lens_recall_vs_fused_truth",
    );
    push_value(
        &mut out,
        report,
        "fusion_matches_or_beats_best_single",
        "fusion_matches_or_beats_best_single",
    );
    out
}

fn render_latency(out: &mut Vec<String>, report: &Value) {
    let latency = &report["latency_us"];
    for key in ["p50", "p95", "p99", "p999", "max"] {
        push_value(out, latency, key, &format!("latency_{key}_us"));
    }
}

fn render_counts(out: &mut Vec<String>, report: &Value, limit_slots: usize) {
    let roster = report["lens_roster"].as_array();
    let bits = report["per_lens_bits"].as_array();
    let slots = report["slots"].as_array();
    push(out, "lens_roster_len", roster.map_or(0, Vec::len));
    push(out, "per_lens_bits_len", bits.map_or(0, Vec::len));
    push(out, "slots_total", slots.map_or(0, Vec::len));
    push(
        out,
        "slots_shown",
        slots.map_or(0, |rows| rows.len().min(limit_slots)),
    );
    if let Some(rows) = slots {
        for slot in rows.iter().take(limit_slots) {
            let slot_id = scalar(&slot["slot"]);
            let name = scalar(&slot["name"]);
            let n_regions = scalar(&slot["n_regions"]);
            let dim = scalar(&slot["dim"]);
            let query_start_row = scalar(&slot["query_start_row"]);
            out.push(format!(
                "slot slot={} name={} dim={} n_regions={} query_start_row={}",
                slot_id, name, dim, n_regions, query_start_row
            ));
        }
    }
}

fn render_plan(out: &mut Vec<String>, report: &Value) {
    let plan = &report["plan_source"];
    push_value(out, plan, "mode", "plan_source_mode");
    push_value(out, plan, "cf_root", "plan_cf_root");
    push_value(out, plan, "association_key", "plan_key");
    push_nested_value(
        out,
        plan,
        &["db_readback", "readback_matches"],
        "plan_db_readback_matches",
    );
    push_nested_value(
        out,
        plan,
        &["db_readback", "value_sha256"],
        "plan_value_sha256",
    );
}

fn render_ground_truth(out: &mut Vec<String>, report: &Value) {
    let truth = &report["ground_truth_source"];
    push_value(out, truth, "mode", "ground_truth_source_mode");
    push_value(out, truth, "scale_suitable", "ground_truth_scale_suitable");
    push_value(out, truth, "query_count", "ground_truth_source_queries");
    push_value(out, truth, "truth_depth", "ground_truth_source_depth");
    push_nested_value(
        out,
        truth,
        &["db_readback", "readback_matches"],
        "ground_truth_db_readback_matches",
    );
    push_nested_value(
        out,
        truth,
        &["db_readback", "value_bytes"],
        "ground_truth_value_bytes",
    );
    push_nested_value(
        out,
        truth,
        &["db_readback", "value_sha256"],
        "ground_truth_value_sha256",
    );
}

fn render_a37(out: &mut Vec<String>, report: &Value) {
    let a37 = &report["a37_admission"];
    push_value(out, a37, "mode", "a37_mode");
    push_value(out, a37, "status", "a37_status");
    push_value(out, a37, "gate_passed", "a37_gate_passed");
    push_value(out, a37, "lens_count", "a37_lens_count");
    push_value(
        out,
        a37,
        "association_family_count",
        "a37_association_family_count",
    );
    push_nested_value(
        out,
        a37,
        &["db_readback", "readback_matches"],
        "a37_db_readback_matches",
    );
    push_nested_value(
        out,
        a37,
        &["db_readback", "value_sha256"],
        "a37_value_sha256",
    );
}

fn render_temporal(out: &mut Vec<String>, report: &Value) {
    let temporal = &report["temporal"];
    push_value(out, temporal, "mode", "temporal_mode");
    push_value(out, temporal, "row_count", "temporal_row_count");
    push_value(out, temporal, "active_count", "temporal_active_count");
    push_value(out, temporal, "inactive_count", "temporal_inactive_count");
    push_value(
        out,
        temporal,
        "duplicate_event_time_rows",
        "temporal_duplicate_event_time_rows",
    );
    push_value(
        out,
        temporal,
        "out_of_order_event_time_rows",
        "temporal_out_of_order_event_time_rows",
    );
    push_nested_value(
        out,
        temporal,
        &["db_readback", "readback_matches"],
        "temporal_db_readback_matches",
    );
    push_nested_value(
        out,
        temporal,
        &["db_readback", "value_sha256"],
        "temporal_value_sha256",
    );
}

fn push_value(out: &mut Vec<String>, root: &Value, key: &str, label: &str) {
    let value = &root[key];
    if !value.is_null() {
        push(out, label, scalar(value));
    }
}

fn push_nested_value(out: &mut Vec<String>, root: &Value, path: &[&str], label: &str) {
    let mut value = root;
    for key in path {
        value = &value[*key];
    }
    if !value.is_null() {
        push(out, label, scalar(value));
    }
}

fn push(out: &mut Vec<String>, key: &str, value: impl std::fmt::Display) {
    out.push(format!("{key}={}", one_line(&value.to_string())));
}

fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(values) => format!("array_len:{}", values.len()),
        Value::Object(values) => format!("object_keys:{}", values.len()),
    }
}

fn one_line(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

fn parse_positive(value: &str, flag: &str) -> CliResult<usize> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| CliError::usage(format!("{flag} must be an unsigned integer")))?;
    if parsed == 0 {
        return Err(CliError::usage(format!("{flag} must be > 0")));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::*;

    #[test]
    fn report_readback_renders_bounded_scalar_lines() {
        let root = temp_root("partitioned-rrf-report-readback");
        let report = sample_report();
        let written = partitioned_rrf_report_store::write(&root, "unit-report", &report).unwrap();
        let (record, loaded) = partitioned_rrf_report_store::read(&root, "unit-report").unwrap();
        let args = Args {
            cf_root: root.clone(),
            report_key: "unit-report".to_string(),
            limit_slots: 1,
        };

        let rendered = render(&record.report, &record.format, &record.mode, &loaded, &args);
        let text = rendered.join("\n");

        assert_eq!(written.value_sha256, loaded.value_sha256);
        assert!(text.contains("partitioned_rrf_report_readback\n"));
        assert!(text.contains("readback_matches=true\n"));
        assert!(text.contains("recall_at_k=0.91\n"));
        assert!(text.contains("latency_p99_us=24000\n"));
        assert!(text.contains("lens_roster_len=2\n"));
        assert!(text.contains("slots_shown=1\n"));
        assert!(text.contains("slot slot=0 name=semantic dim=768 n_regions=12 query_start_row=9"));
        assert!(text.contains("a37_gate_passed=true\n"));
        assert!(text.contains("temporal_active_count=1000\n"));
        assert!(!text.contains('{'));
        assert!(!text.contains('['));
        let _ = fs::remove_dir_all(root);
    }

    fn sample_report() -> Value {
        json!({
            "trigger": "calyx bench partitioned-rrf",
            "mode": "real_multi_slot_rrf",
            "metric_class": "ann_correctness",
            "metric_scope": "multi_slot_rrf",
            "plan_source": {
                "mode": "aster_graph_cf",
                "cf_root": "/tmp/plan-cf",
                "association_key": "plan",
                "db_readback": {
                    "readback_matches": true,
                    "value_sha256": "a".repeat(64)
                }
            },
            "lens_roster": [{"slot": 0}, {"slot": 1}],
            "per_lens_bits": [{"slot": 0}, {"slot": 1}],
            "slots": [
                {"slot": 0, "name": "semantic", "dim": 768, "n_regions": 12, "query_start_row": 9},
                {"slot": 1, "name": "lexical", "dim": 512, "n_regions": 9, "query_start_row": 9}
            ],
            "queries": 1000,
            "k": 10,
            "n_probe": 64,
            "region_beam": 1152,
            "truth_depth": 64,
            "ground_truth_queries": 1000,
            "ground_truth_source": {
                "mode": "precomputed_slot_rrf_aster_cf",
                "scale_suitable": true,
                "query_count": 1000,
                "truth_depth": 64,
                "db_readback": {
                    "readback_matches": true,
                    "value_bytes": 1234,
                    "value_sha256": "b".repeat(64)
                }
            },
            "latency_us": {"p50": 18000, "p99": 24000, "p999": 25000, "max": 70000},
            "fused_ground_truth_recall_at_k": 0.91,
            "recall_floor": 0.85,
            "a37_admission": {
                "mode": "assay_multi_anchor_a37_admission_db",
                "status": "gate_passed",
                "gate_passed": true,
                "lens_count": 10,
                "association_family_count": 4,
                "db_readback": {
                    "readback_matches": true,
                    "value_sha256": "c".repeat(64)
                }
            },
            "temporal": {
                "mode": "aster_graph_cf",
                "row_count": 1000,
                "active_count": 1000,
                "inactive_count": 0,
                "duplicate_event_time_rows": 0,
                "out_of_order_event_time_rows": 0,
                "db_readback": {
                    "readback_matches": true,
                    "value_sha256": "d".repeat(64)
                }
            },
            "best_single_lens_recall_vs_fused_truth": 0.82,
            "fusion_matches_or_beats_best_single": true
        })
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
