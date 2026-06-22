//! PH52 / issue #584 FSV: Lomb-Scargle + autocorrelation periodicity.
//!
//! Synthetic known-I/O discipline: every planted input has a hand-computed
//! expected output (planted period 7.0 -> detected period in [6.65, 7.35]).
//! The `#[ignore]` test writes the byte-readback evidence JSON consumed by the
//! manual FSV (`CALYX_FSV_ROOT`).

use std::fs;
use std::path::PathBuf;

use calyx_assay::{
    MIN_PERIODICITY_SAMPLES, PeriodogramConfig, SIGNIFICANT_PEAK_FAP, autocorrelation,
    bin_event_counts, lomb_scargle, lomb_scargle_with_config,
};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde_json::json;

const PLANTED_PERIOD: f64 = 7.0;
const FSV_SEED: u64 = 42;

/// 100 events at `t_k ~ 7k + N(0, 0.3)` (T06 card synthetic), binned to a
/// unit-width count series for the point-process periodogram path.
fn planted_event_series() -> (Vec<f64>, Vec<f64>) {
    let mut rng = ChaCha8Rng::seed_from_u64(FSV_SEED);
    let times: Vec<f64> = (0..100)
        .map(|k| k as f64 * PLANTED_PERIOD + 0.3 * standard_normal(&mut rng))
        .collect();
    bin_event_counts(&times, 1.0).expect("planted event stream bins cleanly")
}

fn standard_normal(rng: &mut ChaCha8Rng) -> f64 {
    let u1: f64 = rng.gen_range(f64::EPSILON..1.0);
    let u2: f64 = rng.gen_range(0.0..1.0);
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

fn irregular_times(n: usize, mean_gap: f64, seed: u64) -> Vec<f64> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut t = 0.0;
    (0..n)
        .map(|_| {
            t += rng.gen_range(0.2 * mean_gap..1.8 * mean_gap);
            t
        })
        .collect()
}

#[test]
fn planted_period_recovered_within_5pct() {
    let (centres, counts) = planted_event_series();
    let report = lomb_scargle(&centres, &counts).unwrap();
    let dominant = report
        .dominant()
        .expect("planted series has a dominant peak");
    assert!(
        (dominant.period - PLANTED_PERIOD).abs() <= 0.05 * PLANTED_PERIOD,
        "detected {} not within 5% of planted {PLANTED_PERIOD}",
        dominant.period
    );
    assert!(
        dominant.false_alarm_probability <= SIGNIFICANT_PEAK_FAP,
        "planted peak FAP {} above {SIGNIFICANT_PEAK_FAP}",
        dominant.false_alarm_probability
    );
}

#[test]
fn noiseless_sinusoid_recovers_period_with_high_power() {
    let times = irregular_times(120, 1.0, 7);
    let values: Vec<f64> = times
        .iter()
        .map(|t| (2.0 * std::f64::consts::PI * t / PLANTED_PERIOD).sin())
        .collect();
    let report = lomb_scargle(&times, &values).unwrap();
    let dominant = report.dominant().unwrap();
    assert!((dominant.period - PLANTED_PERIOD).abs() <= 0.05 * PLANTED_PERIOD);
    assert!(
        dominant.power > 0.95,
        "noiseless sinusoid power {} should be near 1",
        dominant.power
    );
    assert!(dominant.false_alarm_probability <= SIGNIFICANT_PEAK_FAP);
}

#[test]
fn two_overlapping_periods_both_detected() {
    let times = irregular_times(150, 1.0, 11);
    let values: Vec<f64> = times
        .iter()
        .map(|t| {
            (2.0 * std::f64::consts::PI * t / 7.0).sin()
                + (2.0 * std::f64::consts::PI * t / 11.0).sin()
        })
        .collect();
    let report = lomb_scargle(&times, &values).unwrap();
    for planted in [7.0, 11.0] {
        assert!(
            report
                .peaks
                .iter()
                .any(|peak| (peak.period - planted).abs() <= 0.05 * planted),
            "no peak within 5% of overlapping planted period {planted}; peaks: {:?}",
            report.peaks
        );
    }
}

#[test]
fn autocorrelation_cross_checks_planted_period() {
    let (centres, counts) = planted_event_series();
    let report = autocorrelation(&centres, &counts).unwrap();
    let dominant = report
        .dominant_period
        .expect("planted series has an ACF peak");
    assert!(
        (dominant - PLANTED_PERIOD).abs() <= 1.0,
        "ACF dominant lag {dominant} must be the fundamental {PLANTED_PERIOD}, not a harmonic"
    );
}

#[test]
fn pure_noise_has_no_significant_peak() {
    let times = irregular_times(100, 1.0, 13);
    let mut rng = ChaCha8Rng::seed_from_u64(99);
    let values: Vec<f64> = (0..100).map(|_| standard_normal(&mut rng)).collect();
    let report = lomb_scargle(&times, &values).unwrap();
    assert!(
        report.significant_peaks(SIGNIFICANT_PEAK_FAP).is_empty(),
        "noise must not produce a significant period; peaks: {:?}",
        report.peaks
    );
}

#[test]
fn bin_event_counts_hand_computed_io() {
    let events = [0.0, 0.4, 1.2, 2.5, 2.7, 6.9];
    let (centres, counts) = bin_event_counts(&events, 1.0).unwrap();
    assert_eq!(counts, vec![2.0, 1.0, 2.0, 0.0, 0.0, 0.0, 1.0]);
    assert_eq!(centres, vec![0.5, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5]);
}

#[test]
fn reports_are_bit_deterministic() {
    let (centres, counts) = planted_event_series();
    let first = lomb_scargle(&centres, &counts).unwrap();
    let second = lomb_scargle(&centres, &counts).unwrap();
    assert_eq!(first, second);
    let acf_first = autocorrelation(&centres, &counts).unwrap();
    let acf_second = autocorrelation(&centres, &counts).unwrap();
    assert_eq!(acf_first, acf_second);
}

#[test]
fn fail_closed_error_codes_are_exact() {
    let good_times: Vec<f64> = (0..20).map(|i| i as f64).collect();
    let good_values: Vec<f64> = good_times.iter().map(|t| t.sin()).collect();

    let empty = lomb_scargle(&[], &[]).unwrap_err();
    assert_eq!(empty.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let short = lomb_scargle(&good_times[..5], &good_values[..5]).unwrap_err();
    assert_eq!(short.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert!(short.message.contains(&MIN_PERIODICITY_SAMPLES.to_string()));

    let mismatched = lomb_scargle(&good_times, &good_values[..10]).unwrap_err();
    assert_eq!(mismatched.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let mut nan_values = good_values.clone();
    nan_values[3] = f64::NAN;
    let nan = lomb_scargle(&good_times, &nan_values).unwrap_err();
    assert_eq!(nan.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert!(nan.message.contains("NaN"));

    let mut bad_times = good_times.clone();
    bad_times[4] = bad_times[5] + 1.0;
    let unordered = lomb_scargle(&bad_times, &good_values).unwrap_err();
    assert_eq!(unordered.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert!(unordered.message.contains("strictly increasing"));

    let constant = lomb_scargle(&good_times, &[3.0; 20]).unwrap_err();
    assert_eq!(constant.code, "CALYX_ASSAY_LOW_SIGNAL");

    let exploding_grid = lomb_scargle_with_config(
        &good_times,
        &good_values,
        &PeriodogramConfig {
            max_frequency: Some(1.0e9),
            ..PeriodogramConfig::default()
        },
    )
    .unwrap_err();
    assert_eq!(exploding_grid.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert!(exploding_grid.message.contains("frequency grid"));

    let zero_fap = lomb_scargle_with_config(
        &good_times,
        &good_values,
        &PeriodogramConfig {
            fap_permutations: 0,
            ..PeriodogramConfig::default()
        },
    )
    .unwrap_err();
    assert_eq!(zero_fap.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert!(zero_fap.message.contains("fap_permutations"));

    let bad_bin = bin_event_counts(&good_times, 0.0).unwrap_err();
    assert_eq!(bad_bin.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
}

/// manual FSV: writes the byte-readback SoT JSON. Run with
/// `CALYX_FSV_ROOT=/var/lib/calyx/data/fsv-issue584-periodicity \
///  cargo test -p calyx-assay --test periodicity_fsv -- --ignored --nocapture`
#[test]
#[ignore = "manual FSV writes periodicity source-of-truth readbacks"]
fn periodicity_manual_fsv() {
    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();

    // 1. Planted period (the 2+2=4 case): hand-known input -> read-back output.
    let (centres, counts) = planted_event_series();
    let report = lomb_scargle(&centres, &counts).unwrap();
    let dominant = report.dominant().unwrap();
    let acf = autocorrelation(&centres, &counts).unwrap();
    write_json(
        &root.join("ph52_period.json"),
        &json!({
            "planted_period": PLANTED_PERIOD,
            "n_events": 100,
            "jitter_sigma": 0.3,
            "seed": FSV_SEED,
            "detected_period": dominant.period,
            "detected_power": dominant.power,
            "false_alarm_probability": dominant.false_alarm_probability,
            "within_5pct": (dominant.period - PLANTED_PERIOD).abs() <= 0.05 * PLANTED_PERIOD,
            "acf_dominant_lag": acf.dominant_period,
            "all_peaks": report.peaks,
        }),
    );

    // 2. Edge cases: SoT state before (input spec) and after (exact outcome).
    let good_times: Vec<f64> = (0..20).map(|i| i as f64).collect();
    let good_values: Vec<f64> = good_times.iter().map(|t| t.sin()).collect();
    let mut rng = ChaCha8Rng::seed_from_u64(99);
    let noise_times = irregular_times(100, 1.0, 13);
    let noise_values: Vec<f64> = (0..100).map(|_| standard_normal(&mut rng)).collect();
    let noise_report = lomb_scargle(&noise_times, &noise_values).unwrap();
    let edges = json!([
        edge_case(
            "empty_input",
            json!({"times": [], "values": []}),
            lomb_scargle(&[], &[]).map(|_| "report".to_string())
        ),
        edge_case(
            "below_min_samples",
            json!({"n": 5, "min": MIN_PERIODICITY_SAMPLES}),
            lomb_scargle(&good_times[..5], &good_values[..5]).map(|_| "report".to_string())
        ),
        edge_case(
            "zero_variance_values",
            json!({"n": 20, "values": "constant 3.0"}),
            lomb_scargle(&good_times, &[3.0; 20]).map(|_| "report".to_string())
        ),
        edge_case("nan_value", json!({"n": 20, "values[3]": "NaN"}), {
            let mut nan_values = good_values.clone();
            nan_values[3] = f64::NAN;
            lomb_scargle(&good_times, &nan_values).map(|_| "report".to_string())
        }),
        edge_case(
            "non_monotonic_times",
            json!({"n": 20, "times[4]": "swapped above times[5]"}),
            {
                let mut bad_times = good_times.clone();
                bad_times[4] = bad_times[5] + 1.0;
                lomb_scargle(&bad_times, &good_values).map(|_| "report".to_string())
            }
        ),
        edge_case(
            "pure_noise_no_fabricated_period",
            json!({"n": 100, "values": "seeded standard normal"}),
            Ok::<String, calyx_core::CalyxError>(format!(
                "report with {} significant peaks at FAP<={SIGNIFICANT_PEAK_FAP}; \
                 top peak FAP {:?}",
                noise_report.significant_peaks(SIGNIFICANT_PEAK_FAP).len(),
                noise_report.dominant().map(|p| p.false_alarm_probability),
            )),
        ),
    ]);
    write_json(&root.join("ph52_period_edges.json"), &edges);
    println!(
        "FSV evidence written under {} — read back ph52_period.json and ph52_period_edges.json",
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
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-periodicity-fsv"))
}
