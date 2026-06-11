mod alert;
mod barrier;
mod checksum;

use std::fmt;

use calyx_aster::cf::KeyRange;
use calyx_core::CalyxError;
use serde::{Deserialize, Serialize};

pub use alert::{alert_operator, write_base_restored_event};
pub use barrier::{
    CALYX_ANNEAL_RESTORE_FAILED, RestoreCommand, RestoreConfig, RestoreOutcome, attempt_restore,
    clear_reads_on_range, fail_reads_on_range, install_recorded_read_barriers,
};
pub use checksum::{
    CALYX_ANNEAL_CHECKSUM_INVALID_ROW, base_shard_checksum, load_base_shards,
    record_base_shard_checksum, verify_base_shards,
};

pub const BASE_SHARD_CHECKSUM_TAG: &str = "anneal_base_shard_checksum_v1";
pub const CALYX_ANNEAL_ALERT_WRITE_FAILED: &str = "CALYX_ANNEAL_ALERT_WRITE_FAILED";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ShardId(String);

impl ShardId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ShardId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaseShard {
    pub shard_id: ShardId,
    pub cf_range: KeyRange,
    pub checksum: [u8; 32],
}

impl BaseShard {
    pub fn new(shard_id: ShardId, cf_range: KeyRange, checksum: [u8; 32]) -> Self {
        Self {
            shard_id,
            cf_range,
            checksum,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BaseFaultEvent {
    Corrupt {
        shard: BaseShard,
        expected: [u8; 32],
        actual: [u8; 32],
        detected_at: u64,
    },
}

impl BaseFaultEvent {
    pub fn corrupt(shard: BaseShard, actual: [u8; 32], detected_at: u64) -> Self {
        Self::Corrupt {
            expected: shard.checksum,
            shard,
            actual,
            detected_at,
        }
    }

    pub fn shard(&self) -> &BaseShard {
        match self {
            Self::Corrupt { shard, .. } => shard,
        }
    }

    pub fn expected(&self) -> [u8; 32] {
        match self {
            Self::Corrupt { expected, .. } => *expected,
        }
    }

    pub fn actual(&self) -> [u8; 32] {
        match self {
            Self::Corrupt { actual, .. } => *actual,
        }
    }

    pub fn detected_at(&self) -> u64 {
        match self {
            Self::Corrupt { detected_at, .. } => *detected_at,
        }
    }
}

pub(crate) fn invalid_checksum_row(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: checksum::CALYX_ANNEAL_CHECKSUM_INVALID_ROW,
        message: message.into(),
        remediation: "repair anneal_checksums CF before running base-shard restore",
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn decode_hex32(value: &str, field: &str) -> Result<[u8; 32], CalyxError> {
    if value.len() != 64 {
        return Err(invalid_checksum_row(format!(
            "{field} has {} hex chars, expected 64",
            value.len()
        )));
    }
    let mut out = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        out[index] = (hex_value(pair[0], field)? << 4) | hex_value(pair[1], field)?;
    }
    Ok(out)
}

pub(crate) fn decode_hex_vec(value: &str, field: &str) -> Result<Vec<u8>, CalyxError> {
    if !value.len().is_multiple_of(2) {
        return Err(invalid_checksum_row(format!(
            "{field} has odd hex length {}",
            value.len()
        )));
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().chunks_exact(2) {
        out.push((hex_value(pair[0], field)? << 4) | hex_value(pair[1], field)?);
    }
    Ok(out)
}

fn hex_value(byte: u8, field: &str) -> Result<u8, CalyxError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(invalid_checksum_row(format!(
            "{field} contains non-hex byte"
        ))),
    }
}
