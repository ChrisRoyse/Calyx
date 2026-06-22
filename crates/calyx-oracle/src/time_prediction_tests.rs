use calyx_aster::dedup::OccurrenceId;
use calyx_aster::recurrence::{Occurrence, OccurrenceContext, RollupSummary};

use crate::CALYX_ORACLE_INSUFFICIENT;

use super::*;

const TUESDAY_2024_01_02_14H_UTC: i64 = 1_704_204_000;
const WEEK_SECS: i64 = 604_800;

#[test]
fn twelve_weekly_events_predict_next_tuesday_with_ceiling_cap() {
    let series =
        series_with_times((0..12).map(|week| TUESDAY_2024_01_02_14H_UTC + week * WEEK_SECS));

    let prediction = predict_next_occurrence_from_series(&series, 0.91).expect("prediction");

    assert_eq!(
        prediction.t_hat,
        EpochSecs(TUESDAY_2024_01_02_14H_UTC + 12 * WEEK_SECS)
    );
    assert_eq!(prediction.support, 12);
    assert_eq!(prediction.cadence_secs, WEEK_SECS as f64);
    assert_eq!(prediction.cadence_mad_secs, 0.0);
    assert_eq!(prediction.periodic_confidence, 1.0);
    assert_eq!(prediction.confidence, 0.91);
    assert_eq!(prediction.confidence_ceiling, 0.91);
    assert!(prediction.interval.low <= prediction.t_hat);
    assert!(prediction.interval.high >= prediction.t_hat);
}

#[test]
fn sparse_series_fails_closed_with_oracle_insufficient() {
    let series = series_with_times([100, 200]);

    let error = predict_next_occurrence_from_series(&series, 1.0).expect_err("sparse");

    assert_eq!(error.code, CALYX_ORACLE_INSUFFICIENT);
    assert!(error.message.contains("sparse recurrence series"));
}

#[test]
fn empty_series_fails_closed_with_oracle_insufficient() {
    let series = series_with_times([]);

    let error = predict_next_occurrence_from_series(&series, 1.0).expect_err("empty");

    assert_eq!(error.code, CALYX_ORACLE_INSUFFICIENT);
    assert!(error.message.contains("support=0"));
}

#[test]
fn duplicate_times_fail_closed_before_guessing() {
    let series = series_with_times([100, 100, 200]);

    let error = predict_next_occurrence_from_series(&series, 1.0).expect_err("duplicate");

    assert_eq!(error.code, CALYX_ORACLE_INSUFFICIENT);
    assert!(error.message.contains("strictly increasing"));
}

#[test]
fn invalid_confidence_ceiling_fails_closed() {
    let series = series_with_times([100, 200, 300]);

    let error = predict_next_occurrence_from_series(&series, 1.1).expect_err("ceiling");

    assert_eq!(error.code, CALYX_ORACLE_INSUFFICIENT);
    assert!(error.message.contains("confidence ceiling"));
}

#[test]
fn next_occurrence_overflow_fails_closed_before_interval() {
    let series = series_with_times([i64::MAX - 20, i64::MAX - 10, i64::MAX]);

    let error = predict_next_occurrence_from_series(&series, 1.0).expect_err("overflow");

    assert_eq!(error.code, CALYX_ORACLE_INSUFFICIENT);
    assert!(error.message.contains("next occurrence timestamp overflow"));
}

#[test]
fn interval_high_overflow_fails_closed() {
    let series = series_with_times([i64::MAX - 30, i64::MAX - 20, i64::MAX - 10]);

    let error = predict_next_occurrence_from_series(&series, 1.0).expect_err("interval high");

    assert_eq!(error.code, CALYX_ORACLE_INSUFFICIENT);
    assert!(
        error
            .message
            .contains("prediction interval high bound overflow")
    );
}

#[test]
fn checked_interval_low_overflow_fails_closed() {
    let error = checked_interval(i64::MIN, 1).expect_err("interval low");

    assert_eq!(error.code, CALYX_ORACLE_INSUFFICIENT);
    assert!(
        error
            .message
            .contains("prediction interval low bound overflow")
    );
}

#[test]
fn nonzero_offset_changes_prediction_periodic_bucket() {
    let series =
        series_with_times((0..12).map(|week| TUESDAY_2024_01_02_14H_UTC + week * WEEK_SECS));

    let utc = predict_next_occurrence_from_series_with_tz_offset(&series, 1.0, 0)
        .expect("utc prediction");
    let utc_minus_five = predict_next_occurrence_from_series_with_tz_offset(&series, 1.0, -18_000)
        .expect("offset prediction");

    assert_eq!(time_bucket(TUESDAY_2024_01_02_14H_UTC, 0).hour, 14);
    assert_eq!(time_bucket(TUESDAY_2024_01_02_14H_UTC, -18_000).hour, 9);
    assert_eq!(utc.periodic_confidence, 1.0);
    assert_eq!(utc_minus_five.periodic_confidence, 1.0);
    assert_eq!(utc.tz_offset_secs, 0);
    assert_eq!(utc_minus_five.tz_offset_secs, -18_000);
}

#[test]
fn rolled_frequency_lifts_confidence_after_active_cadence_exists() {
    let series = series_with_times_rollup(
        (9..12).map(|week| TUESDAY_2024_01_02_14H_UTC + week * WEEK_SECS),
        12,
        RollupSummary {
            oldest_t: EpochSecs(TUESDAY_2024_01_02_14H_UTC),
            count_rolled: 9,
            period_estimate_secs: WEEK_SECS as f64,
        },
    );

    let prediction = predict_next_occurrence_from_series(&series, 1.0).expect("rolled support");

    assert_eq!(
        prediction.t_hat,
        EpochSecs(TUESDAY_2024_01_02_14H_UTC + 12 * WEEK_SECS)
    );
    assert_eq!(prediction.support, 12);
    assert_eq!(prediction.active_support, 3);
    assert_eq!(prediction.rolled_support, 9);
    assert_eq!(
        prediction.rollup_period_estimate_secs,
        Some(WEEK_SECS as f64)
    );
    assert_eq!(prediction.cadence_secs, WEEK_SECS as f64);
    assert_eq!(prediction.confidence, 1.0);
}

#[test]
fn rolled_frequency_without_active_cadence_fails_closed_explicitly() {
    let series = series_with_times_rollup(
        [TUESDAY_2024_01_02_14H_UTC + 11 * WEEK_SECS],
        12,
        RollupSummary {
            oldest_t: EpochSecs(TUESDAY_2024_01_02_14H_UTC),
            count_rolled: 11,
            period_estimate_secs: WEEK_SECS as f64,
        },
    );

    let error = predict_next_occurrence_from_series(&series, 1.0).expect_err("active sparse");

    assert_eq!(error.code, CALYX_ORACLE_INSUFFICIENT);
    assert!(error.message.contains("active support=1"));
    assert!(error.message.contains("rolled_support=11"));
    assert!(error.message.contains("cannot define cadence"));
}

#[test]
#[ignore = "manual FSV writes #657 interval-bound readback artifact"]
fn time_prediction_interval_bounds_manual_fsv() {
    let root = std::env::var("CALYX_ISSUE657_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-issue657-time-bounds-fsv"));
    std::fs::create_dir_all(&root).expect("create root");

    let high_series = series_with_times([i64::MAX - 30, i64::MAX - 20, i64::MAX - 10]);
    let (high_t_hat, high_half_width) = interval_inputs(&high_series, 1.0).expect("high inputs");
    let high_error =
        predict_next_occurrence_from_series(&high_series, 1.0).expect_err("high overflow");
    let low_error = checked_interval(i64::MIN, 1).expect_err("low overflow");
    let t_hat_series = series_with_times([i64::MAX - 20, i64::MAX - 10, i64::MAX]);
    let t_hat_error =
        predict_next_occurrence_from_series(&t_hat_series, 1.0).expect_err("t_hat overflow");

    let report = format!(
        concat!(
            "{{\n",
            "  \"issue\": 657,\n",
            "  \"high_before_t_hat\": {},\n",
            "  \"high_before_half_width\": {},\n",
            "  \"high_legacy_low\": {},\n",
            "  \"high_legacy_high\": {},\n",
            "  \"high_after_code\": \"{}\",\n",
            "  \"high_after_message\": \"{}\",\n",
            "  \"low_legacy_low\": {},\n",
            "  \"low_legacy_high\": {},\n",
            "  \"low_after_code\": \"{}\",\n",
            "  \"low_after_message\": \"{}\",\n",
            "  \"t_hat_after_code\": \"{}\",\n",
            "  \"t_hat_after_message\": \"{}\"\n",
            "}}\n"
        ),
        high_t_hat,
        high_half_width,
        high_t_hat.saturating_sub(high_half_width),
        high_t_hat.saturating_add(high_half_width),
        high_error.code,
        high_error.message,
        i64::MIN.saturating_sub(1),
        i64::MIN.saturating_add(1),
        low_error.code,
        low_error.message,
        t_hat_error.code,
        t_hat_error.message
    );
    let path = root.join("issue657-time-prediction-bounds-readback.json");
    std::fs::write(&path, report).expect("write report");
    let bytes = std::fs::read(&path).expect("read report");
    let readback = String::from_utf8(bytes.clone()).expect("utf8 report");
    let digest = digest_hex(&bytes);

    println!("ISSUE657_FSV_ROOT={}", root.display());
    println!("ISSUE657_READBACK={}", path.display());
    println!("ISSUE657_READBACK_BLAKE3={digest}");
    println!("{readback}");

    assert!(readback.contains("\"high_after_code\": \"CALYX_ORACLE_INSUFFICIENT\""));
    assert!(
        readback.contains("\"high_after_message\": \"prediction interval high bound overflow\"")
    );
    assert!(readback.contains("\"low_after_code\": \"CALYX_ORACLE_INSUFFICIENT\""));
    assert!(readback.contains("\"low_after_message\": \"prediction interval low bound overflow\""));
    assert!(readback.contains("\"t_hat_after_code\": \"CALYX_ORACLE_INSUFFICIENT\""));
    assert!(readback.contains("\"t_hat_after_message\": \"next occurrence timestamp overflow\""));
}

fn interval_inputs(series: &RecurrenceSeries, confidence_ceiling: f32) -> Result<(i64, i64)> {
    let times = sorted_times(series);
    let gaps = positive_gaps(&times)?;
    let cadence_secs = median(&gaps);
    let cadence_mad_secs = median_absolute_deviation(&gaps, cadence_secs);
    let t_hat = checked_time_add(
        *times.last().expect("test series has quorum"),
        cadence_secs.round() as i64,
        "next occurrence timestamp overflow",
    )?;
    let confidence = confidence(
        times.len(),
        cadence_secs,
        cadence_mad_secs,
        periodic_confidence_with_tz_offset(&times, 0),
        confidence_ceiling,
    );
    let half_width = cadence_mad_secs
        .max(cadence_secs * f64::from(1.0 - confidence))
        .round() as i64;
    Ok((t_hat, half_width))
}

fn digest_hex(bytes: &[u8]) -> String {
    calyx_core::content_address([bytes])
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn series_with_times(times: impl IntoIterator<Item = i64>) -> RecurrenceSeries {
    let occurrences = occurrences_from_times(times);
    RecurrenceSeries {
        cx_id: CxId::from_bytes([0x57; 16]),
        cadence_secs: calyx_aster::recurrence::cadence_secs(&occurrences),
        frequency: occurrences.len() as u64,
        occurrences,
        rollup_summary: None,
    }
}

fn series_with_times_rollup(
    times: impl IntoIterator<Item = i64>,
    frequency: u64,
    rollup_summary: RollupSummary,
) -> RecurrenceSeries {
    let occurrences = occurrences_from_times(times);
    RecurrenceSeries {
        cx_id: CxId::from_bytes([0x57; 16]),
        cadence_secs: calyx_aster::recurrence::cadence_secs(&occurrences),
        frequency,
        occurrences,
        rollup_summary: Some(rollup_summary),
    }
}

fn occurrences_from_times(times: impl IntoIterator<Item = i64>) -> Vec<Occurrence> {
    times
        .into_iter()
        .enumerate()
        .map(|(index, time)| Occurrence {
            id: OccurrenceId(index as u64),
            t_k: EpochSecs(time),
            context: OccurrenceContext { bytes: Vec::new() },
        })
        .collect()
}
