use std::collections::BTreeMap;
use std::path::Path;

use calyx_aster::ledger_view::AsterLedgerCfStore;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Constellation, CxId, LedgerRef, VaultStore};
use calyx_ledger::{LedgerCfStore, SubjectId, VerifyResult, decode, verify_chain};
use calyx_sextant::{
    CALYX_SEXTANT_PROVENANCE_MISSING, FreshnessTag, Hit, ProvenanceSource, sextant_error,
};

use crate::error::CliResult;

#[cfg(test)]
mod tests;

pub(crate) fn hit_docs(
    vault: &AsterVault,
    hits: &[Hit],
) -> CliResult<BTreeMap<CxId, Constellation>> {
    let snapshot = vault.snapshot();
    let mut docs = BTreeMap::new();
    for hit in hits {
        let cx_id = hit.cx_id;
        let cx = vault.get(cx_id, snapshot).map_err(|error| {
            if error.code == "CALYX_STALE_DERIVED" && error.message.contains("missing") {
                missing_provenance(format!("stored constellation missing for hit {cx_id}"))
            } else {
                error
            }
        })?;
        docs.insert(cx_id, cx);
    }
    Ok(docs)
}

pub(crate) fn attach_verified_provenance(
    hits: &mut [Hit],
    docs: &BTreeMap<CxId, Constellation>,
    vault_dir: &Path,
    seq: u64,
) -> CliResult {
    let ledger = VerifiedLedger::open(vault_dir)?;
    for hit in hits {
        let cx = docs.get(&hit.cx_id).ok_or_else(|| {
            missing_provenance(format!(
                "stored constellation missing for hit {}",
                hit.cx_id
            ))
        })?;
        hit.provenance = ledger.require_ref(hit.cx_id, cx.provenance.clone())?;
        hit.provenance_source = ProvenanceSource::Stored;
        hit.freshness = FreshnessTag::fresh(seq);
    }
    Ok(())
}

struct VerifiedLedger {
    entries: BTreeMap<u64, calyx_ledger::LedgerEntry>,
}

impl VerifiedLedger {
    fn open(vault_dir: &Path) -> CliResult<Self> {
        let store = AsterLedgerCfStore::open(vault_dir).map_err(|error| {
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

    fn require_ref(&self, cx_id: CxId, expected: LedgerRef) -> CliResult<LedgerRef> {
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
