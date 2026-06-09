use super::*;
use calyx_core::{Clock, CxId};
use proptest::prelude::*;
use std::collections::VecDeque;
use std::sync::Mutex;

#[test]
fn appender_clamps_repeated_clock_values() {
    let mut appender = sample_appender([1000, 1000, 1001]);

    append_sample(&mut appender, 1).unwrap();
    append_sample(&mut appender, 2).unwrap();
    append_sample(&mut appender, 3).unwrap();

    let ts = entry_ts(&appender);
    assert_eq!(ts, vec![1000, 1001, 1002]);
    assert_eq!(appender.last_ts(), 1002);
}

#[test]
fn appender_recovers_last_ts_across_restart() {
    let mut first = sample_appender([5000]);
    append_sample(&mut first, 1).unwrap();
    let store = first.into_store();

    let mut reopened =
        LedgerAppender::open(store, SequenceClock::new([4999])).expect("reopen appender");
    append_sample(&mut reopened, 2).unwrap();

    assert_eq!(entry_ts(&reopened), vec![5000, 5001]);
    assert_eq!(reopened.last_ts(), 5001);
}

#[test]
fn prepared_entry_does_not_mutate_store_until_committed() {
    let mut appender = sample_appender([44]);

    let prepared = appender
        .prepare(
            EntryKind::Ingest,
            sample_subject(1),
            b"{}".to_vec(),
            ActorId::Service("svc".to_string()),
        )
        .expect("prepare entry");

    assert_eq!(prepared.seq(), 0);
    assert_eq!(appender.next_seq(), 0);
    assert_eq!(appender.prev_hash(), [0; HASH_BYTES]);
    assert!(appender.store().scan().unwrap().is_empty());

    let ledger_ref = appender.commit_prepared(&prepared).expect("commit");

    assert_eq!(ledger_ref, prepared.ledger_ref());
    assert_eq!(appender.next_seq(), 1);
    assert_eq!(appender.scan_entries().unwrap().len(), 1);
}

#[test]
fn actor_length_edges_fail_closed() {
    assert!(ActorId::Agent(String::new()).validate().is_ok());
    assert!(ActorId::Agent("x".repeat(64)).validate().is_ok());
    assert_eq!(
        ActorId::Agent("x".repeat(65)).validate().unwrap_err().code,
        "CALYX_LEDGER_ACTOR_TOO_LONG"
    );

    let mut appender = sample_appender([1]);
    let error = appender
        .append(
            EntryKind::Ingest,
            sample_subject(1),
            b"{}".to_vec(),
            ActorId::Agent("x".repeat(65)),
        )
        .unwrap_err();
    assert_eq!(error.code, "CALYX_LEDGER_ACTOR_TOO_LONG");
    assert!(appender.scan_entries().unwrap().is_empty());
}

#[test]
fn recovered_zero_ts_still_clamps_forward() {
    let mut store = MemoryLedgerStore::default();
    let entry = LedgerEntry::new(
        0,
        [0; HASH_BYTES],
        EntryKind::Ingest,
        sample_subject(1),
        b"{}".to_vec(),
        ActorId::Service("svc".to_string()),
        0,
    );
    store.insert_raw(0, encode(&entry));

    let mut reopened =
        LedgerAppender::open(store, SequenceClock::new([0])).expect("reopen appender");
    append_sample(&mut reopened, 2).unwrap();

    assert_eq!(entry_ts(&reopened), vec![0, 1]);
}

proptest! {
    #[test]
    fn appender_timestamps_are_monotone_for_any_clock_values(
        values in proptest::collection::vec(0_u64..(u64::MAX - 32), 1..16),
    ) {
        let mut appender = LedgerAppender::open(
            MemoryLedgerStore::default(),
            SequenceClock::new(values.clone()),
        ).expect("open appender");

        for index in 0..values.len() {
            append_sample(&mut appender, index as u8).unwrap();
        }

        let ts = entry_ts(&appender);
        prop_assert!(ts.windows(2).all(|pair| pair[0] < pair[1]));
    }
}

fn sample_appender<const N: usize>(
    values: [u64; N],
) -> LedgerAppender<MemoryLedgerStore, SequenceClock> {
    LedgerAppender::open(MemoryLedgerStore::default(), SequenceClock::new(values))
        .expect("open appender")
}

fn append_sample(
    appender: &mut LedgerAppender<MemoryLedgerStore, SequenceClock>,
    seed: u8,
) -> Result<LedgerRef> {
    appender.append(
        EntryKind::Ingest,
        sample_subject(seed),
        b"{}".to_vec(),
        ActorId::Service("svc".to_string()),
    )
}

fn sample_subject(seed: u8) -> SubjectId {
    SubjectId::Cx(CxId::from_bytes([seed; 16]))
}

fn entry_ts<C: Clock>(appender: &LedgerAppender<MemoryLedgerStore, C>) -> Vec<u64> {
    appender
        .scan_entries()
        .unwrap()
        .into_iter()
        .map(|entry| entry.ts)
        .collect()
}

#[derive(Debug)]
struct SequenceClock {
    values: Mutex<VecDeque<u64>>,
    fallback: u64,
}

impl SequenceClock {
    fn new(values: impl IntoIterator<Item = u64>) -> Self {
        let values = values.into_iter().collect::<VecDeque<_>>();
        let fallback = values.back().copied().unwrap_or(0);
        Self {
            values: Mutex::new(values),
            fallback,
        }
    }
}

impl Clock for SequenceClock {
    fn now(&self) -> u64 {
        self.values
            .lock()
            .expect("clock lock")
            .pop_front()
            .unwrap_or(self.fallback)
    }
}
