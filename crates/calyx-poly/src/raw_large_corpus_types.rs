use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::raw_large_corpus_failure::LargeCorpusFailure;
use crate::raw_large_corpus_range::LargeCorpusRangeState;
use crate::raw_sources::RawFileState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LargeCorpusRequest {
    pub output_root: PathBuf,
    pub timeout_secs: u64,
    pub max_body_bytes: usize,
    pub page_size: usize,
    pub max_pages_per_dataset: usize,
    #[serde(default)]
    pub require_exhaustive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LargeCorpusManifest {
    pub schema_version: String,
    pub captured_at_unix_ms: u128,
    pub source_of_truth: String,
    #[serde(default)]
    pub capture_goal: String,
    #[serde(default)]
    pub require_exhaustive: bool,
    pub page_size: usize,
    pub max_pages_per_dataset: usize,
    #[serde(default)]
    pub bounded_incomplete_datasets: Vec<LargeCorpusBoundedIncompleteDataset>,
    #[serde(default)]
    pub trade_history_state_path: String,
    #[serde(default)]
    pub onchain_backfill_state_path: String,
    pub pages: Vec<LargeCorpusPage>,
    pub edge_cases: Vec<LargeCorpusEdgeCase>,
    pub field_profile_paths: Vec<String>,
    pub join_profile_path: String,
    pub schema_decision_input_path: String,
    pub total_pages: usize,
    pub total_records: usize,
    pub total_body_bytes: u64,
    pub passed: bool,
    pub status_code: String,
    pub failure: Option<LargeCorpusFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LargeCorpusBoundedIncompleteDataset {
    pub dataset: String,
    pub source: String,
    pub endpoint: String,
    pub page_count: usize,
    pub last_page_index: usize,
    pub last_record_count: usize,
    pub page_size: usize,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LargeCorpusPaginationState {
    pub mode: String,
    pub items_field: Option<String>,
    pub requested_limit: usize,
    pub requested_offset: Option<usize>,
    pub request_after_cursor: Option<String>,
    pub response_next_cursor: Option<String>,
    pub terminal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LargeCorpusPage {
    pub dataset: String,
    pub source: String,
    pub endpoint: String,
    pub method: String,
    pub docs_url: String,
    pub page_index: usize,
    pub url: String,
    pub request_path: Option<String>,
    pub request_body_bytes: u64,
    pub request_body_sha256: Option<String>,
    pub status_code: Option<u16>,
    pub http_success: bool,
    pub expectation_met: bool,
    pub record_count: usize,
    pub stop_reason: Option<String>,
    pub body_path: String,
    pub metadata_path: String,
    pub body_format: String,
    pub body_bytes: u64,
    pub body_sha256: Option<String>,
    pub json_parse_ok: bool,
    pub websocket_frame_count: Option<usize>,
    pub websocket_json_frame_count: Option<usize>,
    pub websocket_event_types: Vec<String>,
    pub no_payload_window: bool,
    #[serde(default)]
    pub pagination_state: Option<LargeCorpusPaginationState>,
    #[serde(default)]
    pub range_state: Option<LargeCorpusRangeState>,
    pub before: RawFileState,
    pub after: RawFileState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LargeCorpusEdgeCase {
    pub name: String,
    pub method: String,
    pub url: String,
    pub request_path: Option<String>,
    pub request_body_bytes: u64,
    pub request_body_sha256: Option<String>,
    pub expected_semantics: String,
    pub status_code: Option<u16>,
    pub expectation_met: bool,
    pub record_count: usize,
    pub body_path: String,
    pub metadata_path: String,
    pub body_format: String,
    pub json_parse_ok: bool,
    pub body_sha256: Option<String>,
    pub websocket_frame_count: Option<usize>,
    pub websocket_json_frame_count: Option<usize>,
    pub websocket_event_types: Vec<String>,
    pub no_payload_window: bool,
    #[serde(default)]
    pub range_state: Option<LargeCorpusRangeState>,
    pub before: RawFileState,
    pub after: RawFileState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LargeCorpusReadbackReport {
    pub schema_version: String,
    pub manifest_path: String,
    #[serde(default)]
    pub capture_goal: String,
    #[serde(default)]
    pub require_exhaustive: bool,
    #[serde(default)]
    pub bounded_incomplete_datasets: Vec<LargeCorpusBoundedIncompleteDataset>,
    #[serde(default)]
    pub trade_history_state_path: String,
    #[serde(default)]
    pub onchain_backfill_state_path: String,
    pub checked_file_count: usize,
    pub missing_files: Vec<String>,
    pub sha_mismatches: Vec<String>,
    pub parse_failures: Vec<String>,
    pub total_pages: usize,
    pub total_records: usize,
    pub total_body_bytes: u64,
    pub edge_case_count: usize,
    pub passed: bool,
    pub status_code: String,
    pub failure: Option<LargeCorpusFailure>,
}
