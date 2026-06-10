use std::ops::Range;

use calyx_core::FixedClock;
use calyx_ledger::{
    ActorId, EntryKind, LedgerAppender, LedgerCfStore, MemoryLedgerStore, SubjectId, VerifyResult,
    verify_chain,
};

use super::common::cx;

pub fn memory_chain(count: usize) -> MemoryLedgerStore {
    let mut appender =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(42)).unwrap();
    for seq in 0..count {
        appender
            .append(
                EntryKind::Ingest,
                SubjectId::Cx(cx(seq as u8)),
                format!("payload-{seq}").into_bytes(),
                ActorId::Service("ph36-fsv-unit".to_string()),
            )
            .unwrap();
    }
    appender.into_store()
}

pub fn mutate_row(store: &mut MemoryLedgerStore, seq: u64, offset: usize) {
    let mut row = store
        .scan()
        .unwrap()
        .into_iter()
        .find(|row| row.seq == seq)
        .unwrap();
    row.bytes[offset] ^= 1;
    store.insert_raw(seq, row.bytes);
}

pub fn mutate_row_from_end(store: &mut MemoryLedgerStore, seq: u64, offset_from_end: usize) {
    let mut row = store
        .scan()
        .unwrap()
        .into_iter()
        .find(|row| row.seq == seq)
        .unwrap();
    let offset = row.bytes.len() - offset_from_end;
    row.bytes[offset] ^= 1;
    store.insert_raw(seq, row.bytes);
}

pub fn broken_at(store: &MemoryLedgerStore, range: Range<u64>) -> u64 {
    match verify_chain(store, range).unwrap() {
        VerifyResult::Broken { at_seq, .. } | VerifyResult::Corrupt { at_seq, .. } => at_seq,
        VerifyResult::Intact { .. } => panic!("expected broken chain"),
    }
}
