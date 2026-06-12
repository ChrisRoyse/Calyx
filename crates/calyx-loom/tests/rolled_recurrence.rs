use std::collections::BTreeMap;

use calyx_aster::dedup::EpochSecs;
use calyx_aster::vault::AsterVault;
use calyx_core::{
    Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use calyx_loom::{OccurrenceContext, PeriodicRecallQuery, RetentionPolicy, SeriesStore};

const TUESDAY_2024_01_02_14H_UTC: i64 = 1_704_204_000;
const WEEK_SECS: i64 = 604_800;

#[test]
fn rolled_summary_feeds_recall_support_when_active_phase_exists() {
    let (vault, cx_id) = rolled_weekly_vault(100, 4 * WEEK_SECS as u64, 11);
    let store = SeriesStore::new(&vault);

    let read = store.recurrence_series(cx_id).expect("rolled read");

    assert_eq!(read.series.frequency, 12);
    assert_eq!(read.series.occurrences.len(), 5);
    let summary = read.series.rollup_summary.as_ref().expect("rollup");
    assert_eq!(summary.count_rolled, 7);
    assert_eq!(summary.period_estimate_secs, WEEK_SECS as f64);
    assert_eq!(read.periodic_fit.support, 12);
    assert_eq!(read.periodic_fit.active_support, 5);
    assert_eq!(read.periodic_fit.rolled_support, 7);
    assert_eq!(
        read.periodic_fit.rollup_period_estimate_secs,
        Some(WEEK_SECS as f64)
    );
    assert_eq!(read.periodic_fit.target_hour, Some(14));
    assert_eq!(read.periodic_fit.target_day_of_week, Some(1));

    let recall = store
        .periodic_recall_readback(PeriodicRecallQuery::new(Some(14), Some(1)).expect("query"))
        .expect("recall");

    assert_eq!(recall.hits.len(), 1);
    assert_eq!(recall.hits[0].frequency, 12);
    assert_eq!(recall.hits[0].occurrence_count, 5);
    assert_eq!(recall.hits[0].periodic_fit.support, 12);
    assert_eq!(recall.hits[0].periodic_fit.active_support, 5);
    assert_eq!(recall.hits[0].periodic_fit.rolled_support, 7);
}

#[test]
fn rolled_support_without_active_phase_does_not_match_recall() {
    let (vault, cx_id) = rolled_weekly_vault(1, u64::MAX, 3);
    let store = SeriesStore::new(&vault);

    let read = store.recurrence_series(cx_id).expect("rolled read");

    assert_eq!(read.series.frequency, 4);
    assert_eq!(read.series.occurrences.len(), 1);
    assert_eq!(read.series.rollup_summary.as_ref().unwrap().count_rolled, 3);
    assert_eq!(read.periodic_fit.support, 4);
    assert_eq!(read.periodic_fit.active_support, 1);
    assert_eq!(read.periodic_fit.rolled_support, 3);
    assert_eq!(read.periodic_fit.target_hour, None);
    assert_eq!(read.periodic_fit.target_day_of_week, None);

    let recall = store
        .periodic_recall_readback(PeriodicRecallQuery::new(Some(14), Some(1)).expect("query"))
        .expect("recall");

    assert!(recall.hits.is_empty());
    assert_eq!(recall.stats.candidate_series_count, 1);
}

fn rolled_weekly_vault(
    max_occurrences: usize,
    max_age_secs: u64,
    final_week: i64,
) -> (AsterVault<FixedClock>, CxId) {
    let vault = AsterVault::with_clock(
        vault_id(),
        b"rolled-recurrence-test-salt".to_vec(),
        FixedClock::new(1),
    );
    let cx_id = put_base(&vault, b"rolled-weekly");
    let seed_store = SeriesStore::new(&vault);
    for week in 0..final_week {
        seed_store
            .append_occurrence(cx_id, EpochSecs(weekly_time(week)), ctx("seed"))
            .expect("seed append");
    }
    let retention = RetentionPolicy::new(max_occurrences, max_age_secs).expect("retention");
    let rolling_store = SeriesStore::with_retention(&vault, retention).expect("rolling store");
    rolling_store
        .append_occurrence_observed_at(
            cx_id,
            EpochSecs(weekly_time(final_week)),
            ctx("roll"),
            EpochSecs(weekly_time(final_week)),
        )
        .expect("rolling append");
    (vault, cx_id)
}

fn weekly_time(week: i64) -> i64 {
    TUESDAY_2024_01_02_14H_UTC + week * WEEK_SECS
}

fn ctx(value: &str) -> OccurrenceContext {
    OccurrenceContext::new(value.as_bytes().to_vec()).expect("context")
}

fn put_base(vault: &AsterVault<FixedClock>, input: &[u8]) -> CxId {
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
