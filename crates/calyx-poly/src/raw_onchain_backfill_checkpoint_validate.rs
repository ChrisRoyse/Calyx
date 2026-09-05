//! Prior-checkpoint load + integrity validation for the on-chain backfill runner (issue #214).
//!
//! Extracted verbatim from `raw_onchain_backfill_runner` to keep every source file under the
//! 500-line doctrine limit. `read_previous_checkpoint` loads a resumable run's prior checkpoint and
//! fails closed if it belongs to a different input state, uses a different chunk policy, or has
//! internally inconsistent captured-range summaries / out-of-root artifact paths.

use std::fs;
use std::path::{Path, PathBuf};

use crate::raw_large_corpus_onchain_backfill::CapturedRangeState;
use crate::raw_onchain_backfill_runner_types::{
    ONCHAIN_BACKFILL_RUN_CHECKPOINT_FILE, OnchainBackfillCheckpoint,
    OnchainBackfillContractCheckpoint,
};
use crate::raw_source_support::display_safe_path;
use crate::{PolyError, Result};

pub(crate) fn read_previous_checkpoint(
    root: &Path,
    state_sha: &str,
    max_blocks_per_chunk: u64,
) -> Result<Option<OnchainBackfillCheckpoint>> {
    let path = root.join(ONCHAIN_BACKFILL_RUN_CHECKPOINT_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path).map_err(|err| {
        PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_CHECKPOINT_READ_FAILED",
            format!("read checkpoint {}: {err}", path.display()),
        )
    })?;
    let checkpoint =
        serde_json::from_slice::<OnchainBackfillCheckpoint>(&bytes).map_err(|err| {
            PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_CHECKPOINT_DECODE_FAILED",
                format!("decode checkpoint {}: {err}", path.display()),
            )
        })?;
    if checkpoint.input_state_sha256 != state_sha {
        return Err(PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_CHECKPOINT_STATE_MISMATCH",
            format!(
                "checkpoint {} belongs to input state {}, current state is {}",
                path.display(),
                checkpoint.input_state_sha256,
                state_sha
            ),
        ));
    }
    if checkpoint.max_blocks_per_chunk != max_blocks_per_chunk {
        return Err(PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_CHECKPOINT_CHUNK_POLICY_MISMATCH",
            format!(
                "checkpoint {} used max_blocks_per_chunk {}, current request is {}",
                path.display(),
                checkpoint.max_blocks_per_chunk,
                max_blocks_per_chunk
            ),
        ));
    }
    validate_previous_checkpoint(&path, root, &checkpoint)?;
    Ok(Some(checkpoint))
}

fn validate_previous_checkpoint(
    checkpoint_path: &Path,
    root: &Path,
    checkpoint: &OnchainBackfillCheckpoint,
) -> Result<()> {
    let root = canonical_or_raw(root)?;
    let mut failures = Vec::new();
    for contract in &checkpoint.contracts {
        validate_previous_checkpoint_contract(contract, &root, &mut failures);
    }
    if failures.is_empty() {
        return Ok(());
    }
    Err(PolyError::raw_source(
        "POLY_ONCHAIN_BACKFILL_PREVIOUS_CHECKPOINT_INVALID",
        format!(
            "checkpoint {} failed integrity validation: {}",
            checkpoint_path.display(),
            failures.join("; ")
        ),
    ))
}

fn validate_previous_checkpoint_contract(
    contract: &OnchainBackfillContractCheckpoint,
    root: &Path,
    failures: &mut Vec<String>,
) {
    let range_count = contract.captured_ranges.len();
    if contract.captured_chunk_count != range_count {
        failures.push(format!(
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
        failures.push(format!(
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
        failures.push(format!(
            "{} captured_block_count {} did not match captured_ranges inclusive blocks {}",
            contract.dataset, contract.captured_block_count, block_count
        ));
    }
    for range in &contract.captured_ranges {
        validate_previous_checkpoint_range_paths(range, root, failures);
    }
}

fn validate_previous_checkpoint_range_paths(
    range: &CapturedRangeState,
    root: &Path,
    failures: &mut Vec<String>,
) {
    let metadata_path = match metadata_path_for_body(Path::new(&range.body_path)) {
        Ok(path) => path,
        Err(err) => {
            failures.push(err.message());
            return;
        }
    };
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
            failures.push(format!(
                "checkpoint range path {} could not be canonicalized",
                path.display()
            ));
            continue;
        };
        if !canonical.starts_with(root) {
            failures.push(format!(
                "checkpoint range path {} is outside output root {}",
                canonical.display(),
                root.display()
            ));
        }
    }
}

fn metadata_path_for_body(body_path: &Path) -> Result<PathBuf> {
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

fn canonical_display_path(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path).ok().map(display_safe_path)
}

fn canonical_or_raw(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path)
        .map(display_safe_path)
        .map_err(|err| {
            PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_PATH_CANONICALIZE_FAILED",
                format!("canonicalize {}: {err}", path.display()),
            )
        })
}
