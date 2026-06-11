use crate::cf::{ColumnFamily, KeyRange};
use calyx_core::CalyxError;

pub const CALYX_ASTER_BASE_CORRUPT: &str = "CALYX_ASTER_BASE_CORRUPT";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadBarrier {
    id: String,
    cf: ColumnFamily,
    range: KeyRange,
    code: &'static str,
    message: String,
    remediation: &'static str,
}

impl ReadBarrier {
    pub fn base_corrupt(id: impl Into<String>, range: KeyRange) -> Self {
        let id = id.into();
        Self {
            message: format!("base shard {id} is corrupt; reads in this range are blocked"),
            id,
            cf: ColumnFamily::Base,
            range,
            code: CALYX_ASTER_BASE_CORRUPT,
            remediation: "restore from restic/snapshot",
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn cf(&self) -> ColumnFamily {
        self.cf
    }

    pub fn range(&self) -> &KeyRange {
        &self.range
    }

    pub fn blocks(&self, cf: ColumnFamily, key: &[u8]) -> bool {
        self.cf == cf && self.range.contains(key)
    }

    pub fn error(&self) -> CalyxError {
        CalyxError {
            code: self.code,
            message: self.message.clone(),
            remediation: self.remediation,
        }
    }
}

pub(super) fn first_blocking(
    barriers: &[ReadBarrier],
    cf: ColumnFamily,
    key: &[u8],
) -> Option<CalyxError> {
    barriers
        .iter()
        .find(|barrier| barrier.blocks(cf, key))
        .map(ReadBarrier::error)
}
