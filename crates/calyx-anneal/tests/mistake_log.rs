use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use calyx_anneal::{
    AsterMistakeStorage, CALYX_ANNEAL_INVALID_WINDOW, CALYX_ANNEAL_MISTAKE_INVALID_ROW,
    CALYX_ASTER_CF_UNAVAILABLE, MistakeLog, MistakeStorage, decode_mistake_entry, mistake_key,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{AnchorKind, CalyxError, Clock, CxId, FixedClock, Result};
use proptest::prelude::*;

mod fsv_support;
use fsv_support::vault_id;

#[test]
fn append_three_entries_computes_surprise_and_rate() {
    let log = memory_log(3, 100);
    let cx = cx(1);

    let first = log.append(cx, 0.9, 0.1, AnchorKind::Reward).unwrap();
    let second = log.append(cx, 0.8, 0.7, AnchorKind::Reward).unwrap();
    let third = log.append(cx, 0.5, 0.5, AnchorKind::Reward).unwrap();

    assert_eq!(first.seq, 1);
    assert_eq!(first.surprise, 0.8);
    assert!((second.surprise - 0.1).abs() < 1e-12);
    assert_eq!(third.surprise, 0.0);
    assert!((log.mistake_rate(3).unwrap() - (1.0 / 3.0)).abs() < 1e-12);
    assert_eq!(
        log.mistake_rate_default_window().unwrap(),
        log.mistake_rate(3).unwrap()
    );
}

#[test]
fn recent_returns_tail_in_insertion_order() {
    let log = memory_log(3, 101);
    for id in 1..=3 {
        log.append(cx(id), id as f64, 0.0, AnchorKind::Reward)
            .unwrap();
    }

    let recent = log.recent(2).unwrap();
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].cx_id, cx(2));
    assert_eq!(recent[1].cx_id, cx(3));
    assert_eq!(log.recent(100).unwrap().len(), 3);
}

#[test]
fn empty_and_invalid_window_edges_fail_closed() {
    let log = memory_log(10, 102);

    assert_eq!(log.mistake_rate(10).unwrap(), 0.0);
    assert!(log.recent(5).unwrap().is_empty());
    let err = log.mistake_rate(0).unwrap_err();
    assert_eq!(err.code, CALYX_ANNEAL_INVALID_WINDOW);
}

#[test]
fn non_finite_values_do_not_append() {
    let log = memory_log(10, 103);

    let err = log
        .append(cx(1), f64::NAN, 0.1, AnchorKind::Reward)
        .unwrap_err();

    assert_eq!(err.code, CALYX_ANNEAL_MISTAKE_INVALID_ROW);
    assert!(log.recent(1).unwrap().is_empty());
}

#[test]
fn storage_write_failure_returns_cf_error_but_keeps_session_entry() {
    let storage = MemoryMistakeStorage::default();
    storage.fail_next_put.store(true, Ordering::SeqCst);
    let clock: Arc<dyn Clock> = Arc::new(FixedClock::new(104));
    let log = MistakeLog::open(storage, 10, clock).unwrap();

    let err = log.append(cx(1), 1.0, 0.0, AnchorKind::Reward).unwrap_err();

    assert_eq!(err.code, CALYX_ASTER_CF_UNAVAILABLE);
    let recent = log.recent(1).unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].surprise, 1.0);
}

#[test]
fn aster_storage_writes_cbor_row_under_anneal_mistakes_cf() {
    let vault = AsterVault::with_clock(vault_id(), b"issue406-mistake-log", FixedClock::new(105));
    let storage = AsterMistakeStorage::new(&vault);
    let clock: Arc<dyn Clock> = Arc::new(FixedClock::new(105));
    let log = MistakeLog::open(storage, 10, clock).unwrap();

    log.append(cx(9), 0.9, 0.1, AnchorKind::Label("gold".to_string()))
        .unwrap();

    let rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealMistakes)
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, mistake_key(1));
    let decoded = decode_mistake_entry(&rows[0].1).unwrap();
    assert_eq!(decoded.cx_id, cx(9));
    assert_eq!(decoded.ts, 105);
    assert_eq!(decoded.surprise, 0.8);
}

proptest! {
    #[test]
    fn mistake_rate_is_always_bounded(
        values in proptest::collection::vec((0.0f64..1.0, 0.0f64..1.0), 0..30),
        window in 1usize..50,
    ) {
        let log = memory_log(10, 106);
        for (predicted, observed) in values {
            log.append(cx(1), predicted, observed, AnchorKind::Reward).unwrap();
        }
        let rate = log.mistake_rate(window).unwrap();
        prop_assert!((0.0..=1.0).contains(&rate));
    }
}

#[derive(Clone, Default)]
struct MemoryMistakeStorage {
    rows: Arc<Mutex<BTreeMap<u64, Vec<u8>>>>,
    fail_next_put: Arc<AtomicBool>,
}

impl MistakeStorage for MemoryMistakeStorage {
    fn put_new(&self, seq: u64, value: &[u8]) -> Result<()> {
        if self.fail_next_put.swap(false, Ordering::SeqCst) {
            return Err(CalyxError {
                code: CALYX_ASTER_CF_UNAVAILABLE,
                message: "injected anneal_mistakes CF outage".to_string(),
                remediation: "restore test storage",
            });
        }
        let mut rows = self.rows.lock().unwrap();
        if rows.insert(seq, value.to_vec()).is_some() {
            return Err(CalyxError::ledger_append_only_violation("duplicate seq"));
        }
        Ok(())
    }

    fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let rows = self.rows.lock().unwrap();
        Ok(rows
            .iter()
            .map(|(seq, value)| (mistake_key(*seq), value.clone()))
            .collect())
    }
}

fn memory_log(window_size: usize, ts: u64) -> MistakeLog<MemoryMistakeStorage> {
    let clock: Arc<dyn Clock> = Arc::new(FixedClock::new(ts));
    MistakeLog::open(MemoryMistakeStorage::default(), window_size, clock).unwrap()
}

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}
