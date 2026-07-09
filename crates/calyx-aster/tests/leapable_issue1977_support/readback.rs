use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct FileReadback {
    pub(crate) path: String,
    pub(crate) len: u64,
    pub(crate) sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct BaseRowReadback {
    pub(crate) wal_seq: u64,
    pub(crate) sst_path: Option<String>,
    pub(crate) text: String,
    pub(crate) case: String,
    pub(crate) value_len: usize,
    pub(crate) value_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct VaultReadback {
    pub(crate) vault_dir: String,
    pub(crate) current_sha256: Option<String>,
    pub(crate) manifest_sha256: Option<String>,
    pub(crate) wal_files: Vec<FileReadback>,
    pub(crate) wal_record_count: usize,
    pub(crate) wal_torn_tail: Option<String>,
    pub(crate) wal_base_rows: BTreeMap<String, BaseRowReadback>,
    pub(crate) sst_base_rows: BTreeMap<String, BaseRowReadback>,
}

pub(crate) struct WalReadback {
    pub(crate) files: Vec<FileReadback>,
    pub(crate) record_count: usize,
    pub(crate) torn_tail: Option<String>,
    pub(crate) base_rows: BTreeMap<String, BaseRowReadback>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ExpectedRecord {
    pub(crate) index: u8,
    pub(crate) cx_id: String,
    pub(crate) text: String,
    pub(crate) case: String,
    pub(crate) checkpoint_flushed: bool,
}
