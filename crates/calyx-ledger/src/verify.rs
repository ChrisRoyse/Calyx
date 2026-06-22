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
    Corrupt {
        at_seq: u64,
        reason: String,
    },
}

impl VerifyResult {
    pub fn quarantine_seq(&self) -> Option<u64> {
        match self {
            Self::Intact { .. } => None,
            Self::Broken { at_seq, .. } | Self::Corrupt { at_seq, .. } => Some(*at_seq),
        }
    }
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
    let anchor = store.head_anchor()?;
    if range.start == 0
        && let Some(anchor) = &anchor
        && range.end != anchor.height
    {
        return Ok(corrupt_result(
            range.end.min(anchor.height),
            format!(
                "ledger head anchor mismatch: requested head {}, anchored head {}",
                range.end, anchor.height
            ),
        ));
    }
    let rows = store
        .scan()?
        .into_iter()
        .map(|row| (row.seq, row.bytes))
        .collect::<BTreeMap<_, _>>();
    let mut expected_prev = match expected_prev_hash(&rows, range.start)? {
        StartHash::Ready(hash) => hash,
        StartHash::Corrupt(result) => return Ok(result),
    };
    let mut count = 0_u64;

    for seq in range.clone() {
        let Some(bytes) = rows.get(&seq) else {
            return Ok(corrupt_result(
                seq,
                format!("missing ledger row for seq {seq}"),
            ));
        };
        let entry = match decode_unchecked(bytes) {
            Ok(entry) => entry,
            Err(error) => {
                return Ok(corrupt_result(
                    seq,
                    format!("decode ledger row seq {seq}: {error}"),
                ));
            }
        };
        if entry.seq != seq {
            return Ok(corrupt_result(
                seq,
                format!("ledger key seq {seq} != encoded seq {}", entry.seq),
            ));
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

    if range.start == 0
        && let Some(anchor) = &anchor
        && range.end == anchor.height
        && expected_prev != anchor.tip_hash
    {
        return Ok(VerifyResult::Broken {
            at_seq: range.end.saturating_sub(1),
            expected: anchor.tip_hash,
            found: expected_prev,
        });
    }

    Ok(VerifyResult::Intact { count })
}

enum StartHash {
    Ready([u8; HASH_BYTES]),
    Corrupt(VerifyResult),
}

fn expected_prev_hash(rows: &BTreeMap<u64, Vec<u8>>, start: u64) -> Result<StartHash> {
    if start == 0 {
        return Ok(StartHash::Ready([0; HASH_BYTES]));
    }
    let previous_seq = start - 1;
    let Some(bytes) = rows.get(&previous_seq) else {
        return Ok(StartHash::Corrupt(corrupt_result(
            start,
            format!("missing ledger row for previous seq {previous_seq}"),
        )));
    };
    let entry = match decode_unchecked(bytes) {
        Ok(entry) => entry,
        Err(error) => {
            return Ok(StartHash::Corrupt(corrupt_result(
                start,
                format!("cannot verify range start {start}: previous seq {previous_seq}: {error}"),
            )));
        }
    };
    if entry.seq != previous_seq {
        return Ok(StartHash::Corrupt(corrupt_result(
            start,
            format!(
                "previous key seq {previous_seq} != encoded seq {}",
                entry.seq
            ),
        )));
    }
    if !entry.verify() {
        return Ok(StartHash::Corrupt(corrupt_result(
            start,
            format!("cannot verify range start {start}: previous seq {previous_seq} is broken"),
        )));
    }
    Ok(StartHash::Ready(entry.entry_hash))
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

fn corrupt_result(at_seq: u64, reason: impl Into<String>) -> VerifyResult {
    VerifyResult::Corrupt {
        at_seq,
        reason: reason.into(),
    }
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
    fn empty_zero_range_skips_head_anchor_check() {
        let store = chain_store(3);

        assert_eq!(
            verify_chain(&store, 0..0).unwrap(),
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

    #[test]
    fn newest_row_truncation_reports_anchor_mismatch() {
        let mut store = chain_store(3);
        store.remove_raw(2);

        let result = verify_chain(&store, 0..2).unwrap();

        assert!(matches!(
            result,
            VerifyResult::Corrupt {
                at_seq: 2,
                ref reason
            } if reason.contains("anchored head 3")
        ));
    }

    #[test]
    fn appender_recovery_rejects_newest_row_truncation() {
        let mut store = chain_store(3);
        store.remove_raw(2);

        let err = LedgerAppender::open(store, FixedClock::new(20)).unwrap_err();

        assert_eq!(err.code, "CALYX_LEDGER_CHAIN_BROKEN");
        assert!(err.message.contains("end-truncated"));
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
    fn missing_row_reports_corrupt_result() {
        let mut store = chain_store(3);
        remove_row(&mut store, 1);

        assert!(matches!(
            verify_chain(&store, 0..3).unwrap(),
            VerifyResult::Corrupt { at_seq: 1, .. }
        ));
    }

    #[test]
    fn truncated_row_reports_corrupt_result() {
        let mut store = chain_store(3);
        mutate_row(&mut store, 1, |bytes| bytes.truncate(12));

        let result = verify_chain(&store, 0..3).unwrap();

        assert!(matches!(result, VerifyResult::Corrupt { at_seq: 1, .. }));
        assert_eq!(result.quarantine_seq(), Some(1));
    }

    #[test]
    fn missing_previous_row_reports_range_start_corrupt_result() {
        let mut store = chain_store(3);
        remove_row(&mut store, 1);

        let result = verify_chain(&store, 2..3).unwrap();

        assert!(matches!(
            result,
            VerifyResult::Corrupt {
                at_seq: 2,
                ref reason
            } if reason.contains("previous seq 1")
        ));
    }

    #[test]
    fn encoded_seq_mismatch_reports_corrupt_result() {
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

        let result = verify_chain(&store, 0..1).unwrap();

        assert!(matches!(
            result,
            VerifyResult::Corrupt {
                at_seq: 0,
                ref reason
            } if reason.contains("encoded seq 3")
        ));
    }

    fn remove_row(store: &mut MemoryLedgerStore, seq_to_remove: u64) {
        let rows = store.scan().unwrap();
        let mut filtered = MemoryLedgerStore::default();
        for LedgerRow { seq, bytes } in rows {
            if seq != seq_to_remove {
                filtered.insert_raw(seq, bytes);
            }
        }
        *store = filtered;
    }
}
