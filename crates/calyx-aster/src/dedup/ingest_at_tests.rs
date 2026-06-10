use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxId, FixedClock, Modality, SlotId, SlotVector, VaultId,
};

use super::*;
use crate::cf::{ColumnFamily, base_key, recurrence_key};
use crate::dedup::{
    CALYX_RECURRENCE_SLOT_MISSING, TauStrategy, TctCosineConfig, decode_dedup_online_event,
    dedup_online_key,
};
use crate::recurrence::{StoredRecurrenceRow, decode_recurrence_row, read_series};
use proptest::prelude::*;
use std::sync::{Arc, Barrier};
use std::thread;

#[test]
fn off_policy_stores_each_distinct_input_as_new() {
    let vault = vault(DedupPolicy::Off);
    let first = input("off-a", [1.0, 0.0]);
    let second = input("off-b", [1.0, 0.0]);

    let first_result = ingest_at(&vault, &first, EpochSecs(100), None).expect("first ingest");
    let second_result = ingest_at(&vault, &second, EpochSecs(200), None).expect("second ingest");

    assert!(matches!(first_result, DedupResult::New(_)));
    assert!(matches!(second_result, DedupResult::New(_)));
    assert_eq!(scan(&vault, ColumnFamily::Base).len(), 2);
    assert_eq!(scan(&vault, ColumnFamily::Ledger).len(), 2);
}

#[test]
fn exact_policy_writes_ledger_for_exact_duplicate() {
    let vault = vault(DedupPolicy::Exact);
    let input = input("exact-same", [1.0, 0.0]);
    let first = ingest_at(&vault, &input, EpochSecs(100), None).expect("first ingest");
    let second = ingest_at(&vault, &input, EpochSecs(100), None).expect("second ingest");

    let DedupResult::New(id) = first else {
        panic!("expected first new");
    };
    assert_eq!(second, DedupResult::ExactDuplicate(id));
    assert_eq!(scan(&vault, ColumnFamily::Base).len(), 1);
    assert_eq!(scan(&vault, ColumnFamily::Ledger).len(), 2);
}

#[test]
fn recurrence_series_same_content_appends_online_occurrences() {
    let vault = vault(tct_policy(DedupAction::RecurrenceSeries));

    let first = ingest_at(
        &vault,
        &temporal_input("recurring", [1.0, 0.0], [1.0, 0.0]),
        EpochSecs(100),
        None,
    )
    .expect("first");
    let second = ingest_at(
        &vault,
        &temporal_input("recurring", [1.0, 0.0], [0.0, 1.0]),
        EpochSecs(200),
        None,
    )
    .expect("second");
    let third = ingest_at(
        &vault,
        &temporal_input("recurring", [1.0, 0.0], [-1.0, 0.0]),
        EpochSecs(300),
        None,
    )
    .expect("third");

    let DedupResult::New(id) = first else {
        panic!("expected first new");
    };
    assert_eq!(
        second,
        DedupResult::DedupMerge {
            into: id,
            occurrence: OccurrenceId(1)
        }
    );
    assert_eq!(
        third,
        DedupResult::DedupMerge {
            into: id,
            occurrence: OccurrenceId(2)
        }
    );
    assert_eq!(scan(&vault, ColumnFamily::Base).len(), 1);
    assert_eq!(scan(&vault, ColumnFamily::Ledger).len(), 3);
    let times = (0..=2)
        .map(|occ| occurrence_at(&vault, id, occ))
        .collect::<Vec<_>>();
    assert_eq!(times, vec![100, 200, 300]);
    let recurrence_times = (0..=2)
        .map(|occ| recurrence_at(&vault, id, occ))
        .collect::<Vec<_>>();
    assert_eq!(recurrence_times, vec![100, 200, 300]);
    assert_eq!(scan(&vault, ColumnFamily::Recurrence).len(), 3);
}

#[test]
fn concurrent_recurrence_ingest_allocates_unique_contiguous_occurrences() {
    let vault = Arc::new(vault(tct_policy(DedupAction::RecurrenceSeries)));
    let first = ingest_at(
        vault.as_ref(),
        &input("ingest-race", [1.0, 0.0]),
        EpochSecs(100),
        None,
    )
    .expect("first");
    let DedupResult::New(id) = first else {
        panic!("expected first new");
    };

    let workers = 12;
    let barrier = Arc::new(Barrier::new(workers));
    let handles = (0..workers)
        .map(|index| {
            let vault = Arc::clone(&vault);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                ingest_at(
                    vault.as_ref(),
                    &input("ingest-race", [1.0, 0.0]),
                    EpochSecs(200 + index as i64),
                    None,
                )
            })
        })
        .collect::<Vec<_>>();

    let mut merge_ids = Vec::new();
    for handle in handles {
        let result = handle.join().expect("thread").expect("ingest");
        let DedupResult::DedupMerge { into, occurrence } = result else {
            panic!("expected recurrence merge");
        };
        assert_eq!(into, id);
        merge_ids.push(occurrence);
    }
    merge_ids.sort();
    assert_eq!(
        merge_ids,
        (1..=workers as u64).map(OccurrenceId).collect::<Vec<_>>()
    );

    let series = read_series(vault.as_ref(), id).expect("series");
    assert_eq!(series.frequency, workers as u64 + 1);
    let mut stored_ids = series
        .occurrences
        .iter()
        .map(|occurrence| occurrence.id)
        .collect::<Vec<_>>();
    stored_ids.sort();
    assert_eq!(
        stored_ids,
        (0..=workers as u64).map(OccurrenceId).collect::<Vec<_>>()
    );
    let mut times = series
        .occurrences
        .iter()
        .map(|occurrence| occurrence.t_k.0)
        .collect::<Vec<_>>();
    times.sort();
    assert_eq!(
        times,
        std::iter::once(100)
            .chain((0..workers).map(|index| 200 + index as i64))
            .collect::<Vec<_>>()
    );
}

#[test]
fn recurrence_series_same_temporal_slot_does_not_append_occurrence() {
    let vault = vault(tct_policy(DedupAction::RecurrenceSeries));
    let first_input = temporal_input("same-time", [1.0, 0.0], [1.0, 0.0]);
    let second_input = temporal_input("same-time", [1.0, 0.0], [1.0, 0.0]);

    let first = ingest_at(&vault, &first_input, EpochSecs(100), None).expect("first");
    let second = ingest_at(&vault, &second_input, EpochSecs(200), None).expect("second");

    let DedupResult::New(id) = first else {
        panic!("expected first new");
    };
    assert_eq!(second, DedupResult::ExactDuplicate(id));
    assert_eq!(scan(&vault, ColumnFamily::Base).len(), 1);
    assert_eq!(scan(&vault, ColumnFamily::Recurrence).len(), 1);
    assert_eq!(recurrence_at(&vault, id, 0), 100);
}

#[test]
fn recurrence_series_uses_event_time_when_temporal_slot_list_absent() {
    let vault = vault(tct_policy(DedupAction::RecurrenceSeries));
    let input = input("fallback-time", [1.0, 0.0]);

    let first = ingest_at(&vault, &input, EpochSecs(100), None).expect("first");
    let second = ingest_at(&vault, &input, EpochSecs(200), None).expect("second");

    let DedupResult::New(id) = first else {
        panic!("expected first new");
    };
    assert_eq!(
        second,
        DedupResult::DedupMerge {
            into: id,
            occurrence: OccurrenceId(1)
        }
    );
    assert_eq!(scan(&vault, ColumnFamily::Base).len(), 1);
    assert_eq!(scan(&vault, ColumnFamily::Ledger).len(), 2);
    assert_eq!(scan(&vault, ColumnFamily::Recurrence).len(), 2);
    assert_eq!(recurrence_at(&vault, id, 0), 100);
    assert_eq!(recurrence_at(&vault, id, 1), 200);
}

#[test]
fn recurrence_series_missing_temporal_slot_fails_closed() {
    let vault = vault(tct_policy(DedupAction::RecurrenceSeries));
    let first = temporal_input("missing-temporal", [1.0, 0.0], [1.0, 0.0]);
    let missing_temporal = input("missing-temporal", [1.0, 0.0]).with_temporal_slot(temp_slot());

    let first_result = ingest_at(&vault, &first, EpochSecs(100), None).expect("first");
    let error = ingest_at(&vault, &missing_temporal, EpochSecs(200), None)
        .expect_err("missing temporal slot rejected");

    let DedupResult::New(id) = first_result else {
        panic!("expected first new");
    };
    assert_eq!(error.code, CALYX_RECURRENCE_SLOT_MISSING);
    assert_eq!(scan(&vault, ColumnFamily::Ledger).len(), 1);
    assert_eq!(scan(&vault, ColumnFamily::Recurrence).len(), 1);
    assert_eq!(recurrence_at(&vault, id, 0), 100);
}

#[test]
fn ingest_input_json_missing_temporal_slots_defaults_empty() {
    let json = r#"{
        "raw_bytes":[108,101,103,97,99,121],
        "panel_version":41,
        "modality":"text",
        "slots":{},
        "scalars":{},
        "anchors":[],
        "input_pointer":null,
        "redacted":true
    }"#;

    let input: IngestInput = serde_json::from_str(json).expect("legacy input json");

    assert!(input.temporal_slot_ids().is_empty());
}

#[test]
fn collapse_match_does_not_store_candidate_base() {
    let vault = vault(tct_policy(DedupAction::Collapse));
    let first = input("collapse-a", [1.0, 0.0]);
    let second = input("collapse-b", [1.0, 0.0]);

    let first_result = ingest_at(&vault, &first, EpochSecs(100), None).expect("first");
    let second_result = ingest_at(&vault, &second, EpochSecs(200), None).expect("second");

    let DedupResult::New(existing) = first_result else {
        panic!("expected first new");
    };
    let second_id = vault.cx_id_for_input(b"collapse-b", 41);
    assert_eq!(
        second_result,
        DedupResult::DedupMerge {
            into: existing,
            occurrence: OccurrenceId(0)
        }
    );
    assert!(base_present(&vault, existing));
    assert!(!base_present(&vault, second_id));
    assert_eq!(scan(&vault, ColumnFamily::Ledger).len(), 2);
}

#[test]
fn anchor_conflict_stores_candidate_and_contested_rows_together() {
    let vault = vault(tct_policy(DedupAction::RecurrenceSeries));
    let first = input("speaker-a", [1.0, 0.0]).with_anchor(speaker("alice"));
    let second = input("speaker-b", [1.0, 0.0]).with_anchor(speaker("bob"));

    let first_result = ingest_at(&vault, &first, EpochSecs(100), None).expect("first");
    let second_result = ingest_at(&vault, &second, EpochSecs(200), None).expect("second");

    let DedupResult::New(first_id) = first_result else {
        panic!("expected first new");
    };
    let DedupResult::New(second_id) = second_result else {
        panic!("expected conflict as new");
    };
    assert!(base_present(&vault, first_id));
    assert!(base_present(&vault, second_id));
    assert!(contested_present(&vault, first_id));
    assert!(contested_present(&vault, second_id));
    assert_eq!(scan(&vault, ColumnFamily::Ledger).len(), 2);
}

#[test]
fn negative_event_time_fails_closed_without_rows() {
    let vault = vault(DedupPolicy::Off);
    let error = ingest_at(&vault, &input("negative", [1.0, 0.0]), EpochSecs(-1), None)
        .expect_err("negative event time rejected");

    assert_eq!(error.code, CALYX_DEDUP_INVALID_EVENT_TIME);
    assert_eq!(scan(&vault, ColumnFamily::Base).len(), 0);
    assert_eq!(scan(&vault, ColumnFamily::Ledger).len(), 0);
}

proptest! {
    #[test]
    fn recurrence_series_repeats_are_one_new_then_merges(count in 1usize..=8) {
        let vault = vault(tct_policy(DedupAction::RecurrenceSeries));
        let mut new_count = 0;
        let mut merge_count = 0;
        let mut series_id = None;

        for index in 0..count {
            let input = temporal_input(
                "recurrence-prop",
                [1.0, 0.0],
                cos_vector_for_index(index),
            );
            let result = ingest_at(&vault, &input, EpochSecs(100 + index as i64), None)
                .expect("ingest recurrence property row");
            match result {
                DedupResult::New(id) => {
                    new_count += 1;
                    series_id = Some(id);
                }
                DedupResult::DedupMerge { into, occurrence } => {
                    merge_count += 1;
                    prop_assert_eq!(Some(into), series_id);
                    prop_assert_eq!(occurrence, OccurrenceId(index as u64));
                }
                DedupResult::ExactDuplicate(_) => {
                    prop_assert!(false, "same content at new event time must merge");
                }
            }
        }

        let id = series_id.expect("first ingest creates series");
        prop_assert_eq!(new_count, 1);
        prop_assert_eq!(merge_count, count.saturating_sub(1));
        prop_assert_eq!(scan(&vault, ColumnFamily::Base).len(), 1);
        for occurrence in 0..count as u64 {
            prop_assert_eq!(
                occurrence_at(&vault, id, occurrence),
                100 + occurrence as i64
            );
            prop_assert_eq!(
                recurrence_at(&vault, id, occurrence),
                100 + occurrence as i64
            );
        }
    }
}

fn recurrence_at(vault: &AsterVault<FixedClock>, id: CxId, occ: u64) -> i64 {
    let bytes = vault
        .read_cf_at(
            vault.snapshot(),
            ColumnFamily::Recurrence,
            &recurrence_key(id, occ),
        )
        .expect("read recurrence")
        .expect("recurrence row");
    match decode_recurrence_row(&bytes).expect("decode recurrence") {
        StoredRecurrenceRow::Occurrence(occurrence) => occurrence.t_k.0,
        StoredRecurrenceRow::RollupSummary(_)
        | StoredRecurrenceRow::RolledOccurrence { .. }
        | StoredRecurrenceRow::Tombstone { .. } => panic!("expected occurrence row"),
    }
}

fn occurrence_at(vault: &AsterVault<FixedClock>, id: CxId, occ: u64) -> i64 {
    let key = dedup_online_key(DedupOnlineKind::Occurrence, id, OccurrenceId(occ));
    let bytes = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Online, &key)
        .expect("read online")
        .expect("occurrence row");
    decode_dedup_online_event(&bytes)
        .expect("decode event")
        .at
        .0
}

fn contested_present(vault: &AsterVault<FixedClock>, id: CxId) -> bool {
    vault
        .read_cf_at(
            vault.snapshot(),
            ColumnFamily::Online,
            &contested_with_key(id),
        )
        .expect("read contested")
        .is_some()
}

fn base_present(vault: &AsterVault<FixedClock>, id: CxId) -> bool {
    vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Base, &base_key(id))
        .expect("read base")
        .is_some()
}

fn scan(vault: &AsterVault<FixedClock>, cf: ColumnFamily) -> Vec<(Vec<u8>, Vec<u8>)> {
    vault.scan_cf_at(vault.snapshot(), cf).expect("scan cf")
}

fn input(name: &str, dense_values: [f32; 2]) -> IngestInput {
    IngestInput::new(name.as_bytes().to_vec(), 41, Modality::Text).with_slot(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: dense_values.to_vec(),
        },
    )
}

fn temporal_input(name: &str, dense_values: [f32; 2], temporal_values: [f32; 2]) -> IngestInput {
    input(name, dense_values)
        .with_slot(
            temp_slot(),
            SlotVector::Dense {
                dim: 2,
                data: temporal_values.to_vec(),
            },
        )
        .with_temporal_slot(temp_slot())
}

fn cos_vector_for_index(index: usize) -> [f32; 2] {
    match index {
        0 => [1.0, 0.0],
        1 => [0.0, 1.0],
        2 => [-1.0, 0.0],
        3 => [0.0, -1.0],
        4 => [0.70710677, 0.70710677],
        5 => [-0.70710677, 0.70710677],
        6 => [-0.70710677, -0.70710677],
        _ => [0.70710677, -0.70710677],
    }
}

fn temp_slot() -> SlotId {
    SlotId::new(20)
}

fn speaker(name: &str) -> Anchor {
    Anchor {
        kind: AnchorKind::SpeakerMatch,
        value: AnchorValue::Text(name.to_string()),
        source: "synthetic-ingest-at".to_string(),
        observed_at: 100,
        confidence: 1.0,
    }
}

fn vault(policy: DedupPolicy) -> AsterVault<FixedClock> {
    AsterVault::with_clock_and_dedup_policy(
        vault_id(),
        b"ingest-at-test-salt".to_vec(),
        FixedClock::new(9_000_000),
        policy,
    )
    .expect("vault")
}

fn tct_policy(action: DedupAction) -> DedupPolicy {
    DedupPolicy::TctCosine(
        TctCosineConfig::new(
            vec![SlotId::new(0)],
            TauStrategy::PerSlot(vec![(SlotId::new(0), 0.90)]),
            action,
        )
        .expect("policy"),
    )
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}
