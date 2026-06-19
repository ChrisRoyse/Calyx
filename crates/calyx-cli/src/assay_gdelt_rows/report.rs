use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Debug, Serialize)]
pub(super) struct Report {
    pub(super) format: &'static str,
    pub(super) dataset: String,
    pub(super) rows_jsonl: String,
    pub(super) manifest: String,
    pub(super) rows: usize,
    pub(super) label_counts: BTreeMap<String, usize>,
    pub(super) source_files: usize,
    pub(super) source_bytes: u64,
    pub(super) rows_jsonl_sha256: String,
    pub(super) manifest_sha256: String,
    pub(super) first_row: Option<Value>,
    pub(super) last_row: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct SourceFile {
    pub(super) path: String,
    pub(super) sha256: String,
    pub(super) bytes: u64,
    pub(super) rows_read: usize,
}
