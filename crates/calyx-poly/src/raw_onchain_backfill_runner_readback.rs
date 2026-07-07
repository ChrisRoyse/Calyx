use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::raw_large_corpus::LargeCorpusPage;
use crate::raw_onchain_backfill_readback_checks::{
    canonical_display_path, canonical_root, check_checkpoint_contract_summary,
    check_checkpoint_structure, check_chunk_policy, check_page_range_policy,
    metadata_path_for_body, require_checkpoint_range_paths_under_root,
    require_page_paths_under_root,
};
use crate::raw_onchain_backfill_readback_context::{
    CachedArtifact, ReadbackArtifactKind, ReadbackContext,
};
pub use crate::raw_onchain_backfill_readback_scope::{
    ONCHAIN_BACKFILL_CURRENT_RUN_READBACK_FILE,
    ONCHAIN_BACKFILL_CURRENT_RUN_READBACK_PROGRESS_FILE, ONCHAIN_BACKFILL_READBACK_FILE,
    ONCHAIN_BACKFILL_READBACK_PROGRESS_FILE,
};
use crate::raw_onchain_backfill_readback_scope::{
    OnchainBackfillReadbackScope, check_current_run_pages_in_checkpoint, scoped_readback_files,
};
use crate::raw_onchain_backfill_runner_types::{
    ONCHAIN_BACKFILL_RUN_CHECKPOINT_FILE, ONCHAIN_BACKFILL_RUN_PASSED,
    ONCHAIN_BACKFILL_RUN_REPORT_FILE, ONCHAIN_BACKFILL_RUN_SCHEMA_VERSION,
    OnchainBackfillCheckpoint, OnchainBackfillRunReport,
};
use crate::raw_source_support::{sha256_hex, write_json};
use crate::{PolyError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnchainBackfillReadbackReport {
    pub schema_version: String,
    pub source_of_truth: String,
    pub status_code: String,
    pub output_root: String,
    pub run_report_path: String,
    pub checkpoint_path: String,
    #[serde(default)]
    pub readback_report_path: String,
    #[serde(default)]
    pub readback_scope: OnchainBackfillReadbackScope,
    #[serde(default)]
    pub readback_progress_path: String,
    pub checked_file_count: usize,
    #[serde(default)]
    pub unique_file_read_count: usize,
    #[serde(default)]
    pub deduplicated_file_read_count: usize,
    #[serde(default)]
    pub json_parse_count: usize,
    #[serde(default)]
    pub readback_bytes_read: u64,
    #[serde(default)]
    pub readback_body_bytes_read: u64,
    #[serde(default)]
    pub readback_request_bytes_read: u64,
    #[serde(default)]
    pub readback_metadata_bytes_read: u64,
    #[serde(default)]
    pub progress_event_count: usize,
    pub missing_files: Vec<String>,
    pub sha_mismatches: Vec<String>,
    pub parse_failures: Vec<String>,
    pub total_pages: usize,
    #[serde(default)]
    pub current_run_page_count: usize,
    #[serde(default)]
    pub checkpoint_range_count: usize,
    pub total_records: usize,
    pub total_body_bytes: u64,
    pub all_order_filled_backfill_complete: bool,
    pub passed: bool,
}

pub fn readback_onchain_backfill_run(
    root: &Path,
    expected_max_blocks_per_chunk: u64,
) -> Result<OnchainBackfillReadbackReport> {
    readback_onchain_backfill_run_scoped(
        root,
        expected_max_blocks_per_chunk,
        OnchainBackfillReadbackScope::Full,
    )
}

pub fn readback_onchain_backfill_run_scoped(
    root: &Path,
    expected_max_blocks_per_chunk: u64,
    scope: OnchainBackfillReadbackScope,
) -> Result<OnchainBackfillReadbackReport> {
    let root = canonical_root(root)?;
    let report_path = root.join(ONCHAIN_BACKFILL_RUN_REPORT_FILE);
    let checkpoint_path = root.join(ONCHAIN_BACKFILL_RUN_CHECKPOINT_FILE);
    let (readback_file, progress_file) = scoped_readback_files(scope);
    let readback_path = root.join(readback_file);
    let mut ctx = ReadbackContext::new(&root, progress_file)?;
    ctx.emit(
        "POLY_ONCHAIN_BACKFILL_READBACK_STARTED",
        "start",
        None,
        None,
        None,
        None,
        None,
    )?;
    let report: OnchainBackfillRunReport =
        read_json_file(&report_path, ReadbackArtifactKind::Control, &mut ctx)?;
    let checkpoint: OnchainBackfillCheckpoint =
        read_json_file(&checkpoint_path, ReadbackArtifactKind::Control, &mut ctx)?;
    let checkpoint_range_count = checkpoint
        .contracts
        .iter()
        .map(|contract| contract.captured_ranges.len())
        .sum();
    ctx.emit(
        "POLY_ONCHAIN_BACKFILL_READBACK_CONTROL_FILES_LOADED",
        "control_files",
        None,
        None,
        None,
        Some(checkpoint_range_count),
        Some(report_path.display().to_string()),
    )?;
    if report.schema_version != ONCHAIN_BACKFILL_RUN_SCHEMA_VERSION {
        ctx.parse_failures.push(format!(
            "{} unsupported report schema {}",
            report_path.display(),
            report.schema_version
        ));
    }
    if checkpoint.schema_version != ONCHAIN_BACKFILL_RUN_SCHEMA_VERSION {
        ctx.parse_failures.push(format!(
            "{} unsupported checkpoint schema {}",
            checkpoint_path.display(),
            checkpoint.schema_version
        ));
    }
    ctx.check_artifact_sha(
        &checkpoint_path,
        &report.checkpoint_sha256,
        ReadbackArtifactKind::Control,
        true,
    )?;
    ctx.check_artifact_sha(
        Path::new(&report.input_state_path),
        &report.input_state_sha256,
        ReadbackArtifactKind::Control,
        false,
    )?;
    if checkpoint.input_state_sha256 != report.input_state_sha256 {
        ctx.sha_mismatches
            .push("checkpoint input_state_sha256 did not match run report".to_string());
    }
    check_chunk_policy(
        expected_max_blocks_per_chunk,
        &report,
        &checkpoint,
        &report_path,
        &checkpoint_path,
        &mut ctx.parse_failures,
    );
    if canonical_display_path(Path::new(&report.output_root)).as_ref() != Some(&root) {
        ctx.parse_failures.push(format!(
            "run report output_root {} did not match requested root {}",
            report.output_root,
            root.display()
        ));
    }
    check_checkpoint_structure(&checkpoint, &root, &mut ctx.parse_failures);
    let structural_failure = !ctx.parse_failures.is_empty();
    if !structural_failure {
        for (index, page) in report.pages.iter().enumerate() {
            let page_index = index + 1;
            ctx.emit(
                "POLY_ONCHAIN_BACKFILL_READBACK_RUN_PAGE",
                "current_run_pages",
                Some(page_index),
                Some(report.pages.len()),
                None,
                None,
                Some(page.body_path.clone()),
            )?;
            require_page_paths_under_root(page, &root, &mut ctx.parse_failures);
            check_page(page, expected_max_blocks_per_chunk, &mut ctx)?;
        }
        check_current_run_pages_in_checkpoint(&report, &checkpoint, &mut ctx.parse_failures);
        if scope == OnchainBackfillReadbackScope::Full {
            check_checkpoint_ranges(
                &checkpoint,
                expected_max_blocks_per_chunk,
                checkpoint_range_count,
                &mut ctx,
            )?;
        }
    } else {
        ctx.emit(
            "POLY_ONCHAIN_BACKFILL_READBACK_PREFLIGHT_FAILED",
            "preflight",
            None,
            Some(report.pages.len()),
            None,
            Some(checkpoint_range_count),
            Some(checkpoint_path.display().to_string()),
        )?;
    }
    let total_pages = report.pages.len();
    let total_records = report.pages.iter().map(|page| page.record_count).sum();
    let total_body_bytes = report.pages.iter().map(|page| page.body_bytes).sum();
    if total_pages != report.total_pages {
        ctx.parse_failures.push(format!(
            "report total_pages {} did not match page list {}",
            report.total_pages, total_pages
        ));
    }
    if total_records != report.total_records {
        ctx.parse_failures.push(format!(
            "report total_records {} did not match page records {}",
            report.total_records, total_records
        ));
    }
    if total_body_bytes != report.total_body_bytes {
        ctx.parse_failures.push(format!(
            "report total_body_bytes {} did not match page body bytes {}",
            report.total_body_bytes, total_body_bytes
        ));
    }
    let passed = ctx.missing_files.is_empty()
        && ctx.sha_mismatches.is_empty()
        && ctx.parse_failures.is_empty()
        && report.passed
        && report.status_code == ONCHAIN_BACKFILL_RUN_PASSED;
    let status_code = if passed {
        match scope {
            OnchainBackfillReadbackScope::Full => "POLY_ONCHAIN_BACKFILL_READBACK_PASSED",
            OnchainBackfillReadbackScope::CurrentRun => {
                "POLY_ONCHAIN_BACKFILL_CURRENT_RUN_READBACK_PASSED"
            }
        }
    } else {
        "POLY_ONCHAIN_BACKFILL_READBACK_FAILED"
    };
    ctx.emit(
        status_code,
        "summary",
        None,
        Some(total_pages),
        None,
        Some(checkpoint_range_count),
        Some(readback_path.display().to_string()),
    )?;
    ctx.flush_progress()?;
    let readback = OnchainBackfillReadbackReport {
        schema_version: ONCHAIN_BACKFILL_RUN_SCHEMA_VERSION.to_string(),
        source_of_truth:
            "physical on-chain backfill report, checkpoint, source state, request files, metadata, raw bodies, and readback progress JSONL"
                .to_string(),
        status_code: status_code.to_string(),
        output_root: root.display().to_string(),
        run_report_path: report_path.display().to_string(),
        checkpoint_path: checkpoint_path.display().to_string(),
        readback_report_path: readback_path.display().to_string(),
        readback_scope: scope,
        readback_progress_path: ctx.progress_path.display().to_string(),
        checked_file_count: ctx.checked_file_count,
        unique_file_read_count: ctx.unique_file_read_count,
        deduplicated_file_read_count: ctx.deduplicated_file_read_count,
        json_parse_count: ctx.json_parse_count,
        readback_bytes_read: ctx.readback_bytes_read,
        readback_body_bytes_read: ctx.readback_body_bytes_read,
        readback_request_bytes_read: ctx.readback_request_bytes_read,
        readback_metadata_bytes_read: ctx.readback_metadata_bytes_read,
        progress_event_count: ctx.progress_event_count,
        missing_files: ctx.missing_files,
        sha_mismatches: ctx.sha_mismatches,
        parse_failures: ctx.parse_failures,
        total_pages,
        current_run_page_count: total_pages,
        checkpoint_range_count,
        total_records,
        total_body_bytes,
        all_order_filled_backfill_complete: checkpoint.all_order_filled_backfill_complete,
        passed,
    };
    write_json(&readback_path, &readback)?;
    Ok(readback)
}

pub fn require_onchain_backfill_readback_passed(
    report: &OnchainBackfillReadbackReport,
) -> Result<()> {
    if report.passed {
        return Ok(());
    }
    Err(PolyError::raw_source(
        report.status_code.clone(),
        format!(
            "on-chain backfill readback failed: missing={} sha_mismatches={} parse_failures={}",
            report.missing_files.len(),
            report.sha_mismatches.len(),
            report.parse_failures.len()
        ),
    ))
}

fn check_page(
    page: &LargeCorpusPage,
    expected_max_blocks_per_chunk: u64,
    ctx: &mut ReadbackContext,
) -> Result<()> {
    if let Some(request_path) = &page.request_path {
        if let Some(expected) = &page.request_body_sha256 {
            ctx.check_artifact_sha(
                Path::new(request_path),
                expected,
                ReadbackArtifactKind::Request,
                false,
            )?;
        }
    } else {
        ctx.missing_files
            .push(format!("{} missing request_path", page.metadata_path));
    }
    if let Some(expected) = &page.body_sha256 {
        ctx.check_artifact_sha(
            Path::new(&page.body_path),
            expected,
            ReadbackArtifactKind::Body,
            true,
        )?;
    } else {
        ctx.check_json_file(Path::new(&page.body_path), ReadbackArtifactKind::Body)?;
    }
    ctx.check_page_metadata(page)?;
    if page.range_state.is_none() {
        ctx.parse_failures
            .push(format!("{} missing range_state", page.metadata_path));
    } else if let Some(range) = &page.range_state {
        check_page_range_policy(
            page,
            range,
            expected_max_blocks_per_chunk,
            &mut ctx.parse_failures,
        );
    }
    if !page.expectation_met {
        ctx.parse_failures
            .push(format!("{} expectation_met=false", page.metadata_path));
    }
    if page.body_sha256 != page.after.body_sha256 {
        ctx.sha_mismatches.push(format!(
            "{} body_sha256 did not match after state",
            page.body_path
        ));
    }
    ctx.pages_by_body_path
        .insert(ctx.path_key(Path::new(&page.body_path)), page.clone());
    Ok(())
}

fn check_checkpoint_ranges(
    checkpoint: &OnchainBackfillCheckpoint,
    expected_max_blocks_per_chunk: u64,
    checkpoint_range_count: usize,
    ctx: &mut ReadbackContext,
) -> Result<()> {
    let mut checkpoint_range_index = 0usize;
    for contract in &checkpoint.contracts {
        check_checkpoint_contract_summary(contract, &mut ctx.parse_failures);
        for range in &contract.captured_ranges {
            checkpoint_range_index += 1;
            ctx.emit(
                "POLY_ONCHAIN_BACKFILL_READBACK_CHECKPOINT_RANGE",
                "checkpoint_ranges",
                None,
                None,
                Some(checkpoint_range_index),
                Some(checkpoint_range_count),
                Some(range.body_path.clone()),
            )?;
            let body_key = ctx.path_key(Path::new(&range.body_path));
            let metadata_path = match metadata_path_for_body(Path::new(&range.body_path)) {
                Ok(path) => path,
                Err(err) => {
                    ctx.parse_failures.push(err.message());
                    continue;
                }
            };
            if !require_checkpoint_range_paths_under_root(
                range,
                &metadata_path,
                &ctx.root,
                &mut ctx.parse_failures,
            ) {
                continue;
            }
            let page = if let Some(page) = ctx.pages_by_body_path.get(&body_key) {
                page.clone()
            } else {
                match ctx.read_page_metadata(&metadata_path)? {
                    Some(page) => page,
                    None => continue,
                }
            };
            let metadata_path = Path::new(&page.metadata_path);
            if page.dataset != contract.dataset {
                ctx.parse_failures.push(format!(
                    "{} dataset {} did not match checkpoint {}",
                    metadata_path.display(),
                    page.dataset,
                    contract.dataset
                ));
            }
            if page.request_path.as_deref() != Some(range.request_path.as_str()) {
                ctx.parse_failures.push(format!(
                    "{} request_path did not match checkpoint range",
                    metadata_path.display()
                ));
            }
            if let Some(page_range) = &page.range_state {
                if page_range.from_block != range.from_block
                    || page_range.to_block != range.to_block
                {
                    ctx.parse_failures.push(format!(
                        "{} range {}..{} did not match checkpoint {}..{}",
                        metadata_path.display(),
                        page_range.from_block,
                        page_range.to_block,
                        range.from_block,
                        range.to_block
                    ));
                }
            } else {
                ctx.parse_failures
                    .push(format!("{} missing range_state", metadata_path.display()));
            }
            check_page(&page, expected_max_blocks_per_chunk, ctx)?;
        }
    }
    Ok(())
}

fn read_json_file<T: for<'de> Deserialize<'de>>(
    path: &Path,
    kind: ReadbackArtifactKind,
    ctx: &mut ReadbackContext,
) -> Result<T> {
    let key = ctx.path_key(path);
    if ctx.artifacts.contains_key(&key) {
        ctx.deduplicated_file_read_count += 1;
        return Err(PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_READBACK_DUPLICATE_CONTROL_READ",
            format!(
                "control JSON {} was requested more than once",
                path.display()
            ),
        ));
    }
    let bytes = fs::read(path).map_err(|err| {
        ctx.missing_files
            .push(format!("{} read failed: {err}", path.display()));
        PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_READBACK_FILE_READ_FAILED",
            format!("read {}: {err}", path.display()),
        )
    })?;
    let byte_count = bytes.len() as u64;
    ctx.record_artifact_read(kind, byte_count);
    let actual = sha256_hex(&bytes);
    ctx.json_parse_count += 1;
    let decoded = serde_json::from_slice::<T>(&bytes).map_err(|err| {
        ctx.parse_failures
            .push(format!("decode JSON {}: {err}", path.display()));
        PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_READBACK_JSON_DECODE_FAILED",
            format!("decode {}: {err}", path.display()),
        )
    })?;
    ctx.artifacts.insert(
        key,
        CachedArtifact {
            actual_sha256: actual,
            byte_count,
            json_parse_checked: true,
            json_parse_ok: true,
        },
    );
    Ok(decoded)
}
