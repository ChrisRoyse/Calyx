//! Issue #35 - terminal/reference historical backfill loader FSV.
//!
//! Source of truth: the physical Hugging Face SimpleFunctions JSONL raw corpus already captured
//! under `C:\code\poly\target\fsv`, plus the persisted terminal/reference corpus readback.

#[path = "fsv_support.rs"]
#[allow(dead_code)]
mod support;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_poly::historical_backfill_loader::{
    ERR_HISTORICAL_BACKFILL_DUPLICATE, ERR_HISTORICAL_BACKFILL_INVALID_ROW,
    ERR_HISTORICAL_BACKFILL_JSONL, ERR_HISTORICAL_BACKFILL_ROUTE_FORBIDDEN,
    ensure_historical_record_pre_resolution_eligible, load_historical_terminal_reference_corpus,
    read_historical_terminal_reference_corpus,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use support::{collect_files, reset_dir, write_blake3sums, write_json};

const SOURCE_URL: &str =
    "https://huggingface.co/datasets/SimpleFunctions/settled-markets/resolve/main/2026-04.jsonl";
const SOURCE_DATASET: &str = "SimpleFunctions/settled-markets/2026-04.jsonl";

#[test]
fn issue035_historical_backfill_loader_fsv() {
    let root = issue35_root();
    assert_c_drive(&root);
    reset_dir(&root);

    let source_path = historical_source_path();
    assert_c_drive(&source_path);
    let source_readback = read_source_truth(&source_path);
    write_json(&root.join("source-readback.json"), &source_readback);

    let subset_path = root.join("real-simplefunctions-subset.jsonl");
    let subset = write_real_subset(&source_path, &subset_path);
    assert_eq!(subset.polymarket_lines.len(), 3);
    assert_eq!(subset.non_polymarket_lines, 2);
    assert!(
        subset.outcomes.contains(&0) && subset.outcomes.contains(&1),
        "subset must include both resolved outcomes"
    );

    let report = load_historical_terminal_reference_corpus(
        &subset_path,
        &root.join("loader"),
        SOURCE_DATASET,
        SOURCE_URL,
        3,
    )
    .expect("load real historical terminal subset");
    assert_eq!(report.rows_loaded, 3);
    assert_eq!(report.skipped_non_polymarket, 2);
    assert!(!report.truncated_by_limit);
    assert!(report.readback_matched);
    assert!(report.corpus.records.iter().all(|record| {
        record.terminal && record.reference_only && !record.pre_resolution_eligible
    }));

    let persisted = read_historical_terminal_reference_corpus(Path::new(&report.corpus_path))
        .expect("read persisted terminal corpus");
    assert_eq!(persisted, report.corpus);

    let route_error = ensure_historical_record_pre_resolution_eligible(&report.corpus.records[0])
        .expect_err("terminal historical rows must not enter pre-resolution corpus");
    assert_eq!(route_error.code(), ERR_HISTORICAL_BACKFILL_ROUTE_FORBIDDEN);

    let edge_cases = edge_cases_fail_closed(&root, &subset.polymarket_lines[0], &route_error);
    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let final_report = json!({
        "issue": 35,
        "proof_claim": "Real historical resolved-market JSONL rows are loaded as persisted terminal/reference records, read back from disk, and refused from any pre-resolution route.",
        "minimum_sufficient_proof_corpus": {
            "source": source_readback,
            "loader_subset_rows": 5,
            "loaded_polymarket_rows": 3,
            "skipped_non_polymarket_rows": 2,
            "why_this_is_sufficient": "The claim is structural: parse real row shape, tag terminal/reference/not-pre-resolution, persist/read back, and fail closed on invalid JSONL, missing terminal fields, duplicate conflicts, and pre-resolution routing. Three real Polymarket rows include both outcomes and distinct categories; extra rows repeat the same path.",
            "why_smaller_is_insufficient": "Fewer than three loaded rows would not cover both resolved outcomes plus multiple observed Polymarket categories while also proving mixed-venue skip behavior.",
            "why_larger_is_wasteful": "The full 72,864-row file proves scale/full-source coverage, not additional correctness for these loader and leak-prevention invariants."
        },
        "loader_report": serde_json::to_value(&report).expect("report JSON"),
        "route_refusal": route_error.diagnostic(),
        "edge_cases": edge_cases,
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue035_historical_backfill_loader_fsv_report.json");
    write_json(&report_path, &final_report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback, final_report);
    write_blake3sums(&root);
}

fn edge_cases_fail_closed(
    root: &Path,
    real_polymarket_line: &str,
    route_error: &calyx_poly::PolyError,
) -> Vec<Value> {
    let edge_root = root.join("edge-cases");
    fs::create_dir_all(&edge_root).expect("create edge root");
    let malformed = edge_root.join("malformed.jsonl");
    fs::write(&malformed, "{not-json}\n").expect("write malformed edge");
    let malformed_err = expect_loader_code(
        &malformed,
        &edge_root.join("malformed-out"),
        ERR_HISTORICAL_BACKFILL_JSONL,
    );

    let mut missing: Value =
        serde_json::from_str(real_polymarket_line).expect("parse real Polymarket row");
    missing
        .as_object_mut()
        .expect("row object")
        .remove("resolved_outcome");
    let missing_path = edge_root.join("missing-outcome.jsonl");
    fs::write(
        &missing_path,
        format!(
            "{}\n",
            serde_json::to_string(&missing).expect("encode missing")
        ),
    )
    .expect("write missing field edge");
    let missing_err = expect_loader_code(
        &missing_path,
        &edge_root.join("missing-outcome-out"),
        ERR_HISTORICAL_BACKFILL_INVALID_ROW,
    );

    let dup_a: Value = serde_json::from_str(real_polymarket_line).expect("parse duplicate row a");
    let mut dup_b = dup_a.clone();
    let original_outcome = dup_a["resolved_outcome"].as_u64().expect("real outcome") as u8;
    dup_b["resolved_outcome"] = json!(if original_outcome == 0 { 1 } else { 0 });
    let duplicate_path = edge_root.join("duplicate-conflict.jsonl");
    fs::write(
        &duplicate_path,
        format!(
            "{}\n{}\n",
            serde_json::to_string(&dup_a).expect("encode dup a"),
            serde_json::to_string(&dup_b).expect("encode dup b")
        ),
    )
    .expect("write duplicate edge");
    let duplicate_err = expect_loader_code(
        &duplicate_path,
        &edge_root.join("duplicate-conflict-out"),
        ERR_HISTORICAL_BACKFILL_DUPLICATE,
    );

    vec![
        json!({"case": "malformed_jsonl", "before": {"path": malformed.display().to_string()}, "after": malformed_err}),
        json!({"case": "missing_resolved_outcome", "before": {"path": missing_path.display().to_string()}, "after": missing_err}),
        json!({"case": "duplicate_ticker_conflict", "before": {"path": duplicate_path.display().to_string()}, "after": duplicate_err}),
        json!({"case": "terminal_route_forbidden", "after": route_error.diagnostic()}),
    ]
}

fn expect_loader_code(path: &Path, out: &Path, expected: &str) -> Value {
    match load_historical_terminal_reference_corpus(path, out, SOURCE_DATASET, SOURCE_URL, 10) {
        Ok(report) => panic!("expected {expected}, got success: {report:?}"),
        Err(err) => {
            assert_eq!(err.code(), expected);
            json!(err.diagnostic())
        }
    }
}

struct RealSubset {
    polymarket_lines: Vec<String>,
    non_polymarket_lines: usize,
    outcomes: BTreeSet<u8>,
}

fn write_real_subset(source_path: &Path, subset_path: &Path) -> RealSubset {
    let text = fs::read_to_string(source_path).expect("read real historical JSONL source");
    let mut lines = Vec::new();
    let mut non_polymarket = 0usize;
    let mut polymarket_lines = Vec::new();
    let mut outcomes = BTreeSet::new();
    for line in text.lines() {
        let value: Value = serde_json::from_str(line).expect("real source JSON line");
        let venue = value["venue"].as_str().unwrap_or_default();
        if venue == "kalshi" && non_polymarket < 2 {
            lines.push(line.to_string());
            non_polymarket += 1;
            continue;
        }
        if venue == "polymarket" && polymarket_lines.len() < 3 {
            let outcome = value["resolved_outcome"].as_u64().expect("real outcome") as u8;
            outcomes.insert(outcome);
            polymarket_lines.push(line.to_string());
            lines.push(line.to_string());
        }
        if non_polymarket == 2 && polymarket_lines.len() == 3 && outcomes.len() == 2 {
            break;
        }
    }
    assert_eq!(non_polymarket, 2, "expected real non-Polymarket rows");
    assert_eq!(polymarket_lines.len(), 3, "expected real Polymarket rows");
    fs::write(subset_path, format!("{}\n", lines.join("\n"))).expect("write real subset");
    RealSubset {
        polymarket_lines,
        non_polymarket_lines: non_polymarket,
        outcomes,
    }
}

fn read_source_truth(source_path: &Path) -> Value {
    let bytes = fs::read(source_path).expect("read source truth");
    let sha = sha256_hex(&bytes);
    let metadata_path = source_path.with_file_name("metadata.json");
    let metadata: Value =
        serde_json::from_slice(&fs::read(&metadata_path).expect("read metadata")).expect("decode");
    assert_eq!(metadata["body_sha256"], sha);
    assert_eq!(metadata["body_bytes"], json!(bytes.len()));
    json!({
        "source_path": source_path.display().to_string(),
        "metadata_path": metadata_path.display().to_string(),
        "source_bytes": bytes.len(),
        "source_sha256": sha,
        "metadata_body_sha256": metadata["body_sha256"],
        "metadata_record_count": metadata["record_count"],
        "metadata_top_level_fields": metadata["top_level_fields"]
    })
}

fn issue35_root() -> PathBuf {
    if let Some(path) = std::env::var_os("POLY_ISSUE35_FSV_ROOT") {
        return PathBuf::from(path);
    }
    repo_root().join("target/fsv/issue35_historical_backfill_loader_20260707")
}

fn historical_source_path() -> PathBuf {
    if let Some(path) = std::env::var_os("POLY_ISSUE35_HISTORICAL_JSONL") {
        return PathBuf::from(path);
    }
    let relative = Path::new(
        "target/fsv/polymarket_raw_source_inventory/raw/historical-dump/\
         hf_simplefunctions_settled_markets_2026_04_jsonl/body.jsonl",
    );
    let local = repo_root().join(relative);
    if local.exists() {
        return local;
    }
    let poly_capture = PathBuf::from(r"C:\code\poly").join(relative);
    if poly_capture.exists() {
        return poly_capture;
    }
    local
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("repo root")
        .to_path_buf()
}

fn assert_c_drive(path: &Path) {
    #[cfg(windows)]
    assert!(
        path.to_string_lossy().starts_with("C:"),
        "{} must stay on C:",
        path.display()
    );
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
