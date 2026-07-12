use std::path::PathBuf;

use serde_json::Value;

use crate::error::{CliError, CliResult};
use crate::output;
use crate::partitioned_rrf_report_store::{self, PartitionedRrfReportDbReadback};

const DEFAULT_LIMIT_SLOTS: usize = 12;
const DEFAULT_LIMIT_PAIRS: usize = 12;

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
    limit_pairs: usize,
}

impl Args {
    fn parse(raw: &[String]) -> CliResult<Self> {
        let mut cf_root = None;
        let mut report_key = partitioned_rrf_report_store::DEFAULT_ASSOCIATION_KEY.to_string();
        let mut limit_slots = DEFAULT_LIMIT_SLOTS;
        let mut limit_pairs = DEFAULT_LIMIT_PAIRS;
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
                "--limit-pairs" => {
                    limit_pairs = parse_positive(&next()?, "--limit-pairs")?;
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
            limit_pairs,
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
    render_ensemble(&mut out, report, args.limit_pairs);
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

fn render_ensemble(out: &mut Vec<String>, report: &Value, limit_pairs: usize) {
    let ensemble = &report["ensemble_decomposition"];
    if ensemble.is_null() {
        return;
    }
    push_value(out, ensemble, "mode", "ensemble_mode");
    let method = &ensemble["redundancy_method"];
    for (key, label) in [
        ("metric", "redundancy_metric"),
        ("tuple_design", "redundancy_tuple_design"),
        ("row_count", "redundancy_row_count"),
        ("tuple_count", "redundancy_tuple_count"),
        ("seed_hex", "redundancy_seed_hex"),
        ("tuple_plan_blake3", "redundancy_tuple_plan_blake3"),
        ("exact", "redundancy_exact"),
        ("uncertainty_method", "redundancy_uncertainty_method"),
        ("uncertainty_blocks", "redundancy_uncertainty_blocks"),
        ("gate_score_method", "redundancy_gate_score_method"),
    ] {
        push_value(out, method, key, label);
    }
    let pairs = ensemble["pair_values"].as_array();
    push(out, "pair_values_total", pairs.map_or(0, Vec::len));
    push(
        out,
        "pair_values_shown",
        pairs.map_or(0, |rows| rows.len().min(limit_pairs)),
    );
    if let Some(rows) = pairs {
        for pair in rows.iter().take(limit_pairs) {
            let redundancy = &pair["redundancy"];
            out.push(format!(
                "pair slot_a={} slot_b={} a={} b={} corr={} nmi={} raw_signed_point={} redundancy_point={} mc_standard_error={} mc_gate_upper_estimate={}",
                scalar(&pair["slot_a"]),
                scalar(&pair["slot_b"]),
                scalar(&pair["a"]),
                scalar(&pair["b"]),
                scalar(&pair["corr"]),
                scalar(&pair["nmi"]),
                scalar(&redundancy["raw_signed_point"]),
                scalar(&redundancy["redundancy_point"]),
                scalar(&redundancy["mc_standard_error"]),
                scalar(&redundancy["mc_gate_upper_estimate"]),
            ));
        }
    }
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
        Value::String(value) => one_line(value),
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
#[path = "report_readback/tests.rs"]
mod tests;
