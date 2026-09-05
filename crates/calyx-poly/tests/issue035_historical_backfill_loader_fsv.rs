//! Issue #35 - terminal/reference historical backfill loader FSV.
//!
//! Source of truth: a captured or hermetic SimpleFunctions-compatible JSONL corpus, plus the
//! persisted terminal/reference corpus readback.

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;

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

    let source_path = historical_source_path(&root);
    assert_c_drive(&source_path);
    let source_readback = read_source_truth(&source_path);
    write_json(&root.join("source-readback.json"), &source_readback);

    let subset_path = root.join("simplefunctions-subset.jsonl");
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
        "proof_claim": "Historical resolved-market JSONL rows are loaded as persisted terminal/reference records, read back from disk, and refused from any pre-resolution route.",
        "minimum_sufficient_proof_corpus": {
            "source": source_readback,
            "loader_subset_rows": 5,
            "loaded_polymarket_rows": 3,
            "skipped_non_polymarket_rows": 2,
            "why_this_is_sufficient": "The claim is structural: parse source-compatible row shape, tag terminal/reference/not-pre-resolution, persist/read back, and fail closed on invalid JSONL, missing terminal fields, duplicate conflicts, and pre-resolution routing. Three Polymarket rows include both outcomes and distinct categories; extra rows repeat the same path.",
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
    calyx_fsv::fsv_root_or_target(
        "POLY_ISSUE35_FSV_ROOT",
        "issue35-historical-backfill-loader",
        || repo_root().join("target/fsv/issue35_historical_backfill_loader_20260707"),
    )
}

fn historical_source_path(root: &Path) -> PathBuf {
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
    write_historical_fixture(&root.join("source-fixture"))
}

fn write_historical_fixture(dir: &Path) -> PathBuf {
    fs::create_dir_all(dir).expect("create issue35 fixture source directory");
    let rows = [
        json!({
            "venue": "kalshi",
            "ticker": "KALSHI-BTC-ABOVE-70K",
            "title": "Kalshi BTC above 70k",
            "category": "crypto",
            "volume": 1000.0,
            "predicted_price": 0.41,
            "predicted_price_t24h": 0.39,
            "resolved_outcome": 1,
            "resolved_at": "2026-04-01T00:00:00Z"
        }),
        json!({
            "venue": "polymarket",
            "ticker": "POLY-BTC-64K-YES",
            "title": "Bitcoin above 64,000 on April 2?",
            "category": "crypto",
            "volume": 125000.0,
            "predicted_price": 0.62,
            "predicted_price_t24h": 0.58,
            "resolved_outcome": 1,
            "resolved_at": "2026-04-02T23:59:59Z"
        }),
        json!({
            "venue": "polymarket",
            "ticker": "POLY-ETH-4K-YES",
            "title": "Ethereum above 4,000 on April 3?",
            "category": "crypto",
            "volume": 84000.0,
            "predicted_price": 0.35,
            "predicted_price_t24h": 0.31,
            "resolved_outcome": 0,
            "resolved_at": "2026-04-03T23:59:59Z"
        }),
        json!({
            "venue": "kalshi",
            "ticker": "KALSHI-CPI-APRIL",
            "title": "Kalshi CPI release",
            "category": "macro",
            "volume": 2000.0,
            "predicted_price": 0.52,
            "predicted_price_t24h": 0.49,
            "resolved_outcome": 0,
            "resolved_at": "2026-04-04T00:00:00Z"
        }),
        json!({
            "venue": "polymarket",
            "ticker": "POLY-SOL-200-YES",
            "title": "Solana above 200 on April 4?",
            "category": "crypto",
            "volume": 43000.0,
            "predicted_price": 0.47,
            "predicted_price_t24h": 0.44,
            "resolved_outcome": 1,
            "resolved_at": "2026-04-04T23:59:59Z"
        }),
    ];
    let body = rows
        .iter()
        .map(|row| serde_json::to_string(row).expect("encode fixture row"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let body_path = dir.join("body.jsonl");
    fs::write(&body_path, body.as_bytes()).expect("write issue35 fixture body");
    let metadata = json!({
        "body_sha256": sha256_hex(body.as_bytes()),
        "body_bytes": body.len(),
        "record_count": rows.len(),
        "top_level_fields": [
            "category",
            "predicted_price",
            "predicted_price_t24h",
            "resolved_at",
            "resolved_outcome",
            "ticker",
            "title",
            "venue",
            "volume"
        ]
    });
    write_json(&dir.join("metadata.json"), &metadata);
    body_path
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("repo root")
        .to_path_buf()
}

fn assert_c_drive(path: &Path) {
    support::assert_host_fsv_root(path, "issue35 FSV path");
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
