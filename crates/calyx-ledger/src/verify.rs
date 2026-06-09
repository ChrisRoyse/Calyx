//! Ledger hash-chain verification.

use std::collections::BTreeMap;
use std::ops::Range;

use calyx_core::{CalyxError, Result};

use crate::append::LedgerCfStore;
use crate::codec::decode_unchecked;
use crate::entry::{HASH_BYTES, LedgerEntry, compute_entry_hash};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyResult {
    Intact {
        count: u64,
    },
    Broken {
        at_seq: u64,
        expected: [u8; HASH_BYTES],
        found: [u8; HASH_BYTES],
    },
}

pub fn verify_chain(store: &dyn LedgerCfStore, range: Range<u64>) -> Result<VerifyResult> {
    if range.start > range.end {
        return Err(CalyxError::ledger_corrupt(format!(
            "invalid ledger range {}..{}",
            range.start, range.end
        )));
    }
    if range.start == range.end {
        return Ok(VerifyResult::Intact { count: 0 });
    }

    let rows = store
        .scan()?
        .into_iter()
        .map(|row| (row.seq, row.bytes))
        .collect::<BTreeMap<_, _>>();
    let mut expected_prev = expected_prev_hash(&rows, range.start)?;
    let mut count = 0_u64;

    for seq in range.clone() {
        let Some(bytes) = rows.get(&seq) else {
            return Err(CalyxError::ledger_corrupt(format!(
                "missing ledger row for seq {seq}"
            )));
        };
        let entry = match decode_unchecked(bytes) {
            Ok(entry) => entry,
            Err(_) => {
                return Ok(VerifyResult::Broken {
                    at_seq: seq,
                    expected: expected_prev,
                    found: [0; HASH_BYTES],
                });
            }
        };
        if entry.seq != seq {
            return Err(CalyxError::ledger_corrupt(format!(
                "ledger key seq {seq} != encoded seq {}",
                entry.seq
            )));
        }
        if entry.prev_hash != expected_prev {
            return Ok(VerifyResult::Broken {
                at_seq: seq,
                expected: expected_prev,
                found: entry.prev_hash,
            });
        }
        let expected_entry_hash = recompute_hash(&entry);
        if entry.entry_hash != expected_entry_hash {
            return Ok(VerifyResult::Broken {
                at_seq: seq,
                expected: expected_entry_hash,
                found: entry.entry_hash,
            });
        }
        expected_prev = entry.entry_hash;
        count += 1;
    }

    Ok(VerifyResult::Intact { count })
}

fn expected_prev_hash(rows: &BTreeMap<u64, Vec<u8>>, start: u64) -> Result<[u8; HASH_BYTES]> {
    if start == 0 {
        return Ok([0; HASH_BYTES]);
    }
    let previous_seq = start - 1;
    let bytes = rows.get(&previous_seq).ok_or_else(|| {
        CalyxError::ledger_corrupt(format!(
            "missing ledger row for previous seq {previous_seq}"
        ))
    })?;
    let entry = decode_unchecked(bytes)?;
    if entry.seq != previous_seq || !entry.verify() {
        return Err(CalyxError::ledger_chain_broken(format!(
            "cannot verify range start {start}: previous seq {previous_seq} is broken"
        )));
    }
    Ok(entry.entry_hash)
}

fn recompute_hash(entry: &LedgerEntry) -> [u8; HASH_BYTES] {
    compute_entry_hash(
        entry.seq,
        &entry.prev_hash,
        entry.kind,
        &entry.subject,
        &entry.payload,
        &entry.actor,
        entry.ts,
    )
}

#[cfg(test)]
mod tests {
    use calyx_core::{CxId, FixedClock};

    use super::*;
    use crate::{
        ActorId, EntryKind, LedgerAppender, LedgerCfStore, LedgerEntry, LedgerRow,
        MemoryLedgerStore, SubjectId, encode,
    };

    #[test]
    fn intact_chain_reports_count() {
        let store = chain_store(10);

        assert_eq!(
            verify_chain(&store, 0..10).unwrap(),
            VerifyResult::Intact { count: 10 }
        );
    }

    #[test]
    fn empty_range_is_intact_zero() {
        let store = chain_store(1);

        assert_eq!(
            verify_chain(&store, 1..1).unwrap(),
            VerifyResult::Intact { count: 0 }
        );
    }

    #[test]
    fn wrong_genesis_prev_hash_breaks_at_zero() {
        let mut store = chain_store(1);
        mutate_row(&mut store, 0, |bytes| bytes[8] ^= 1);

        assert!(matches!(
            verify_chain(&store, 0..1).unwrap(),
            VerifyResult::Broken { at_seq: 0, .. }
        ));
    }

    #[test]
    fn corrupted_prev_hash_reports_that_seq() {
        let mut store = chain_store(10);
        mutate_row(&mut store, 5, |bytes| bytes[8] ^= 1);

        assert!(matches!(
            verify_chain(&store, 0..10).unwrap(),
            VerifyResult::Broken { at_seq: 5, .. }
        ));
    }

    #[test]
    fn corrupted_entry_hash_reports_that_seq() {
        let mut store = chain_store(10);
        mutate_row(&mut store, 5, |bytes| {
            let last = bytes.len() - 1;
            bytes[last] ^= 1;
        });

        assert!(matches!(
            verify_chain(&store, 0..10).unwrap(),
            VerifyResult::Broken { at_seq: 5, .. }
        ));
    }

    #[test]
    fn nonzero_range_checks_previous_link() {
        let store = chain_store(10);

        assert_eq!(
            verify_chain(&store, 4..7).unwrap(),
            VerifyResult::Intact { count: 3 }
        );
    }

    fn chain_store(count: usize) -> MemoryLedgerStore {
        let mut appender = LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(10))
            .expect("open appender");
        for seq in 0..count {
            appender
                .append(
                    EntryKind::Ingest,
                    SubjectId::Cx(CxId::from_bytes([seq as u8; 16])),
                    format!("payload-{seq}").into_bytes(),
                    ActorId::Service("verify-test".to_string()),
                )
                .expect("append entry");
        }
        appender.into_store()
    }

    fn mutate_row(store: &mut MemoryLedgerStore, seq: u64, mutate: impl FnOnce(&mut Vec<u8>)) {
        let mut rows = store.scan().unwrap();
        let row = rows
            .iter_mut()
            .find(|row| row.seq == seq)
            .expect("row to mutate");
        mutate(&mut row.bytes);
        let mut mutated = MemoryLedgerStore::default();
        for LedgerRow { seq, bytes } in rows {
            mutated.insert_raw(seq, bytes);
        }
        *store = mutated;
    }

    #[test]
    fn encoded_seq_mismatch_is_corrupt_not_quarantine() {
        let mut store = MemoryLedgerStore::default();
        let entry = LedgerEntry::new(
            3,
            [0; HASH_BYTES],
            EntryKind::Ingest,
            SubjectId::Cx(CxId::from_bytes([3; 16])),
            b"payload".to_vec(),
            ActorId::Service("verify-test".to_string()),
            10,
        );
        store.insert_raw(0, encode(&entry));

        let error = verify_chain(&store, 0..1).unwrap_err();

        assert_eq!(error.code, "CALYX_LEDGER_CORRUPT");
    }
}
