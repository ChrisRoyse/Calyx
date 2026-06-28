use std::collections::BTreeMap;
use std::path::Path;

use calyx_aster::ledger_view::AsterLedgerCfStore;
use calyx_core::{CalyxError, CxId, LedgerRef};
use calyx_ledger::{LedgerCfStore, SubjectId, VerifyResult, decode, verify_chain};
use calyx_sextant::{CALYX_SEXTANT_PROVENANCE_MISSING, sextant_error};

use crate::server::ToolResult;

pub(super) struct VerifiedSearchLedger {
    entries: BTreeMap<u64, calyx_ledger::LedgerEntry>,
}

impl VerifiedSearchLedger {
    pub(super) fn open(path: &Path) -> ToolResult<Self> {
        let store = AsterLedgerCfStore::open(path).map_err(|error| {
            if error.code == "CALYX_LEDGER_CORRUPT" {
                CalyxError::ledger_chain_broken(format!(
                    "search provenance ledger chain unreadable: {}",
                    error.message
                ))
            } else {
                error
            }
        })?;
        let rows = store.scan()?;
        let end = rows
            .iter()
            .map(|row| row.seq)
            .max()
            .map_or(0, |seq| seq.saturating_add(1));
        match verify_chain(&store, 0..end)? {
            VerifyResult::Intact { .. } => {}
            VerifyResult::Broken { at_seq, .. } | VerifyResult::Corrupt { at_seq, .. } => {
                return Err(CalyxError::ledger_chain_broken(format!(
                    "search provenance ledger chain broken at seq={at_seq}"
                ))
                .into());
            }
        }
        let mut entries = BTreeMap::new();
        for row in rows {
            entries.insert(row.seq, decode(&row.bytes)?);
        }
        Ok(Self { entries })
    }

    pub(super) fn require_ref(&self, cx_id: CxId, expected: LedgerRef) -> ToolResult<LedgerRef> {
        let entry = self.entries.get(&expected.seq).ok_or_else(|| {
            missing_provenance(format!(
                "search hit {cx_id} references missing ledger seq {}",
                expected.seq
            ))
        })?;
        if entry.entry_hash != expected.hash {
            return Err(CalyxError::ledger_corrupt(format!(
                "search hit {cx_id} ledger seq {} hash does not match Base provenance",
                expected.seq
            ))
            .into());
        }
        if entry.subject != SubjectId::Cx(cx_id) {
            return Err(CalyxError::ledger_corrupt(format!(
                "search hit {cx_id} ledger seq {} subject mismatch",
                expected.seq
            ))
            .into());
        }
        Ok(expected)
    }
}

fn missing_provenance(message: impl Into<String>) -> CalyxError {
    sextant_error(CALYX_SEXTANT_PROVENANCE_MISSING, message)
}
