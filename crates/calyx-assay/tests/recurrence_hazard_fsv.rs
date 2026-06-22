//! PH52 / issue #585 FSV: inter-event-time hazard ("overdue" anomaly) and
//! CUSUM recurrence-rate change-point.
//!
//! Synthetic known-I/O discipline (the `2+2=4` rule): every planted input has a
//! hand-computed expected output. The vault-backed tests write occurrences to
//! the **Aster Recurrence CF** (the on-disk source of truth), read them back
//! with `read_series`, and run the detectors on the read-back series — proving
//! the SoT path end-to-end, never trusting a return value alone. The
//! `#[ignore]` test writes the byte-readback evidence JSON consumed by the
//! manual FSV (`CALYX_FSV_ROOT`).

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use calyx_assay::{
    CusumConfig, RateShift, inter_event_hazard, inter_event_hazard_from_series,
    inter_event_hazard_with_alpha, recurrence_rate_cusum, recurrence_rate_cusum_from_series,
    recurrence_rate_cusum_with_config,
};
use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::{
    OccurrenceContext, RecurrenceSeries, RetentionPolicy, append_occurrence, read_series,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use serde_json::json;

// ── Hand-computed pure-math tests (no I/O) ──────────────────────────────────

#[test]
fn regular_series_that_stopped_is_overdue_at_known_time() {
    // 11 events spaced exactly 100s: gaps are ten 100s → μ=100, σ=0 (CV=0),
    // so the deterministic renewal branch applies: S(d)=1 for d<100 else 0.
    let times: Vec<f64> = (0..11).map(|k| 1000.0 + k as f64 * 100.0).collect();
    let last = 2000.0;

    // now = 2350 → elapsed 350 ≥ 100 → overdue; expected_next = 2100.
    let report = inter_event_hazard(&times, last + 350.0).unwrap();
    assert!(report.deterministic);
    assert_eq!(report.mean_gap, 100.0);
    assert_eq!(report.gap_variance, 0.0);
    assert_eq!(report.survival, 0.0);
    assert_eq!(report.empirical_survival, 0.0);
    assert_eq!(report.expected_next, 2100.0);
    assert_eq!(report.overdue_threshold_secs, 100.0);
    assert!(
        report.overdue,
        "350s after the last of a 100s cadence is overdue"
    );

    // now = 2050 → elapsed 50 < 100 → not overdue.
    let fresh = inter_event_hazard(&times, last + 50.0).unwrap();
    assert_eq!(fresh.survival, 1.0);
    assert!(!fresh.overdue, "50s into a 100s cadence is not overdue");
}

#[test]
fn gamma_path_overdue_crosses_at_the_survival_threshold() {
    // Jittered ~100s cadence so CV>0 and the Gamma renewal branch runs.
    let gaps = [
        90.0, 110.0, 95.0, 105.0, 100.0, 108.0, 92.0, 103.0, 97.0, 100.0,
    ];
    let mut times = vec![0.0];
    for g in gaps {
        times.push(times.last().unwrap() + g);
    }
    let last = *times.last().unwrap();

    // overdue_threshold_secs is, by construction, where S=α. Just past it the
    // event must be overdue; just before it, not — trigger → outcome.
    let probe = inter_event_hazard(&times, last + 1.0).unwrap();
    assert!(!probe.deterministic);
    let threshold = probe.overdue_threshold_secs;
    assert!(
        threshold > 100.0,
        "threshold {threshold} should exceed the mean cadence"
    );

    let before = inter_event_hazard(&times, last + threshold - 5.0).unwrap();
    let after = inter_event_hazard(&times, last + threshold + 5.0).unwrap();
    assert!(!before.overdue, "elapsed below threshold is not overdue");
    assert!(after.overdue, "elapsed above threshold is overdue");
    // Survival is monotone-decreasing in elapsed.
    assert!(before.survival > after.survival);
    assert!(after.survival <= before.alpha);
}

#[test]
fn cusum_localizes_a_planted_rate_shift_at_the_known_index() {
    // 20 gaps of 100 (slow) then 20 gaps of 20 (fast): the rate jumps 5× at
    // gap index 20, i.e. occurrence index 20 at t = 1000 + 20*100 = 3000.
    let mut times = vec![1000.0];
    for _ in 0..20 {
        times.push(times.last().unwrap() + 100.0);
    }
    for _ in 0..20 {
        times.push(times.last().unwrap() + 20.0);
    }
    let report =
        recurrence_rate_cusum_with_config(&times, &CusumConfig::with_baseline(10)).unwrap();

    let cp = report.change_point.expect("planted shift must fire");
    assert_eq!(cp.gap_index, 20, "onset at the first shifted gap");
    assert_eq!(cp.occurrence_index, 20);
    assert_eq!(
        cp.change_time, 3000.0,
        "absolute timestamp of the regime change"
    );
    assert_eq!(cp.direction, RateShift::SpeedUp, "gaps shrank → rate ↑");
    assert_eq!(report.baseline_mean_gap, 100.0);
}

#[test]
fn cusum_holds_steady_with_no_false_alarm() {
    // A perfectly steady 50s cadence must not fabricate a change-point.
    let times: Vec<f64> = (0..30).map(|k| 500.0 + k as f64 * 50.0).collect();
    let report = recurrence_rate_cusum(&times).unwrap();
    assert!(
        report.change_point.is_none(),
        "steady rate has no change-point"
    );
    assert_eq!(report.baseline_mean_gap, 50.0);
}

#[test]
fn slowdown_shift_is_detected_with_the_opposite_direction() {
    // Fast (gap 20) then slow (gap 200): rate drops → SlowDown.
    let mut times = vec![0.0];
    for _ in 0..12 {
        times.push(times.last().unwrap() + 20.0);
    }
    for _ in 0..12 {
        times.push(times.last().unwrap() + 200.0);
    }
    let report = recurrence_rate_cusum_with_config(&times, &CusumConfig::with_baseline(6)).unwrap();
    let cp = report.change_point.expect("slow-down must fire");
    assert_eq!(cp.direction, RateShift::SlowDown);
    assert_eq!(cp.gap_index, 12);
}

#[test]
fn detectors_are_bit_deterministic() {
    let times: Vec<f64> = (0..16).map(|k| k as f64 * 7.0).collect();
    assert_eq!(
        inter_event_hazard(&times, 200.0).unwrap(),
        inter_event_hazard(&times, 200.0).unwrap()
    );
    assert_eq!(
        recurrence_rate_cusum(&times).unwrap(),
        recurrence_rate_cusum(&times).unwrap()
    );
}

#[test]
fn fail_closed_error_codes_are_exact() {
    let good: Vec<f64> = (0..12).map(|k| k as f64 * 100.0).collect();

    // Hazard: too few occurrences.
    let short = inter_event_hazard(&[0.0, 1.0, 2.0], 5.0).unwrap_err();
    assert_eq!(short.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert!(short.message.contains("≥ 4 occurrences"));

    // Hazard: now precedes the last occurrence.
    let backwards = inter_event_hazard(&good, good[good.len() - 1] - 10.0).unwrap_err();
    assert_eq!(backwards.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert!(backwards.message.contains("elapsed is negative"));

    // Hazard: non-increasing times.
    let mut bad = good.clone();
    bad[5] = bad[4];
    let unordered = inter_event_hazard(&bad, 9_999.0).unwrap_err();
    assert_eq!(unordered.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert!(unordered.message.contains("strictly increasing"));

    // Hazard: NaN occurrence.
    let mut nan = good.clone();
    nan[3] = f64::NAN;
    let nan_err = inter_event_hazard(&nan, 9_999.0).unwrap_err();
    assert_eq!(nan_err.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert!(nan_err.message.contains("NaN or infinite"));

    // Hazard: invalid alpha (must be in [0,1)).
    let bad_alpha = inter_event_hazard_with_alpha(&good, 9_999.0, 1.0).unwrap_err();
    assert_eq!(bad_alpha.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert!(bad_alpha.message.contains("alpha"));

    // CUSUM: baseline window not smaller than the gap count.
    let bad_baseline =
        recurrence_rate_cusum_with_config(&good, &CusumConfig::with_baseline(50)).unwrap_err();
    assert_eq!(bad_baseline.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert!(bad_baseline.message.contains("baseline_gaps"));

    // CUSUM: too few occurrences.
    let cusum_short = recurrence_rate_cusum(&[0.0, 1.0, 2.0]).unwrap_err();
    assert_eq!(cusum_short.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
}

// ── Vault-backed on-disk SoT readback ───────────────────────────────────────

/// Build a durable vault, append a planted occurrence series to the Recurrence
/// CF (the on-disk SoT), then read it back — proving the bytes, not the call.
fn series_from_vault(name: &str, planted_gaps: &[i64], base_t: i64) -> RecurrenceSeries {
    let nonce = std::process::id();
    let dir = std::env::temp_dir().join(format!("calyx-issue585-{name}-{nonce}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create vault dir");
    let vault_id: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id");
    let vault = AsterVault::new_durable(
        dir.join("vault"),
        vault_id,
        format!("issue585-{name}-salt").into_bytes(),
        VaultOptions::default(),
    )
    .expect("open durable vault");

    let cx_id = vault.cx_id_for_input(name.as_bytes(), 41);
    vault.put(base_cx(cx_id, vault_id)).expect("put base");
    vault.flush().expect("flush base");

    let mut t = base_t;
    let mut times = vec![t];
    for &g in planted_gaps {
        t += g;
        times.push(t);
    }
    for &time in &times {
        append_occurrence(
            &vault,
            cx_id,
            EpochSecs(time),
            OccurrenceContext::new(Vec::new()).expect("context"),
            EpochSecs(time),
            RetentionPolicy::default(),
        )
        .expect("append occurrence to recurrence CF");
    }

    // Independent read of the source of truth.
    let series = read_series(&vault, cx_id).expect("read recurrence series back");
    assert_eq!(
        series.occurrences.len(),
        times.len(),
        "all planted occurrences must be physically present in the CF"
    );
    let _ = fs::remove_dir_all(&dir);
    series
}

fn base_cx(cx_id: CxId, vault_id: VaultId) -> Constellation {
    Constellation {
        cx_id,
        vault_id,
        panel_version: 41,
        created_at: 100,
        input_ref: InputRef {
            hash: *blake3::hash(b"issue585-recurrence-hazard").as_bytes(),
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
    }
}

#[test]
fn vault_readback_overdue_fires_at_expected_time() {
    // 10 gaps of 100s from t=1000 → last occurrence at t=2000.
    let series = series_from_vault("overdue", &[100; 10], 1000);
    let report = inter_event_hazard_from_series(&series, EpochSecs(2350), 0.05).unwrap();
    assert!(
        report.overdue,
        "the recurrence is 350s overdue against a 100s cadence"
    );
    assert_eq!(report.expected_next, 2100.0);
    assert_eq!(report.survival, 0.0);

    let fresh = inter_event_hazard_from_series(&series, EpochSecs(2050), 0.05).unwrap();
    assert!(!fresh.overdue, "50s in is not overdue");
}

#[test]
fn vault_readback_cusum_finds_planted_change_point() {
    // 20 gaps of 100 then 20 of 20, from t=1000 → change at t=3000.
    let mut gaps = vec![100_i64; 20];
    gaps.extend(std::iter::repeat_n(20_i64, 20));
    let series = series_from_vault("cusum", &gaps, 1000);
    let report =
        recurrence_rate_cusum_from_series(&series, &CusumConfig::with_baseline(10)).unwrap();
    let cp = report
        .change_point
        .expect("planted shift fires on the read-back series");
    assert_eq!(cp.occurrence_index, 20);
    assert_eq!(cp.change_time, 3000.0);
    assert_eq!(cp.direction, RateShift::SpeedUp);
}

/// manual FSV: writes the byte-readback SoT JSON. Run with
/// `CALYX_FSV_ROOT=/var/lib/calyx/data/fsv-issue585-recurrence-hazard \
///  cargo test -p calyx-assay --test recurrence_hazard_fsv -- --ignored --nocapture`
#[test]
#[ignore = "manual FSV writes recurrence-hazard source-of-truth readbacks"]
fn recurrence_hazard_manual_fsv() {
    let root = fsv_root();
    fs::create_dir_all(&root).expect("create fsv root");

    // 1. Overdue hazard from a real vault read-back (the 2+2=4 case).
    let hazard_series = series_from_vault("fsv-overdue", &[100; 10], 1000);
    let overdue = inter_event_hazard_from_series(&hazard_series, EpochSecs(2350), 0.05).unwrap();
    let not_overdue =
        inter_event_hazard_from_series(&hazard_series, EpochSecs(2050), 0.05).unwrap();
    write_json(
        &root.join("issue585_hazard.json"),
        &json!({
            "planted_cadence_secs": 100,
            "n_occurrences": hazard_series.occurrences.len(),
            "last_occurrence": 2000,
            "overdue_probe_now": 2350,
            "overdue": overdue.overdue,
            "overdue_survival": overdue.survival,
            "expected_next": overdue.expected_next,
            "overdue_threshold_secs": overdue.overdue_threshold_secs,
            "fresh_probe_now": 2050,
            "fresh_overdue": not_overdue.overdue,
            "fresh_survival": not_overdue.survival,
        }),
    );

    // 2. CUSUM change-point from a real vault read-back.
    let mut gaps = vec![100_i64; 20];
    gaps.extend(std::iter::repeat_n(20_i64, 20));
    let cusum_series = series_from_vault("fsv-cusum", &gaps, 1000);
    let cusum =
        recurrence_rate_cusum_from_series(&cusum_series, &CusumConfig::with_baseline(10)).unwrap();
    let cp = cusum.change_point.expect("planted shift");
    write_json(
        &root.join("issue585_cusum.json"),
        &json!({
            "planted_change_gap_index": 20,
            "planted_change_time": 3000,
            "detected_gap_index": cp.gap_index,
            "detected_change_time": cp.change_time,
            "detected_direction": cp.direction,
            "alarm_gap_index": cp.alarm_gap_index,
            "baseline_mean_gap": cusum.baseline_mean_gap,
            "baseline_sigma": cusum.baseline_sigma,
        }),
    );

    // 3. Edge cases: SoT state before (input spec) and after (exact outcome).
    let good: Vec<f64> = (0..12).map(|k| k as f64 * 100.0).collect();
    let mut nan = good.clone();
    nan[3] = f64::NAN;
    let edges = json!([
        edge_case(
            "too_few_occurrences",
            json!({"n_occurrences": 3, "min": 4}),
            inter_event_hazard(&[0.0, 1.0, 2.0], 5.0).map(|_| "report".to_string()),
        ),
        edge_case(
            "now_before_last",
            json!({"last": good[good.len() - 1], "now": good[good.len() - 1] - 10.0}),
            inter_event_hazard(&good, good[good.len() - 1] - 10.0).map(|_| "report".to_string()),
        ),
        edge_case(
            "nan_occurrence",
            json!({"n_occurrences": 12, "times[3]": "NaN"}),
            inter_event_hazard(&nan, 9_999.0).map(|_| "report".to_string()),
        ),
        edge_case(
            "cusum_baseline_too_large",
            json!({"n_gaps": 11, "baseline_gaps": 50}),
            recurrence_rate_cusum_with_config(&good, &CusumConfig::with_baseline(50))
                .map(|_| "report".to_string()),
        ),
    ]);
    write_json(&root.join("issue585_edges.json"), &edges);
    println!(
        "FSV evidence written under {} — read back issue585_hazard.json, issue585_cusum.json, issue585_edges.json",
        root.display()
    );
}

fn edge_case(
    name: &str,
    state_before: serde_json::Value,
    outcome: Result<String, calyx_core::CalyxError>,
) -> serde_json::Value {
    let state_after = match outcome {
        Ok(detail) => json!({"ok": detail}),
        Err(error) => json!({"error_code": error.code, "message": error.message}),
    };
    json!({"case": name, "state_before": state_before, "state_after": state_after})
}

fn write_json(path: &PathBuf, value: &serde_json::Value) {
    fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-recurrence-hazard-fsv"))
}
