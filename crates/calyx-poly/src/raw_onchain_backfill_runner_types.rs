use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::raw_large_corpus::LargeCorpusPage;
use crate::raw_large_corpus_onchain_backfill::CapturedRangeState;
use crate::raw_large_corpus_onchain_chunks::POLYGON_RPC_SAFE_LOG_CHUNK_BLOCKS;
use crate::raw_source_support::{display_safe_path, sha256_hex};
use crate::{PolyError, Result};

pub const ONCHAIN_BACKFILL_RUN_SCHEMA_VERSION: &str = "poly.onchain_backfill.runner.v1";
pub const ONCHAIN_BACKFILL_RUN_PASSED: &str = "POLY_ONCHAIN_BACKFILL_RUN_PASSED";
pub const ONCHAIN_BACKFILL_RUN_CHECKPOINT_FILE: &str = "onchain-backfill-checkpoint.json";
pub const ONCHAIN_BACKFILL_RUN_REPORT_FILE: &str = "onchain-backfill-run-report.json";

pub(crate) fn planned_chunk_count(from_block: u64, to_block: u64, chunk_size: u64) -> u64 {
    let block_count = to_block.saturating_sub(from_block).saturating_add(1);
    (block_count.saturating_add(chunk_size).saturating_sub(1)) / chunk_size
}

pub(crate) fn sha256_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|err| {
        PolyError::raw_source(
            "POLY_ONCHAIN_BACKFILL_SHA_READ_FAILED",
            format!("read {} for SHA256: {err}", path.display()),
        )
    })?;
    Ok(sha256_hex(&bytes))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnchainBackfillRunRequest {
    pub state_path: PathBuf,
    pub output_root: PathBuf,
    pub timeout_secs: u64,
    pub max_body_bytes: usize,
    pub max_chunks_per_contract: usize,
    pub max_blocks_per_chunk: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnchainBackfillRunReport {
    pub schema_version: String,
    pub source_of_truth: String,
    pub status_code: String,
    pub input_state_path: String,
    pub input_state_sha256: String,
    pub output_root: String,
    pub checkpoint_path: String,
    pub checkpoint_sha256: String,
    pub max_chunks_per_contract: usize,
    pub max_blocks_per_chunk: u64,
    pub pages: Vec<LargeCorpusPage>,
    pub contracts: Vec<OnchainBackfillContractRun>,
    pub total_pages: usize,
    pub total_records: usize,
    pub total_body_bytes: u64,
    pub all_order_filled_backfill_complete: bool,
    pub next_required_action: String,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnchainBackfillContractRun {
    pub dataset: String,
    pub address: String,
    pub planned_from_block: u64,
    pub planned_to_block: u64,
    pub planned_chunk_count: u64,
    pub start_from_block: Option<u64>,
    pub chunks_captured_this_run: usize,
    pub records_captured_this_run: usize,
    pub next_required_from_block: Option<u64>,
    pub coverage_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OnchainBackfillCheckpoint {
    pub schema_version: String,
    pub source_of_truth: String,
    pub status_code: String,
    pub input_state_path: String,
    pub input_state_sha256: String,
    pub chain: String,
    pub chain_id: u64,
    pub latest_safe_block: u64,
    pub max_blocks_per_chunk: u64,
    pub contracts: Vec<OnchainBackfillContractCheckpoint>,
    pub all_order_filled_backfill_complete: bool,
    pub next_required_action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OnchainBackfillContractCheckpoint {
    pub dataset: String,
    pub address: String,
    pub planned_from_block: u64,
    pub planned_to_block: u64,
    pub planned_chunk_count: u64,
    pub captured_ranges: Vec<CapturedRangeState>,
    pub captured_chunk_count: usize,
    pub captured_record_count: usize,
    pub captured_block_count: u64,
    pub next_required_from_block: Option<u64>,
    pub coverage_complete: bool,
}

impl OnchainBackfillRunRequest {
    pub fn target_default() -> Self {
        Self {
            state_path: PathBuf::from("target/fsv/onchain-backfill-state.json"),
            output_root: PathBuf::from("target/fsv/issue27_onchain_backfill_runner"),
            timeout_secs: 45,
            max_body_bytes: 50 * 1024 * 1024,
            max_chunks_per_contract: 1,
            max_blocks_per_chunk: POLYGON_RPC_SAFE_LOG_CHUNK_BLOCKS,
        }
    }

    pub fn normalized(mut self) -> Result<Self> {
        if self.state_path.as_os_str().is_empty() {
            return Err(PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_STATE_PATH_EMPTY",
                "on-chain backfill state path must not be empty",
            ));
        }
        if self.output_root.as_os_str().is_empty() {
            return Err(PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_OUTPUT_ROOT_EMPTY",
                "on-chain backfill output root must not be empty",
            ));
        }
        if self.timeout_secs == 0 {
            return Err(PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_TIMEOUT_INVALID",
                "on-chain backfill timeout must be greater than zero",
            ));
        }
        if self.max_body_bytes == 0 {
            return Err(PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_BODY_LIMIT_INVALID",
                "on-chain backfill max body bytes must be greater than zero",
            ));
        }
        if self.max_chunks_per_contract == 0 {
            return Err(PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_CHUNK_LIMIT_INVALID",
                "on-chain backfill max chunks per contract must be greater than zero",
            ));
        }
        if self.max_blocks_per_chunk == 0 {
            return Err(PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_BLOCK_CHUNK_INVALID",
                "on-chain backfill max blocks per chunk must be greater than zero",
            ));
        }
        let cwd = env::current_dir().map_err(|err| {
            PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_CURRENT_DIR_FAILED",
                format!("read current directory: {err}"),
            )
        })?;
        if self.state_path.is_relative() {
            self.state_path = cwd.join(&self.state_path);
        }
        if self.output_root.is_relative() {
            self.output_root = cwd.join(&self.output_root);
        }
        fs::create_dir_all(&self.output_root).map_err(|err| {
            PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_OUTPUT_ROOT_CREATE_FAILED",
                format!(
                    "create on-chain backfill output root {}: {err}",
                    self.output_root.display()
                ),
            )
        })?;
        self.output_root =
            display_safe_path(fs::canonicalize(&self.output_root).map_err(|err| {
                PolyError::raw_source(
                    "POLY_ONCHAIN_BACKFILL_OUTPUT_ROOT_CANONICALIZE_FAILED",
                    format!(
                        "canonicalize on-chain backfill output root {}: {err}",
                        self.output_root.display()
                    ),
                )
            })?);
        self.state_path = display_safe_path(fs::canonicalize(&self.state_path).map_err(|err| {
            PolyError::raw_source(
                "POLY_ONCHAIN_BACKFILL_STATE_CANONICALIZE_FAILED",
                format!(
                    "canonicalize on-chain backfill state {}: {err}",
                    self.state_path.display()
                ),
            )
        })?);
        Ok(self)
    }
}
