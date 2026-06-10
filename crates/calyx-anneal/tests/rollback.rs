use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::thread;

use calyx_anneal::{
    ArtifactKey, ArtifactPtr, CALYX_ANNEAL_CHANGE_COMMITTED, CALYX_ANNEAL_INVALID_ROLLBACK_STATE,
    CALYX_ANNEAL_UNKNOWN_CHANGE_ID, ChangeId, RollbackStorage, RollbackStore, rollback_live_key,
};
use calyx_core::{CalyxError, FixedClock, Result, Seq};
use proptest::prelude::*;

const WAL_SYNC: &str = "CALYX_ASTER_WAL_SYNC";

#[test]
fn prepare_promote_rollback_restores_prior_pointer_bytes() {
    let clock = FixedClock::new(1_785_500_396);
    let storage = MemoryStorage::default();
    let store = RollbackStore::open(&clock, 42, storage.clone()).unwrap();
    let key = key(1);
    let prior = ptr(2);
    let candidate = ptr(3);

    store.install_live_ptr(key.clone(), prior.clone()).unwrap();
    let live_before = storage.get(&rollback_live_key(&key)).unwrap().unwrap();
    let change_id = store
        .prepare_with_description(key.clone(), candidate.clone(), "known rollback")
        .unwrap();
    store.promote(change_id).unwrap();
    assert_eq!(store.live_ptr(&key).unwrap(), Some(candidate));

    store.rollback(change_id).unwrap();

    let readback = store.readback(change_id).unwrap();
    assert_eq!(store.live_ptr(&key).unwrap(), Some(prior));
    assert_eq!(readback.live_bytes, live_before);
    assert!(readback.snapshot.promoted);
    assert!(readback.snapshot.reverted);
    assert!(!readback.snapshot.committed);
}

#[test]
fn rollback_after_commit_fails_closed() {
    let clock = FixedClock::new(1_785_500_396);
    let store = RollbackStore::open(&clock, 7, MemoryStorage::default()).unwrap();
    let key = key(4);
    store.install_live_ptr(key.clone(), ptr(5)).unwrap();
    let id = store.prepare(key, ptr(6)).unwrap();
    store.promote(id).unwrap();
    store.commit(id).unwrap();

    let err = store.rollback(id).unwrap_err();

    assert_eq!(err.code, CALYX_ANNEAL_CHANGE_COMMITTED);
}

#[test]
fn unknown_and_empty_store_fail_closed() {
    let clock = FixedClock::new(1_785_500_396);
    let store = RollbackStore::open(&clock, 0, MemoryStorage::default()).unwrap();

    let err = store.rollback(ChangeId(99)).unwrap_err();

    assert_eq!(err.code, CALYX_ANNEAL_UNKNOWN_CHANGE_ID);
    assert_eq!(store.live_ptr(&key(1)).unwrap(), None);
}

#[test]
fn concurrent_promote_and_rollback_on_different_keys_are_independent() {
    let clock = FixedClock::new(1_785_500_396);
    let store = Arc::new(RollbackStore::open(&clock, 100, MemoryStorage::default()).unwrap());
    let key_a = key(10);
    let key_b = key(20);
    store.install_live_ptr(key_a.clone(), ptr(11)).unwrap();
    store.install_live_ptr(key_b.clone(), ptr(21)).unwrap();
    let id_a = store.prepare(key_a.clone(), ptr(12)).unwrap();
    let id_b = store.prepare(key_b.clone(), ptr(22)).unwrap();

    thread::scope(|scope| {
        let promote = store.clone();
        let rollback = store.clone();
        let left = scope.spawn(move || promote.promote(id_a));
        let right = scope.spawn(move || rollback.rollback(id_b));
        left.join().unwrap().unwrap();
        right.join().unwrap().unwrap();
    });

    assert_eq!(store.live_ptr(&key_a).unwrap(), Some(ptr(12)));
    assert_eq!(store.live_ptr(&key_b).unwrap(), Some(ptr(21)));
}

#[test]
fn prepare_wal_failure_propagates_without_partial_snapshot() {
    let clock = FixedClock::new(1_785_500_396);
    let storage = MemoryStorage::default();
    let store = RollbackStore::open(&clock, 4, storage.clone()).unwrap();
    let rollback_key = key(30);
    store
        .install_live_ptr(rollback_key.clone(), ptr(31))
        .unwrap();
    storage.fail_next(WAL_SYNC);

    let err = store.prepare(rollback_key.clone(), ptr(32)).unwrap_err();

    assert_eq!(err.code, WAL_SYNC);
    assert_eq!(store.live_ptr(&rollback_key).unwrap(), Some(ptr(31)));
    assert!(
        storage
            .scan()
            .unwrap()
            .iter()
            .all(|(k, _)| !k.starts_with(b"change:"))
    );
}

#[test]
fn prepare_without_live_pointer_fails_closed() {
    let clock = FixedClock::new(1_785_500_396);
    let store = RollbackStore::open(&clock, 0, MemoryStorage::default()).unwrap();

    let err = store.prepare(key(40), ptr(41)).unwrap_err();

    assert_eq!(err.code, CALYX_ANNEAL_INVALID_ROLLBACK_STATE);
}

proptest! {
    #[test]
    fn live_pointer_stays_defined_across_prepare_promote_rollback(ops in proptest::collection::vec(0u8..3, 1..64)) {
        let clock = FixedClock::new(1_785_500_396);
        let store = RollbackStore::open(&clock, 99, MemoryStorage::default()).unwrap();
        let rollback_key = key(50);
        let prior = ptr(51);
        store.install_live_ptr(rollback_key.clone(), prior.clone()).unwrap();
        let mut ids = Vec::new();
        let mut known = vec![prior];
        let mut next = 52u8;

        for op in ops {
            match op {
                0 => {
                    let candidate = ptr(next);
                    next = next.wrapping_add(1);
                    ids.push(store.prepare(rollback_key.clone(), candidate.clone()).unwrap());
                    known.push(candidate);
                }
                1 => {
                    if let Some(id) = ids.last().copied() {
                        let _ = store.promote(id);
                    }
                }
                _ => {
                    if let Some(id) = ids.last().copied() {
                        let _ = store.rollback(id);
                    }
                }
            }
            let live = store.live_ptr(&rollback_key).unwrap();
            prop_assert!(live.as_ref().is_some_and(|ptr| known.contains(ptr)));
        }
    }
}

#[derive(Clone, Default)]
struct MemoryStorage {
    inner: Arc<Mutex<MemoryInner>>,
}

#[derive(Default)]
struct MemoryInner {
    rows: BTreeMap<Vec<u8>, Vec<u8>>,
    fail_next: Option<CalyxError>,
}

impl MemoryStorage {
    fn fail_next(&self, code: &'static str) {
        self.inner.lock().unwrap().fail_next = Some(CalyxError {
            code,
            message: "injected rollback storage failure".to_string(),
            remediation: "retry after WAL sync succeeds",
        });
    }
}

impl RollbackStorage for MemoryStorage {
    fn put_many(&self, rows: Vec<(Vec<u8>, Vec<u8>)>) -> Result<Seq> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(error) = inner.fail_next.take() {
            return Err(error);
        }
        for (key, value) in rows {
            inner.rows.insert(key, value);
        }
        Ok(inner.rows.len() as Seq)
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.inner.lock().unwrap().rows.get(key).cloned())
    }

    fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .rows
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect())
    }
}

fn key(byte: u8) -> ArtifactKey {
    ArtifactKey::ConfigCache([byte; 32])
}

fn ptr(byte: u8) -> ArtifactPtr {
    ArtifactPtr::ConfigCacheKeyHash([byte; 32])
}
