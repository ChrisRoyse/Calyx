use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::cf::ColumnFamily;
use crate::layers::{RecordKey, Row};

pub(super) const DEFAULT_BATCH_SIZE: usize = 10_000;
pub(super) const MAX_BATCH_SIZE: usize = 10_000;
pub(super) const RECORD_DISC: u8 = 0x01;

pub(super) type IndexRow = (ColumnFamily, Vec<u8>, Vec<u8>);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebuildStats {
    pub rows_scanned: u64,
    pub keys_added: u64,
    pub stale_removed: u64,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexHealth {
    pub missing: u64,
    pub stale: u64,
    pub healthy: bool,
}

pub(super) struct RecordDataRow {
    pub data_key: Vec<u8>,
    pub pk: RecordKey,
    pub row: Row,
}

pub(super) fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
