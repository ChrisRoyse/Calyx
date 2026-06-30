use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use calyx_aster::ledger_view::read_ledger_seqs;
use calyx_aster::mvcc::Snapshot;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Constellation, CxId, LedgerRef};
use calyx_ledger::{EntryKind, LedgerEntry, SubjectId, decode};
use calyx_sextant::{
    CALYX_SEXTANT_PROVENANCE_MISSING, FreshnessTag, Hit, ProvenanceSource, sextant_error,
};
use serde_json::Value;

use crate::error::CliResult;

#[cfg(test)]
mod tests;

pub(crate) fn hit_docs_at(
    vault: &AsterVault,
    hits: &[Hit],
    snapshot: Snapshot,
    hydrate_slots: bool,
) -> CliResult<BTreeMap<CxId, Constellation>> {
    let mut docs = BTreeMap::new();
    for hit in hits {
        let cx_id = hit.cx_id;
        let read = if hydrate_slots {
            vault.get_at_snapshot(cx_id, snapshot)
        } else {
            vault.get_base_at_snapshot(cx_id, snapshot)
        };
        let cx = read.map_err(|error| {
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
    let mut ledger = TargetedLedgerVerifier::open(vault_dir, hits, docs)?;
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

struct TargetedLedgerVerifier {
    rows: BTreeMap<u64, calyx_ledger::LedgerRow>,
    entries: BTreeMap<u64, LedgerEntry>,
}

impl TargetedLedgerVerifier {
    fn open(
        vault_dir: &Path,
        hits: &[Hit],
        docs: &BTreeMap<CxId, Constellation>,
    ) -> CliResult<Self> {
        let mut required = BTreeSet::new();
        for hit in hits {
            let cx = docs.get(&hit.cx_id).ok_or_else(|| {
                missing_provenance(format!(
                    "stored constellation missing for hit {}",
                    hit.cx_id
                ))
            })?;
            required.insert(cx.provenance.seq);
            if cx.provenance.seq > 0 {
                required.insert(cx.provenance.seq - 1);
            }
        }
        Ok(Self {
            rows: read_ledger_seqs(vault_dir, &required)?,
            entries: BTreeMap::new(),
        })
    }

    fn require_ref(&mut self, cx_id: CxId, expected: LedgerRef) -> CliResult<LedgerRef> {
        let entry = self.entry(cx_id, expected.seq)?;
        let entry_hash = entry.entry_hash;
        if entry.entry_hash != expected.hash {
            return Err(CalyxError::ledger_corrupt(format!(
                "search hit {cx_id} ledger seq {} hash does not match Base provenance",
                expected.seq
            ))
            .into());
        }
        if !entry_covers_cx(entry, cx_id)? {
            return Err(CalyxError::ledger_corrupt(format!(
                "search hit {cx_id} ledger seq {} subject mismatch",
                expected.seq
            ))
            .into());
        }
        self.require_chain_link(cx_id, expected.seq, entry_hash)?;
        Ok(expected)
    }

    fn entry(&mut self, cx_id: CxId, seq: u64) -> CliResult<&LedgerEntry> {
        if !self.entries.contains_key(&seq) {
            let bytes = self
                .rows
                .get(&seq)
                .ok_or_else(|| {
                    missing_provenance(format!(
                        "search hit {cx_id} references missing ledger seq {seq}"
                    ))
                })?
                .clone()
                .bytes;
            let entry = decode(&bytes).map_err(|error| {
                CalyxError::ledger_chain_broken(format!(
                    "search hit {cx_id} ledger seq {seq} is unreadable: {}",
                    error.message
                ))
            })?;
            if entry.seq != seq {
                return Err(CalyxError::ledger_corrupt(format!(
                    "search hit {cx_id} ledger row decoded seq {} != requested seq {seq}",
                    entry.seq
                ))
                .into());
            }
            self.entries.insert(seq, entry);
        }
        Ok(self
            .entries
            .get(&seq)
            .expect("targeted ledger entry inserted before lookup"))
    }

    fn require_chain_link(&mut self, cx_id: CxId, seq: u64, entry_hash: [u8; 32]) -> CliResult {
        if seq == 0 {
            let entry = self.entry(cx_id, seq)?;
            if entry.prev_hash != [0; 32] {
                return Err(CalyxError::ledger_chain_broken(format!(
                    "search hit {cx_id} ledger seq 0 prev_hash is not the genesis hash"
                ))
                .into());
            }
            return Ok(());
        }
        let previous = self.entry(cx_id, seq - 1)?;
        let previous_hash = previous.entry_hash;
        let entry = self.entry(cx_id, seq)?;
        if entry.prev_hash != previous_hash {
            return Err(CalyxError::ledger_chain_broken(format!(
                "search hit {cx_id} ledger seq {seq} prev_hash does not match seq {} entry_hash",
                seq - 1
            ))
            .into());
        }
        if entry.entry_hash != entry_hash {
            return Err(CalyxError::ledger_chain_broken(format!(
                "search hit {cx_id} ledger seq {seq} changed during targeted verification"
            ))
            .into());
        }
        Ok(())
    }
}

fn missing_provenance(message: impl Into<String>) -> CalyxError {
    sextant_error(CALYX_SEXTANT_PROVENANCE_MISSING, message)
}

fn entry_covers_cx(entry: &LedgerEntry, cx_id: CxId) -> CliResult<bool> {
    if entry.subject == SubjectId::Cx(cx_id) {
        return Ok(true);
    }
    if entry.kind != EntryKind::Ingest {
        return Ok(false);
    }
    batch_ingest_payload_contains_cx(entry, cx_id)
}

fn batch_ingest_payload_contains_cx(entry: &LedgerEntry, cx_id: CxId) -> CliResult<bool> {
    let payload = serde_json::from_slice::<Value>(&entry.payload).map_err(|error| {
        CalyxError::ledger_corrupt(format!(
            "ingest ledger seq {} subject mismatch and payload is invalid JSON: {error}",
            entry.seq
        ))
    })?;
    let ids = payload
        .get("cx_id")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CalyxError::ledger_corrupt(format!(
                "ingest ledger seq {} subject mismatch and payload missing cx_id array",
                entry.seq
            ))
        })?;
    Ok(ids
        .iter()
        .any(|value| value.as_str() == Some(&cx_id.to_string())))
}
