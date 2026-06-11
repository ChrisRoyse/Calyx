use std::collections::BTreeMap;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::dedup::{EpochSecs, OccurrenceId};
use calyx_aster::recurrence::FREQUENCY_SCALAR;
use calyx_aster::vault::AsterVault;
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use calyx_loom::{
    CALYX_LOOM_SERIES_READ_ERROR, LeadLagResult, Occurrence, OccurrenceContext, RecurrenceSeries,
    SeriesStore, co_occurrence_pairs, decode_lead_lag_result, encode_lead_lag_result,
    lead_lag_secs, temporal_cross_term,
};
use proptest::prelude::*;

#[test]
fn lead_lag_positive_when_a_leads_b() {
    let a = series(cx(1), &[100, 200, 300]);
    let b = series(cx(2), &[110, 210, 310]);

    let pairs = co_occurrence_pairs(&a, &b, 30);
    let result = lead_lag_secs(&a, &b, 30).expect("lead lag");

    assert_eq!(
        pairs,
        vec![
            (EpochSecs(100), EpochSecs(110)),
            (EpochSecs(200), EpochSecs(210)),
            (EpochSecs(300), EpochSecs(310))
        ]
    );
    assert_eq!(result, lead_lag(cx(1), cx(2), 10.0, 3, 30));
}

#[test]
fn lead_lag_negative_when_b_leads_a() {
    let a = series(cx(1), &[100, 200, 300]);
    let b = series(cx(2), &[90, 190, 290]);

    let result = lead_lag_secs(&a, &b, 30).expect("lead lag");

    assert_eq!(result.lead_lag_secs, -10.0);
    assert_eq!(result.n_pairs, 3);
}

#[test]
fn no_nearby_or_insufficient_pairs_returns_none() {
    let a = series(cx(1), &[100, 200, 300, 400, 500]);
    let far = series(cx(2), &[1_000, 2_000]);
    let sparse = series(cx(3), &[102, 202]);

    assert!(lead_lag_secs(&a, &far, 5).is_none());
    assert!(lead_lag_secs(&a, &sparse, 5).is_none());
}

#[test]
fn window_zero_finds_no_pairs() {
    let a = series(cx(1), &[100, 200, 300]);
    let b = series(cx(2), &[100, 200, 300]);

    assert!(co_occurrence_pairs(&a, &b, 0).is_empty());
    assert!(lead_lag_secs(&a, &b, 0).is_none());
}

#[test]
fn temporal_cross_term_stores_and_decodes_byte_exact_row() {
    let vault = AsterVault::new(vault_id(), b"issue388-unit".to_vec());
    let a = cx(1);
    let b = cx(2);
    put_base(&vault, a, 0.0);
    put_base(&vault, b, 0.0);
    append_times(&vault, a, &[100, 200, 300, 400, 500]);
    append_times(&vault, b, &[115, 215, 315, 415, 515]);

    let result = temporal_cross_term(a, b, &vault, 30)
        .expect("temporal cross term")
        .expect("result");
    let bytes = vault
        .read_temporal_xterm(vault.latest_seq(), a, b)
        .expect("read row")
        .expect("row present");
    let decoded = decode_lead_lag_result(&bytes).expect("decode");

    assert_eq!(result, lead_lag(a, b, 15.0, 5, 30));
    assert_eq!(decoded, result);
    assert_eq!(bytes, encode_lead_lag_result(&result).expect("encode"));
    assert_eq!(&bytes[37..45], &15.0_f64.to_be_bytes());
}

#[test]
fn same_cxid_reports_zero_lag_without_storing() {
    let vault = AsterVault::new(vault_id(), b"issue388-self".to_vec());
    let id = cx(9);
    put_base(&vault, id, 0.0);
    append_times(&vault, id, &[100, 200, 300]);
    let before = row_count(&vault);

    let result = temporal_cross_term(id, id, &vault, 30)
        .expect("temporal cross term")
        .expect("self result");
    let stored = vault
        .read_temporal_xterm(vault.latest_seq(), id, id)
        .expect("read self row");

    assert_eq!(result, lead_lag(id, id, 0.0, 3, 30));
    assert_eq!(stored, None);
    assert_eq!(row_count(&vault), before);
}

#[test]
fn series_read_errors_fail_closed_with_loom_code() {
    let vault = AsterVault::new(vault_id(), b"issue388-error".to_vec());
    let bad = cx(4);
    let good = cx(5);
    put_base(&vault, bad, 1.5);
    put_base(&vault, good, 0.0);

    let error = temporal_cross_term(bad, good, &vault, 30).expect_err("series read error");

    assert_eq!(error.code, CALYX_LOOM_SERIES_READ_ERROR);
    assert!(error.message.contains("recurrence frequency scalar"));
}

proptest! {
    #[test]
    fn lead_lag_sign_flips_when_direction_reverses(
        start in 0_i64..10_000,
        spacing in 100_i64..1_000,
        offset in 1_i64..50,
        forward in any::<bool>(),
    ) {
        let signed_offset = if forward { offset } else { -offset };
        let a_times = (0..5).map(|index| start + (index * spacing)).collect::<Vec<_>>();
        let b_times = a_times.iter().map(|time| time + signed_offset).collect::<Vec<_>>();
        let a = series(cx(1), &a_times);
        let b = series(cx(2), &b_times);
        let window = offset as u64 + 1;

        let ab = lead_lag_secs(&a, &b, window).expect("ab lag");
        let ba = lead_lag_secs(&b, &a, window).expect("ba lag");

        prop_assert_eq!(ab.n_pairs, 5);
        prop_assert_eq!(ba.n_pairs, 5);
        prop_assert!((ab.lead_lag_secs + ba.lead_lag_secs).abs() < f64::EPSILON);
    }
}

fn series(cx_id: CxId, times: &[i64]) -> RecurrenceSeries {
    RecurrenceSeries {
        cx_id,
        occurrences: times
            .iter()
            .enumerate()
            .map(|(index, time)| Occurrence {
                id: OccurrenceId(index as u64),
                t_k: EpochSecs(*time),
                context: OccurrenceContext::new(format!("t={time}").into_bytes()).unwrap(),
            })
            .collect(),
        frequency: times.len() as u64,
        cadence_secs: None,
        rollup_summary: None,
    }
}

fn append_times(vault: &AsterVault, cx_id: CxId, times: &[i64]) {
    let store = SeriesStore::new(vault);
    for time in times {
        store
            .append_occurrence(
                cx_id,
                EpochSecs(*time),
                OccurrenceContext::new(format!("t={time}").into_bytes()).unwrap(),
            )
            .expect("append occurrence");
    }
}

fn put_base(vault: &AsterVault, cx_id: CxId, frequency: f64) {
    let mut cx = base_cx(cx_id);
    cx.scalars.insert(FREQUENCY_SCALAR.to_string(), frequency);
    vault.put(cx).expect("put base");
}

fn base_cx(cx_id: CxId) -> Constellation {
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 42,
        created_at: 1_786_406_600,
        input_ref: InputRef {
            hash: [cx_id.to_bytes()[0]; 32],
            pointer: None,
            redacted: false,
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
        flags: CxFlags::default(),
    }
}

fn row_count(vault: &AsterVault) -> usize {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::TemporalXTerm)
        .expect("scan temporal xterm")
        .len()
}

fn lead_lag(
    cx_a: CxId,
    cx_b: CxId,
    lead_lag_secs: f64,
    n_pairs: usize,
    window: u64,
) -> LeadLagResult {
    LeadLagResult {
        cx_a,
        cx_b,
        lead_lag_secs,
        n_pairs,
        proximity_window_secs: window,
    }
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse()
        .expect("valid vault id")
}
