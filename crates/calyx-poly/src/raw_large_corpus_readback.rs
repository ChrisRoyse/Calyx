use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::raw_large_corpus::{LargeCorpusFailure, LargeCorpusManifest, LargeCorpusReadbackReport};
use crate::raw_large_corpus_onchain_backfill_readback::check_onchain_backfill_state_artifact;
use crate::raw_large_corpus_pagination_readback::{
    check_pagination_chains, check_pagination_state,
};
use crate::raw_large_corpus_readback_range::check_range_request;
use crate::raw_large_corpus_support::{
    bounded_incomplete_datasets, capture_goal, exhaustive_incomplete_failure, failure,
};
use crate::raw_large_corpus_trade_history::check_trade_history_source_state_artifact;
use crate::raw_large_corpus_ws_semantics::check_websocket_runtime_semantics_artifact;
use crate::raw_source_support::{sha256_hex, write_json};
use crate::{PolyError, Result};

pub fn read_large_corpus_manifest(path: &Path) -> Result<LargeCorpusManifest> {
    let bytes = fs::read(path).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_MANIFEST_READ_FAILED",
            format!("read large corpus manifest {}: {err}", path.display()),
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_MANIFEST_DECODE_FAILED",
            format!("decode large corpus manifest {}: {err}", path.display()),
        )
    })
}

pub fn readback_large_corpus(root: &Path) -> Result<LargeCorpusReadbackReport> {
    readback_large_corpus_with_exhaustive(root, None)
}

pub fn readback_large_corpus_with_exhaustive(
    root: &Path,
    require_exhaustive_override: Option<bool>,
) -> Result<LargeCorpusReadbackReport> {
    let manifest_path = root.join("large-corpus-manifest.json");
    let mut manifest = read_large_corpus_manifest(&manifest_path)?;
    if let Some(require_exhaustive) = require_exhaustive_override {
        manifest.require_exhaustive = require_exhaustive;
        manifest.capture_goal = capture_goal(require_exhaustive).to_string();
    }
    if manifest.capture_goal.is_empty() {
        manifest.capture_goal = capture_goal(manifest.require_exhaustive).to_string();
    }
    manifest.bounded_incomplete_datasets = bounded_incomplete_datasets(
        &manifest.pages,
        manifest.page_size,
        manifest.max_pages_per_dataset,
    );
    let mut report = LargeCorpusReadbackReport {
        schema_version: "poly.large_corpus.readback.v1".to_string(),
        manifest_path: manifest_path.display().to_string(),
        capture_goal: manifest.capture_goal.clone(),
        require_exhaustive: manifest.require_exhaustive,
        bounded_incomplete_datasets: manifest.bounded_incomplete_datasets.clone(),
        trade_history_state_path: manifest.trade_history_state_path.clone(),
        onchain_backfill_state_path: manifest.onchain_backfill_state_path.clone(),
        checked_file_count: 0,
        missing_files: Vec::new(),
        sha_mismatches: Vec::new(),
        parse_failures: Vec::new(),
        total_pages: manifest.total_pages,
        total_records: manifest.total_records,
        total_body_bytes: manifest.total_body_bytes,
        edge_case_count: manifest.edge_cases.len(),
        passed: false,
        status_code: String::new(),
        failure: None,
    };
    check_pages(&manifest, &mut report);
    check_required_artifacts(root, &manifest, &mut report);
    let failure = readback_failure(&report).or_else(|| manifest_failure(&manifest));
    report.status_code = failure
        .as_ref()
        .map(|failure| failure.code.clone())
        .unwrap_or_else(|| "POLY_LARGE_CORPUS_READBACK_PASSED".to_string());
    report.passed = failure.is_none();
    report.failure = failure;
    write_json(&root.join("large-corpus-readback-report.json"), &report)?;
    Ok(report)
}

pub fn require_large_corpus_passed(report: &LargeCorpusReadbackReport) -> Result<()> {
    if report.passed {
        Ok(())
    } else {
        let failure = report.failure.as_ref().ok_or_else(|| {
            PolyError::raw_source(
                "POLY_LARGE_CORPUS_READBACK_FAILED",
                "large corpus readback failed without a structured failure",
            )
        })?;
        Err(PolyError::raw_source(
            failure.code.clone(),
            failure.message.clone(),
        ))
    }
}

fn check_pages(manifest: &LargeCorpusManifest, report: &mut LargeCorpusReadbackReport) {
    for page in &manifest.pages {
        if let Some(path) = &page.request_path {
            check_body(path, &page.request_body_sha256, true, report);
        }
        if let Some(range_state) = &page.range_state {
            check_range_request(&page.request_path, range_state, report);
        }
        check_pagination_state(page, &mut report.parse_failures);
        check_body_format(
            &page.body_path,
            &page.body_sha256,
            &page.body_format,
            report,
        );
        check_body(&page.metadata_path, &None, true, report);
    }
    check_pagination_chains(&manifest.pages, &mut report.parse_failures);
    for edge in &manifest.edge_cases {
        if let Some(path) = &edge.request_path {
            check_body(path, &edge.request_body_sha256, true, report);
        }
        if let Some(range_state) = &edge.range_state {
            check_range_request(&edge.request_path, range_state, report);
        }
        if edge.expected_semantics == "expected_format_failure" {
            check_expected_format_failure(
                &edge.body_path,
                &edge.body_sha256,
                &edge.body_format,
                report,
            );
        } else {
            check_body_format(
                &edge.body_path,
                &edge.body_sha256,
                &edge.body_format,
                report,
            );
        }
        check_body(&edge.metadata_path, &None, true, report);
    }
}

fn check_required_artifacts(
    root: &Path,
    manifest: &LargeCorpusManifest,
    report: &mut LargeCorpusReadbackReport,
) {
    for path in &manifest.field_profile_paths {
        check_body(path, &None, true, report);
    }
    check_body(&manifest.join_profile_path, &None, true, report);
    check_body(&manifest.schema_decision_input_path, &None, false, report);
    check_trade_history_source_state_artifact(manifest, report);
    check_onchain_backfill_state_artifact(manifest, report);
    check_websocket_runtime_semantics_artifact(root, report);
}

fn check_body(
    path: &str,
    expected_sha: &Option<String>,
    expect_json: bool,
    report: &mut LargeCorpusReadbackReport,
) {
    report.checked_file_count += 1;
    let path_obj = Path::new(path);
    let bytes = match fs::read(path_obj) {
        Ok(bytes) => bytes,
        Err(err) => {
            report
                .missing_files
                .push(format!("{}: {err}", path_obj.display()));
            return;
        }
    };
    if let Some(expected) = expected_sha {
        let actual = sha256_hex(&bytes);
        if &actual != expected {
            report.sha_mismatches.push(format!(
                "{} expected {} actual {}",
                path_obj.display(),
                expected,
                actual
            ));
        }
    }
    if expect_json && serde_json::from_slice::<Value>(&bytes).is_err() {
        report.parse_failures.push(path_obj.display().to_string());
    }
}

fn check_body_format(
    path: &str,
    expected_sha: &Option<String>,
    format: &str,
    report: &mut LargeCorpusReadbackReport,
) {
    report.checked_file_count += 1;
    let path_obj = Path::new(path);
    let bytes = match fs::read(path_obj) {
        Ok(bytes) => bytes,
        Err(err) => {
            report
                .missing_files
                .push(format!("{}: {err}", path_obj.display()));
            return;
        }
    };
    if let Some(expected) = expected_sha {
        let actual = sha256_hex(&bytes);
        if &actual != expected {
            report.sha_mismatches.push(format!(
                "{} expected {} actual {}",
                path_obj.display(),
                expected,
                actual
            ));
        }
    }
    let parse_ok = match format {
        "json" => serde_json::from_slice::<Value>(&bytes).is_ok(),
        "jsonl" => validate_jsonl(&bytes),
        "text" | "binary" => !bytes.is_empty(),
        _ => false,
    };
    if !parse_ok {
        report
            .parse_failures
            .push(format!("{} format={format}", path_obj.display()));
    }
}

fn validate_jsonl(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut count = 0usize;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        if serde_json::from_str::<Value>(line).is_err() {
            return false;
        }
        count += 1;
    }
    count > 0
}

fn check_expected_format_failure(
    path: &str,
    expected_sha: &Option<String>,
    format: &str,
    report: &mut LargeCorpusReadbackReport,
) {
    report.checked_file_count += 1;
    let path_obj = Path::new(path);
    let bytes = match fs::read(path_obj) {
        Ok(bytes) => bytes,
        Err(err) => {
            report
                .missing_files
                .push(format!("{}: {err}", path_obj.display()));
            return;
        }
    };
    if let Some(expected) = expected_sha {
        let actual = sha256_hex(&bytes);
        if &actual != expected {
            report.sha_mismatches.push(format!(
                "{} expected {} actual {}",
                path_obj.display(),
                expected,
                actual
            ));
        }
    }
    let failed_as_expected = match format {
        "json" => serde_json::from_slice::<Value>(&bytes).is_err(),
        "jsonl" => !validate_jsonl(&bytes),
        _ => !bytes.is_empty(),
    };
    if !failed_as_expected {
        report.parse_failures.push(format!(
            "{} did not fail expected format={format}",
            path_obj.display()
        ));
    }
}

fn readback_failure(report: &LargeCorpusReadbackReport) -> Option<LargeCorpusFailure> {
    if !report.missing_files.is_empty() {
        return Some(failure(
            "POLY_LARGE_CORPUS_READBACK_FILE_MISSING",
            format!(
                "{} large corpus files are missing",
                report.missing_files.len()
            ),
        ));
    }
    if !report.sha_mismatches.is_empty() {
        return Some(failure(
            "POLY_LARGE_CORPUS_READBACK_SHA_MISMATCH",
            format!(
                "{} large corpus files failed SHA256 readback",
                report.sha_mismatches.len()
            ),
        ));
    }
    if !report.parse_failures.is_empty() {
        return Some(failure(
            "POLY_LARGE_CORPUS_READBACK_PARSE_FAILED",
            format!(
                "{} large corpus JSON files failed readback parsing",
                report.parse_failures.len()
            ),
        ));
    }
    None
}

fn manifest_failure(manifest: &LargeCorpusManifest) -> Option<LargeCorpusFailure> {
    if let Some(failure) = exhaustive_incomplete_failure(
        manifest.require_exhaustive,
        &manifest.bounded_incomplete_datasets,
    ) {
        return Some(failure);
    }
    if manifest.passed {
        None
    } else {
        Some(failure(
            manifest.status_code.clone(),
            manifest
                .failure
                .as_ref()
                .map(|failure| failure.message.clone())
                .unwrap_or_else(|| "large corpus manifest failed".to_string()),
        ))
    }
}
