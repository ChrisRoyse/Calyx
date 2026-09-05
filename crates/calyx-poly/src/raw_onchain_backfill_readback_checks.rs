//! Pure structural/policy validators and path helpers for on-chain backfill readback (issue #214).
//!
//! Extracted verbatim from `raw_onchain_backfill_runner_readback` to keep every source file under the
//! 500-line doctrine limit. These functions carry no readback state: they take the artifacts to check
//! plus a `&mut Vec<String>` failure sink, so they are trivially reusable by both the preflight
//! structural pass and the per-page pass in the parent orchestrator.

use std::fs;
use std::path::{Path, PathBuf};

use crate::raw_large_corpus::LargeCorpusPage;
use crate::raw_onchain_backfill_runner_types::{
    OnchainBackfillCheckpoint, OnchainBackfillRunReport,
};
use crate::raw_source_support::{display_safe_path, sha256_hex};
use crate::{PolyError, Result};

pub(crate) fn require_page_paths_under_root(
    page: &LargeCorpusPage,
    root: &Path,
    parse_failures: &mut Vec<String>,
) {
    for path in [
        Some(page.body_path.as_str()),
        Some(page.metadata_path.as_str()),
        page.request_path.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let Some(canonical) = canonical_display_path(Path::new(path)) else {
            parse_failures.push(format!("page path {path} could not be canonicalized"));
            continue;
        };
        if !canonical.starts_with(root) {
            parse_failures.push(format!(
                "page path {} is outside requested root {}",
                canonical.display(),
                root.display()
            ));
        }
    }
}

pub(crate) fn check_checkpoint_structure(
    checkpoint: &OnchainBackfillCheckpoint,
    root: &Path,
    parse_failures: &mut Vec<String>,
) {
    for contract in &checkpoint.contracts {
        check_checkpoint_contract_summary(contract, parse_failures);
        for range in &contract.captured_ranges {
            let metadata_path = match metadata_path_for_body(Path::new(&range.body_path)) {
                Ok(path) => path,
                Err(err) => {
                    parse_failures.push(err.message());
                    continue;
                }
            };
            require_checkpoint_range_paths_under_root(range, &metadata_path, root, parse_failures);
        }
    }
}

pub(crate) fn check_checkpoint_contract_summary(
    contract: &crate::raw_onchain_backfill_runner_types::OnchainBackfillContractCheckpoint,
    parse_failures: &mut Vec<String>,
) {
    let range_count = contract.captured_ranges.len();
    if contract.captured_chunk_count != range_count {
        parse_failures.push(format!(
            "{} captured_chunk_count {} did not match captured_ranges len {}",
            contract.dataset, contract.captured_chunk_count, range_count
        ));
    }
    let record_count = contract
        .captured_ranges
        .iter()
        .map(|range| range.record_count)
        .sum::<usize>();
    if contract.captured_record_count != record_count {
        parse_failures.push(format!(
            "{} captured_record_count {} did not match captured_ranges records {}",
            contract.dataset, contract.captured_record_count, record_count
        ));
    }
    let block_count = contract
        .captured_ranges
        .iter()
        .map(|range| {
            range
                .to_block
                .saturating_sub(range.from_block)
                .saturating_add(1)
        })
        .sum::<u64>();
    if contract.captured_block_count != block_count {
        parse_failures.push(format!(
            "{} captured_block_count {} did not match captured_ranges inclusive blocks {}",
            contract.dataset, contract.captured_block_count, block_count
        ));
    }
}

pub(crate) fn require_checkpoint_range_paths_under_root(
    range: &crate::raw_large_corpus_onchain_backfill::CapturedRangeState,
    metadata_path: &Path,
    root: &Path,
    parse_failures: &mut Vec<String>,
) -> bool {
    let mut ok = true;
    let metadata_path_text = metadata_path.display().to_string();
    for path in [
        range.body_path.as_str(),
        range.request_path.as_str(),
        metadata_path_text.as_str(),
    ] {
        let path = Path::new(path);
        let Some(canonical) = canonical_display_path(path) else {
            if path.is_absolute() && display_safe_path(path.to_path_buf()).starts_with(root) {
                continue;
            }
            parse_failures.push(format!(
                "checkpoint range path {} could not be canonicalized",
                path.display()
            ));
            ok = false;
            continue;
        };
        if !canonical.starts_with(root) {
            parse_failures.push(format!(
                "checkpoint range path {} is outside requested root {}",
                canonical.display(),
                root.display()
            ));
            ok = false;
        }
    }
    ok
}

pub(crate) fn check_chunk_policy(
    expected_max_blocks_per_chunk: u64,
    report: &OnchainBackfillRunReport,
    checkpoint: &OnchainBackfillCheckpoint,
    report_path: &Path,
    checkpoint_path: &Path,
    parse_failures: &mut Vec<String>,
) {
    if expected_max_blocks_per_chunk == 0 {
        parse_failures.push("requested max_blocks_per_chunk must be greater than zero".to_string());
    }
    if report.max_blocks_per_chunk != expected_max_blocks_per_chunk {
        parse_failures.push(format!(
            "{} max_blocks_per_chunk {} did not match requested {}",
            report_path.display(),
            report.max_blocks_per_chunk,
            expected_max_blocks_per_chunk
        ));
    }
    if checkpoint.max_blocks_per_chunk != expected_max_blocks_per_chunk {
        parse_failures.push(format!(
            "{} max_blocks_per_chunk {} did not match requested {}",
            checkpoint_path.display(),
            checkpoint.max_blocks_per_chunk,
            expected_max_blocks_per_chunk
        ));
    }
    if report.max_blocks_per_chunk != checkpoint.max_blocks_per_chunk {
        parse_failures.push(format!(
            "{} max_blocks_per_chunk {} did not match checkpoint {} value {}",
            report_path.display(),
            report.max_blocks_per_chunk,
            checkpoint_path.display(),
            checkpoint.max_blocks_per_chunk
        ));
    }
}

pub(crate) fn check_page_range_policy(
    page: &LargeCorpusPage,
    range: &crate::raw_large_corpus_range::LargeCorpusRangeState,
    expected_max_blocks_per_chunk: u64,
    parse_failures: &mut Vec<String>,
) {
    let observed_block_count = range
        .to_block
        .saturating_sub(range.from_block)
        .saturating_add(1);
    if range.requested_block_count == 0 {
        parse_failures.push(format!(
            "{} requested_block_count must be greater than zero",
            page.metadata_path
        ));
    }
    if range.requested_block_count != observed_block_count {
        parse_failures.push(format!(
            "{} requested_block_count {} did not match range {}..{} observed block count {}",
            page.metadata_path,
            range.requested_block_count,
            range.from_block,
            range.to_block,
            observed_block_count
        ));
    }
    if range.max_blocks_per_chunk != range.requested_block_count {
        parse_failures.push(format!(
            "{} range_state max_blocks_per_chunk {} did not match requested_block_count {}",
            page.metadata_path, range.max_blocks_per_chunk, range.requested_block_count
        ));
    }
    if range.requested_block_count > expected_max_blocks_per_chunk {
        parse_failures.push(format!(
            "{} requested_block_count {} exceeds requested max_blocks_per_chunk {}",
            page.metadata_path, range.requested_block_count, expected_max_blocks_per_chunk
        ));
    }
}

pub(crate) fn metadata_path_for_body(body_path: &Path) -> Result<PathBuf> {
    let file_name = body_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_BODY_PATH_INVALID",
                format!("body path {} has no UTF-8 file name", body_path.display()),
            )
        })?;
    let metadata_name = file_name.strip_suffix(".json").ok_or_else(|| {
        PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_BODY_PATH_FORMAT_INVALID",
            format!("body path {} is not a .json body", body_path.display()),
        )
    })?;
    Ok(body_path.with_file_name(format!("{metadata_name}.metadata.json")))
}

pub(crate) fn canonical_root(root: &Path) -> Result<PathBuf> {
    let path = fs::canonicalize(root).map_err(|err| {
        PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_READBACK_ROOT_CANONICALIZE_FAILED",
            format!(
                "canonicalize on-chain backfill root {}: {err}",
                root.display()
            ),
        )
    })?;
    Ok(display_safe_path(path))
}

pub(crate) fn canonical_display_path(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path).ok().map(display_safe_path)
}

pub(crate) fn sha256_page_metadata(page: &LargeCorpusPage) -> Result<String> {
    let bytes = serde_json::to_vec_pretty(page).map_err(|err| {
        PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_METADATA_ENCODE_FAILED",
            format!(
                "encode metadata page {} for SHA256: {err}",
                page.metadata_path
            ),
        )
    })?;
    Ok(sha256_hex(&bytes))
}
