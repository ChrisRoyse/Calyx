use std::collections::BTreeMap;
use std::sync::{Arc, Barrier};
use std::thread;

use calyx_aster::dedup::{EpochSecs, OccurrenceId};
use calyx_aster::vault::AsterVault;
use calyx_core::{
    Constellation, CxFlags, FixedClock, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use proptest::prelude::*;

use crate::error::CALYX_RECURRENCE_CONTEXT_TOO_LARGE;

use super::*;

const TUESDAY_2024_01_02_14H_UTC: i64 = 1_704_204_000;
const WEEK_SECS: i64 = 604_800;

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
fn concurrent_series_store_appends_allocate_unique_contiguous_ids() {
    let (vault, cx_id) = vault_with_base();
    let vault = Arc::new(vault);
    let workers = 16;
    let barrier = Arc::new(Barrier::new(workers));
    let handles = (0..workers)
        .map(|index| {
            let vault = Arc::clone(&vault);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                let store = SeriesStore::new(vault.as_ref());
                store.append_occurrence(
                    cx_id,
                    EpochSecs(1_000 + index as i64),
                    ctx(&format!("race-{index}")),
                )
            })
        })
        .collect::<Vec<_>>();

    let mut ids = Vec::new();
    for handle in handles {
        ids.push(handle.join().expect("thread").expect("append"));
    }
    ids.sort();
    assert_eq!(
        ids,
        (0..workers as u64).map(OccurrenceId).collect::<Vec<_>>()
    );

    let store = SeriesStore::new(vault.as_ref());
    let series = store.read_series(cx_id).expect("series");
    assert_eq!(series.frequency, workers as u64);
    assert_eq!(series.occurrences.len(), workers);
    let mut stored_ids = series
        .occurrences
        .iter()
        .map(|occurrence| occurrence.id)
        .collect::<Vec<_>>();
    stored_ids.sort();
    assert_eq!(stored_ids, ids);
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
fn single_occurrence_has_no_periodic_target() {
    let (vault, cx_id) = vault_with_base();
    let store = SeriesStore::new(&vault);

    store
        .append_occurrence(cx_id, EpochSecs(TUESDAY_2024_01_02_14H_UTC), ctx("one"))
        .expect("append one");

    let read = store.recurrence_series(cx_id).expect("public read");

    assert_eq!(read.periodic_fit.support, 1);
    assert_eq!(read.periodic_fit.target_hour, None);
    assert_eq!(read.periodic_fit.target_day_of_week, None);
    assert_eq!(read.periodic_fit.target_hour_day, None);
    assert_eq!(read.periodic_fit.dominant_period_secs, None);
    assert_eq!(read.periodic_fit.hour_confidence, 0.0);
    assert_eq!(read.periodic_fit.day_confidence, 0.0);
    assert_eq!(read.periodic_fit.hour_day_confidence, 0.0);
}

#[test]
fn public_recurrence_series_adds_periodic_fit() {
    let (vault, cx_id) = vault_with_base();
    let store = SeriesStore::new(&vault);

    for week in 0..6 {
        store
            .append_occurrence(
                cx_id,
                EpochSecs(TUESDAY_2024_01_02_14H_UTC + week * WEEK_SECS),
                ctx("weekly"),
            )
            .expect("append weekly occurrence");
    }

    let read = store.recurrence_series(cx_id).expect("public read");

    assert_eq!(read.series.frequency, 6);
    assert_eq!(read.periodic_fit.target_hour, Some(14));
    assert_eq!(read.periodic_fit.target_day_of_week, Some(1));
    assert_eq!(
        read.periodic_fit.target_hour_day,
        Some(PeriodicTimeBucket {
            target_hour: 14,
            target_day_of_week: 1
        })
    );
    assert_eq!(
        read.periodic_fit.dominant_period_secs,
        Some(WEEK_SECS as f64)
    );
    assert_eq!(read.periodic_fit.support, 6);
    assert_eq!(read.periodic_fit.hour_confidence, 1.0);
    assert_eq!(read.periodic_fit.day_confidence, 1.0);
    assert_eq!(read.periodic_fit.hour_day_confidence, 1.0);
}

#[test]
fn tied_periodic_buckets_do_not_claim_target() {
    let (vault, cx_id) = vault_with_base();
    let store = SeriesStore::new(&vault);

    store
        .append_occurrence(cx_id, EpochSecs(TUESDAY_2024_01_02_14H_UTC), ctx("tue"))
        .expect("append tuesday");
    store
        .append_occurrence(
            cx_id,
            EpochSecs(TUESDAY_2024_01_02_14H_UTC + 19 * 3_600),
            ctx("wed"),
        )
        .expect("append wednesday");

    let read = store.recurrence_series(cx_id).expect("public read");
    let recall = store
        .periodic_recall(PeriodicRecallQuery::new(Some(14), Some(1)).expect("query"))
        .expect("periodic recall");

    assert_eq!(read.periodic_fit.support, 2);
    assert_eq!(read.periodic_fit.target_hour, None);
    assert_eq!(read.periodic_fit.target_day_of_week, None);
    assert_eq!(read.periodic_fit.target_hour_day, None);
    assert_eq!(read.periodic_fit.hour_confidence, 0.5);
    assert_eq!(read.periodic_fit.day_confidence, 0.5);
    assert_eq!(read.periodic_fit.hour_day_confidence, 0.5);
    assert!(recall.is_empty());
}

#[test]
fn public_periodic_fit_sorts_unsorted_input_before_period_estimate() {
    let occurrences = vec![
        occurrence(2, 300, "c"),
        occurrence(0, 100, "a"),
        occurrence(1, 200, "b"),
    ];

    let fit = periodic_fit(&occurrences);

    assert_eq!(fit.support, 3);
    assert_eq!(fit.dominant_period_secs, Some(100.0));
}

#[test]
fn joint_recall_requires_observed_hour_day_bucket() {
    let (vault, cx_id) = vault_with_base();
    let store = SeriesStore::new(&vault);
    for (offset_secs, context) in [
        (3_600, "tue-15"),
        (24 * 3_600, "wed-14"),
        (2 * 24 * 3_600, "thu-14"),
        (2 * 3_600, "tue-16"),
    ] {
        store
            .append_occurrence(
                cx_id,
                EpochSecs(TUESDAY_2024_01_02_14H_UTC + offset_secs),
                ctx(context),
            )
            .expect("append mixed occurrence");
    }

    let read = store.recurrence_series(cx_id).expect("public read");
    let joint_hits = store
        .periodic_recall(PeriodicRecallQuery::new(Some(14), Some(1)).expect("joint query"))
        .expect("joint recall");
    let hour_hits = store
        .periodic_recall(PeriodicRecallQuery::new(Some(14), None).expect("hour query"))
        .expect("hour recall");
    let day_hits = store
        .periodic_recall(PeriodicRecallQuery::new(None, Some(1)).expect("day query"))
        .expect("day recall");

    assert_eq!(read.periodic_fit.target_hour, Some(14));
    assert_eq!(read.periodic_fit.target_day_of_week, Some(1));
    assert_eq!(read.periodic_fit.target_hour_day, None);
    assert!(joint_hits.is_empty());
    assert_eq!(hour_hits.len(), 1);
    assert_eq!(day_hits.len(), 1);
}

#[test]
fn periodic_recall_returns_only_matching_series() {
    let vault = AsterVault::with_clock(
        vault_id(),
        b"recurrence-test-salt".to_vec(),
        FixedClock::new(1),
    );
    let tuesday = put_base(&vault, b"recurrence-tuesday");
    let wednesday = put_base(&vault, b"recurrence-wednesday");
    let store = SeriesStore::new(&vault);

    for week in 0..3 {
        store
            .append_occurrence(
                tuesday,
                EpochSecs(TUESDAY_2024_01_02_14H_UTC + week * WEEK_SECS),
                ctx("tue"),
            )
            .expect("append tuesday");
        store
            .append_occurrence(
                wednesday,
                EpochSecs(TUESDAY_2024_01_02_14H_UTC + 19 * 3_600 + week * WEEK_SECS),
                ctx("wed"),
            )
            .expect("append wednesday");
    }

    let query = PeriodicRecallQuery::new(Some(14), Some(1)).expect("query");
    let hits = store.periodic_recall(query).expect("periodic recall");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].cx_id, tuesday);
    assert_eq!(hits[0].frequency, 3);
    assert_eq!(hits[0].periodic_fit.target_hour, Some(14));
    assert_eq!(hits[0].periodic_fit.target_day_of_week, Some(1));
    assert_eq!(
        hits[0].periodic_fit.target_hour_day,
        Some(PeriodicTimeBucket {
            target_hour: 14,
            target_day_of_week: 1
        })
    );
}

#[test]
fn periodic_recall_rejects_invalid_query() {
    let error = PeriodicRecallQuery::new(Some(24), None).expect_err("invalid hour");

    assert_eq!(error.code, calyx_core::CALYX_TEMPORAL_INVALID_PERIOD);
}

#[test]
fn periodic_recall_rejects_empty_query() {
    let error = PeriodicRecallQuery::new(None, None).expect_err("empty query");

    assert_eq!(error.code, calyx_core::CALYX_TEMPORAL_INVALID_PERIOD);
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

fn occurrence(id: u64, time_secs: i64, context: &str) -> Occurrence {
    Occurrence {
        id: OccurrenceId(id),
        t_k: EpochSecs(time_secs),
        context: ctx(context),
    }
}

fn vault_with_base() -> (AsterVault<FixedClock>, calyx_core::CxId) {
    let vault = AsterVault::with_clock(
        vault_id(),
        b"recurrence-test-salt".to_vec(),
        FixedClock::new(1),
    );
    let cx_id = put_base(&vault, b"recurrence-base");
    (vault, cx_id)
}

fn put_base(vault: &AsterVault<FixedClock>, input: &[u8]) -> calyx_core::CxId {
    let cx_id = vault.cx_id_for_input(input, 41);
    let cx = Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 41,
        created_at: 100,
        input_ref: InputRef {
            hash: *blake3::hash(input).as_bytes(),
            pointer: None,
            redacted: true,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
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
    cx_id
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}
