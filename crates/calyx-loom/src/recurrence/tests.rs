use std::collections::BTreeMap;

use calyx_aster::dedup::EpochSecs;
use calyx_aster::vault::AsterVault;
use calyx_core::{
    Constellation, CxFlags, FixedClock, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use proptest::prelude::*;

use crate::error::CALYX_RECURRENCE_CONTEXT_TOO_LARGE;

use super::*;

#[test]
fn append_three_occurrences_reads_sorted_with_cadence() {
    let (vault, cx_id) = vault_with_base();
    let store = SeriesStore::new(&vault);

    store
        .append_occurrence(cx_id, EpochSecs(300), ctx("c"))
        .expect("append 300");
    store
        .append_occurrence(cx_id, EpochSecs(100), ctx("a"))
        .expect("append 100");
    store
        .append_occurrence(cx_id, EpochSecs(200), ctx("b"))
        .expect("append 200");

    let series = store.read_series(cx_id).expect("read series");
    assert_eq!(times(&series), vec![100, 200, 300]);
    assert_eq!(series.cadence_secs, Some(100.0));
    assert_eq!(series.frequency, 3);
    assert_eq!(store.occurrence_count(cx_id).expect("count"), 3);
}

#[test]
fn single_occurrence_has_no_cadence() {
    let (vault, cx_id) = vault_with_base();
    let store = SeriesStore::new(&vault);

    store
        .append_occurrence(cx_id, EpochSecs(100), ctx("one"))
        .expect("append one");

    let series = store.read_series(cx_id).expect("read series");
    assert_eq!(times(&series), vec![100]);
    assert_eq!(series.cadence_secs, None);
    assert_eq!(series.frequency, 1);
}

#[test]
fn max_occurrence_rollup_keeps_frequency_total() {
    let (vault, cx_id) = vault_with_base();
    let policy = RetentionPolicy::new(5, u64::MAX).expect("policy");
    let store = SeriesStore::with_retention(&vault, policy).expect("store");

    for index in 0..6 {
        store
            .append_occurrence(cx_id, EpochSecs(index), ctx("roll"))
            .expect("append occurrence");
    }

    let series = store.read_series(cx_id).expect("read series");
    assert_eq!(times(&series), vec![1, 2, 3, 4, 5]);
    assert_eq!(series.frequency, 6);
    let summary = series.rollup_summary.expect("rollup summary");
    assert_eq!(summary.oldest_t, EpochSecs(0));
    assert_eq!(summary.count_rolled, 1);
}

#[test]
fn max_occurrence_rollup_uses_oldest_ten_percent() {
    let (vault, cx_id) = vault_with_base();
    let policy = RetentionPolicy::new(10, u64::MAX).expect("policy");
    let store = SeriesStore::with_retention(&vault, policy).expect("store");

    for index in 0..11 {
        store
            .append_occurrence(cx_id, EpochSecs(index), ctx("roll10"))
            .expect("append occurrence");
    }

    let series = store.read_series(cx_id).expect("read series");
    assert_eq!(times(&series), vec![2, 3, 4, 5, 6, 7, 8, 9, 10]);
    assert_eq!(series.frequency, 11);
    assert_eq!(series.rollup_summary.unwrap().count_rolled, 2);
}

#[test]
fn age_rollup_uses_observed_time() {
    let (vault, cx_id) = vault_with_base();
    let policy = RetentionPolicy::new(10, 3_600).expect("policy");
    let store = SeriesStore::with_retention(&vault, policy).expect("store");

    store
        .append_occurrence_observed_at(cx_id, EpochSecs(0), ctx("old"), EpochSecs(0))
        .expect("append old");
    store
        .append_occurrence_observed_at(cx_id, EpochSecs(7_200), ctx("new"), EpochSecs(7_200))
        .expect("append new");

    let series = store.read_series(cx_id).expect("read series");
    assert_eq!(times(&series), vec![7_200]);
    assert_eq!(series.frequency, 2);
    assert_eq!(series.rollup_summary.unwrap().count_rolled, 1);
}

#[test]
fn empty_series_reads_zero_without_occurrences() {
    let (vault, cx_id) = vault_with_base();
    let store = SeriesStore::new(&vault);

    let series = store.read_series(cx_id).expect("read empty");

    assert_eq!(series.cx_id, cx_id);
    assert!(series.occurrences.is_empty());
    assert_eq!(series.frequency, 0);
    assert_eq!(series.cadence_secs, None);
}

#[test]
fn oversized_context_fails_closed_before_commit() {
    let (vault, cx_id) = vault_with_base();
    let store = SeriesStore::new(&vault);
    let error = OccurrenceContext::new(vec![7; MAX_CONTEXT_BYTES + 1])
        .and_then(|context| store.append_occurrence(cx_id, EpochSecs(1), context))
        .expect_err("context too large");

    assert_eq!(error.code, CALYX_RECURRENCE_CONTEXT_TOO_LARGE);
    assert_eq!(store.occurrence_count(cx_id).expect("count"), 0);
    assert!(
        store
            .read_series(cx_id)
            .expect("series")
            .occurrences
            .is_empty()
    );
}

proptest! {
    #[test]
    fn frequency_never_undercounts_appends(count in 1usize..=20) {
        let (vault, cx_id) = vault_with_base();
        let policy = RetentionPolicy::new(5, u64::MAX).expect("policy");
        let store = SeriesStore::with_retention(&vault, policy).expect("store");

        for index in 0..count {
            store
                .append_occurrence(cx_id, EpochSecs(index as i64), ctx("prop"))
                .expect("append property occurrence");
        }

        let series = store.read_series(cx_id).expect("read property series");
        prop_assert_eq!(series.frequency, count as u64);
        prop_assert!(series.occurrences.len() <= 5);
        let rolled = series
            .rollup_summary
            .as_ref()
            .map_or(0, |summary| summary.count_rolled);
        prop_assert_eq!(rolled + series.occurrences.len() as u64, count as u64);
    }
}

fn times(series: &RecurrenceSeries) -> Vec<i64> {
    series
        .occurrences
        .iter()
        .map(|occurrence| occurrence.t_k.0)
        .collect()
}

fn ctx(value: &str) -> OccurrenceContext {
    OccurrenceContext::new(value.as_bytes().to_vec()).expect("context")
}

fn vault_with_base() -> (AsterVault<FixedClock>, calyx_core::CxId) {
    let vault = AsterVault::with_clock(
        vault_id(),
        b"recurrence-test-salt".to_vec(),
        FixedClock::new(1),
    );
    let cx_id = vault.cx_id_for_input(b"recurrence-base", 41);
    let cx = Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 41,
        created_at: 100,
        input_ref: InputRef {
            hash: *blake3::hash(b"recurrence-base").as_bytes(),
            pointer: None,
            redacted: true,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            redacted_input: true,
            ..CxFlags::default()
        },
    };
    vault.put(cx).expect("put base");
    (vault, cx_id)
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}
