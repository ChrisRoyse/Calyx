use std::sync::{Arc, Mutex};

use calyx_anneal::{
    AsterMistakeStorage, AsterReplayStorage, CALYX_ANNEAL_INVALID_CAPACITY, MistakeLog, MistakeRef,
    ReplayBuffer, ReplayEntry, ReplayStorage, decode_replay_snapshot, replay_snapshot_key,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{AnchorKind, Clock, CxId, FixedClock, Result};
use proptest::prelude::*;

mod fsv_support;
use fsv_support::vault_id;

#[test]
fn capacity_evicts_lowest_surprise() {
    let mut buffer = memory_buffer(2, 200);

    assert!(buffer.push(entry(1, 0.8, 1)).unwrap());
    assert!(buffer.push(entry(2, 0.3, 2)).unwrap());
    assert!(!buffer.push(entry(3, 0.1, 3)).unwrap());

    assert_eq!(buffer.len(), 2);
    assert_eq!(buffer.top_surprises(5), vec![0.8, 0.3]);
    let entries = buffer.entries_by_priority();
    assert_eq!(entries[0].cx_id, cx(1));
    assert_eq!(entries[1].cx_id, cx(2));
}

#[test]
fn sample_batch_is_seeded_and_read_only() {
    let mut buffer = memory_buffer(5, 201);
    for (idx, surprise) in [0.1, 0.2, 0.3, 0.4, 0.5].into_iter().enumerate() {
        buffer
            .push(entry((idx + 1) as u8, surprise, (idx + 1) as u64))
            .unwrap();
    }

    let first = buffer.sample_batch(2, 42);
    let second = buffer.sample_batch(2, 42);
    let third = buffer.sample_batch(2, 7);

    assert_eq!(first, second);
    assert_ne!(first, third);
    assert_eq!(buffer.len(), 5);
}

#[test]
fn capacity_one_replaces_only_on_higher_surprise() {
    let mut buffer = memory_buffer(1, 202);

    assert!(buffer.push(entry(1, 0.2, 1)).unwrap());
    assert!(!buffer.push(entry(2, 0.2, 2)).unwrap());
    assert_eq!(buffer.entries_by_priority()[0].mistake_ref.seq, 1);
    assert!(buffer.push(entry(3, 0.9, 3)).unwrap());
    assert_eq!(buffer.entries_by_priority()[0].mistake_ref.seq, 3);
}

#[test]
fn empty_and_overwide_sampling_edges_are_safe() {
    let mut buffer = memory_buffer(3, 203);

    assert!(buffer.sample_batch(2, 42).is_empty());
    buffer.push(entry(1, 0.6, 1)).unwrap();
    buffer.push(entry(2, 0.4, 2)).unwrap();

    let all = buffer.sample_batch(10, 42);
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].surprise, 0.6);
    assert_eq!(all[1].surprise, 0.4);
}

#[test]
fn zero_capacity_fails_closed() {
    let clock: Arc<dyn Clock> = Arc::new(FixedClock::new(204));
    let err = ReplayBuffer::open(MemoryReplayStorage::default(), 0, clock)
        .err()
        .unwrap();

    assert_eq!(err.code, CALYX_ANNEAL_INVALID_CAPACITY);
}

#[test]
fn seed_from_log_replays_recent_mistakes_without_log_feedback() {
    let vault = AsterVault::with_clock(vault_id(), b"issue407-replay-seed", FixedClock::new(205));
    let clock: Arc<dyn Clock> = Arc::new(FixedClock::new(205));
    let log = MistakeLog::open(AsterMistakeStorage::new(&vault), 10, clock.clone()).unwrap();
    log.append(cx(1), 0.9, 0.1, AnchorKind::Reward).unwrap();
    log.append(cx(2), 0.7, 0.3, AnchorKind::Reward).unwrap();
    log.append(cx(3), 0.5, 0.5, AnchorKind::Reward).unwrap();
    let log_rows_before = log.readback_recent(10).unwrap().len();
    let mut buffer = ReplayBuffer::open(MemoryReplayStorage::default(), 2, clock).unwrap();

    let accepted = buffer.seed_from_log(&log, 3).unwrap();

    assert_eq!(accepted, 2);
    assert_eq!(buffer.top_surprises(5), vec![0.8, 0.39999999999999997]);
    assert_eq!(log.readback_recent(10).unwrap().len(), log_rows_before);
}

#[test]
fn aster_storage_writes_cbor_snapshot_under_anneal_replay_cf() {
    let vault = AsterVault::with_clock(vault_id(), b"issue407-replay", FixedClock::new(206));
    let storage = AsterReplayStorage::new(&vault);
    let clock: Arc<dyn Clock> = Arc::new(FixedClock::new(206));
    let mut buffer = ReplayBuffer::open(storage, 2, clock).unwrap();

    buffer.push(entry(9, 0.75, 1)).unwrap();

    let rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealReplay)
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, replay_snapshot_key());
    let snapshot = decode_replay_snapshot(&rows[0].1).unwrap();
    assert_eq!(snapshot.capacity, 2);
    assert_eq!(snapshot.entries.len(), 1);
    assert_eq!(snapshot.entries[0].cx_id, cx(9));
    assert_eq!(snapshot.entries[0].surprise, 0.75);
}

proptest! {
    #[test]
    fn len_never_exceeds_capacity(
        surprises in proptest::collection::vec(0.0f64..1.0, 0..80),
        capacity in 1usize..20,
    ) {
        let mut buffer = memory_buffer(capacity, 207);
        for (idx, surprise) in surprises.into_iter().enumerate() {
            buffer.push(entry((idx % 255) as u8, surprise, idx as u64 + 1)).unwrap();
            prop_assert!(buffer.len() <= capacity);
        }
    }
}

#[derive(Clone, Default)]
struct MemoryReplayStorage {
    snapshot: Arc<Mutex<Option<Vec<u8>>>>,
}

impl ReplayStorage for MemoryReplayStorage {
    fn load_snapshot(&self) -> Result<Option<Vec<u8>>> {
        Ok(self.snapshot.lock().unwrap().clone())
    }

    fn save_snapshot(&self, value: &[u8]) -> Result<()> {
        *self.snapshot.lock().unwrap() = Some(value.to_vec());
        Ok(())
    }
}

fn memory_buffer(capacity: usize, ts: u64) -> ReplayBuffer<MemoryReplayStorage> {
    let clock: Arc<dyn Clock> = Arc::new(FixedClock::new(ts));
    ReplayBuffer::open(MemoryReplayStorage::default(), capacity, clock).unwrap()
}

fn entry(byte: u8, surprise: f64, seq: u64) -> ReplayEntry {
    ReplayEntry::new(cx(byte), surprise, MistakeRef { seq, surprise }, 1000 + seq).unwrap()
}

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}
