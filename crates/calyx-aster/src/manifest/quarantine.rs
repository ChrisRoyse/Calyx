use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuarantineRecord {
    pub range_start: u64,
    pub range_end: u64,
    pub broken_at_seq: u64,
    pub detected_at_ts: u64,
}

impl QuarantineRecord {
    pub fn new(
        range_start: u64,
        range_end: u64,
        broken_at_seq: u64,
        detected_at_ts: u64,
    ) -> Result<Self> {
        let record = Self {
            range_start,
            range_end,
            broken_at_seq,
            detected_at_ts,
        };
        record.validate()?;
        Ok(record)
    }

    pub const fn contains(&self, seq: u64) -> bool {
        self.range_start <= seq && seq < self.range_end
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.range_start >= self.range_end {
            return Err(CalyxError::ledger_chain_broken(
                "quarantine range must be non-empty",
            ));
        }
        if !self.contains(self.broken_at_seq) {
            return Err(CalyxError::ledger_chain_broken(format!(
                "broken seq {} is outside quarantine range {}..{}",
                self.broken_at_seq, self.range_start, self.range_end
            )));
        }
        Ok(())
    }
}
