use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::RecurrenceSeries;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Clock, CxId, Result};
use serde::{Deserialize, Serialize};

pub const CALYX_ORACLE_INSUFFICIENT: &str = "CALYX_ORACLE_INSUFFICIENT";
pub const MIN_TIME_PREDICTION_OCCURRENCES: usize = 3;

const FULL_CONFIDENCE_SUPPORT: f32 = 12.0;
const SECS_PER_HOUR: i64 = 3_600;
const SECS_PER_DAY: i64 = 86_400;
const UNIX_EPOCH_DAY_OF_WEEK_MONDAY_ZERO: i64 = 3;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimePrediction {
    pub cx_id: CxId,
    pub sufficient: bool,
    pub support: usize,
    pub t_hat: EpochSecs,
    pub confidence: f32,
    pub confidence_ceiling: f32,
    pub cadence_secs: f64,
    pub cadence_mad_secs: f64,
    pub interval: TimePredictionInterval,
    pub periodic_confidence: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimePredictionInterval {
    pub low: EpochSecs,
    pub high: EpochSecs,
}

pub fn predict_next_occurrence<C>(
    vault: &AsterVault<C>,
    cx_id: CxId,
    confidence_ceiling: f32,
) -> Result<TimePrediction>
where
    C: Clock,
{
    let series = calyx_aster::recurrence::read_series(vault, cx_id)?;
    predict_next_occurrence_from_series(&series, confidence_ceiling)
}

pub fn predict_next_occurrence_from_series(
    series: &RecurrenceSeries,
    confidence_ceiling: f32,
) -> Result<TimePrediction> {
    validate_confidence_ceiling(confidence_ceiling)?;
    let times = sorted_times(series);
    if times.len() < MIN_TIME_PREDICTION_OCCURRENCES {
        return Err(oracle_insufficient(format!(
            "sparse recurrence series support={} min={MIN_TIME_PREDICTION_OCCURRENCES}",
            times.len()
        )));
    }
    let gaps = positive_gaps(&times)?;
    let cadence_secs = median(&gaps);
    if !cadence_secs.is_finite() || cadence_secs <= 0.0 {
        return Err(oracle_insufficient("cadence posterior is not positive"));
    }
    let cadence_mad_secs = median_absolute_deviation(&gaps, cadence_secs);
    let t_hat = checked_time_add(
        *times.last().expect("quorum checked"),
        cadence_secs.round() as i64,
        "next occurrence timestamp overflow",
    )?;
    let confidence = confidence(
        times.len(),
        cadence_secs,
        cadence_mad_secs,
        periodic_confidence(&times),
        confidence_ceiling,
    );
    let half_width = cadence_mad_secs
        .max(cadence_secs * f64::from(1.0 - confidence))
        .round() as i64;
    Ok(TimePrediction {
        cx_id: series.cx_id,
        sufficient: true,
        support: times.len(),
        t_hat: EpochSecs(t_hat),
        confidence,
        confidence_ceiling,
        cadence_secs,
        cadence_mad_secs,
        interval: TimePredictionInterval {
            low: EpochSecs(t_hat.saturating_sub(half_width)),
            high: EpochSecs(t_hat.saturating_add(half_width)),
        },
        periodic_confidence: periodic_confidence(&times),
    })
}

fn validate_confidence_ceiling(confidence_ceiling: f32) -> Result<()> {
    if !confidence_ceiling.is_finite() || !(0.0..=1.0).contains(&confidence_ceiling) {
        return Err(oracle_insufficient(
            "confidence ceiling must be finite and in 0.0..=1.0",
        ));
    }
    Ok(())
}

fn sorted_times(series: &RecurrenceSeries) -> Vec<i64> {
    let mut times = series
        .occurrences
        .iter()
        .map(|occurrence| occurrence.t_k.0)
        .collect::<Vec<_>>();
    times.sort_unstable();
    times
}

fn positive_gaps(times: &[i64]) -> Result<Vec<f64>> {
    times
        .windows(2)
        .map(|pair| {
            let gap = pair[1] - pair[0];
            if gap <= 0 {
                return Err(oracle_insufficient(
                    "recurrence timestamps must be strictly increasing",
                ));
            }
            Ok(gap as f64)
        })
        .collect()
}

fn confidence(
    support: usize,
    cadence_secs: f64,
    cadence_mad_secs: f64,
    periodic_confidence: f32,
    confidence_ceiling: f32,
) -> f32 {
    let regularity = (1.0 - (cadence_mad_secs / cadence_secs)).clamp(0.0, 1.0) as f32;
    let support_confidence = (support as f32 / FULL_CONFIDENCE_SUPPORT).min(1.0);
    (regularity * support_confidence * periodic_confidence)
        .min(confidence_ceiling)
        .clamp(0.0, 1.0)
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

fn median_absolute_deviation(values: &[f64], center: f64) -> f64 {
    let deviations = values
        .iter()
        .map(|value| (value - center).abs())
        .collect::<Vec<_>>();
    median(&deviations)
}

fn periodic_confidence(times: &[i64]) -> f32 {
    hour_day_confidence(times)
        .max(mode_confidence(times, 24, |time| {
            local_hour_and_day(time).0
        }))
        .max(mode_confidence(times, 7, |time| local_hour_and_day(time).1))
}

fn mode_confidence<F>(times: &[i64], domain: usize, bucket: F) -> f32
where
    F: Fn(i64) -> u8,
{
    let mut counts = vec![0_usize; domain];
    for time in times {
        counts[usize::from(bucket(*time))] += 1;
    }
    let max_count = counts.iter().copied().max().unwrap_or(0);
    max_count as f32 / times.len() as f32
}

fn hour_day_confidence(times: &[i64]) -> f32 {
    let mut counts = [0_usize; 24 * 7];
    for time in times {
        let (hour, day) = local_hour_and_day(*time);
        counts[usize::from(day) * 24 + usize::from(hour)] += 1;
    }
    let max_count = counts.iter().copied().max().unwrap_or(0);
    max_count as f32 / times.len() as f32
}

fn local_hour_and_day(time_secs: i64) -> (u8, u8) {
    let local_hour = (time_secs.rem_euclid(SECS_PER_DAY) / SECS_PER_HOUR) as u8;
    let local_day = time_secs.div_euclid(SECS_PER_DAY);
    let day_of_week = (local_day + UNIX_EPOCH_DAY_OF_WEEK_MONDAY_ZERO).rem_euclid(7) as u8;
    (local_hour, day_of_week)
}

fn checked_time_add(time: i64, delta: i64, message: &'static str) -> Result<i64> {
    time.checked_add(delta)
        .ok_or_else(|| oracle_insufficient(message))
}

fn oracle_insufficient(message: impl Into<String>) -> CalyxError {
    CalyxError::oracle_insufficient(message)
}

#[cfg(test)]
mod tests {
    use calyx_aster::dedup::OccurrenceId;
    use calyx_aster::recurrence::{Occurrence, OccurrenceContext};

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

    fn series_with_times(times: impl IntoIterator<Item = i64>) -> RecurrenceSeries {
        let occurrences = times
            .into_iter()
            .enumerate()
            .map(|(index, time)| Occurrence {
                id: OccurrenceId(index as u64),
                t_k: EpochSecs(time),
                context: OccurrenceContext { bytes: Vec::new() },
            })
            .collect::<Vec<_>>();
        RecurrenceSeries {
            cx_id: CxId::from_bytes([0x57; 16]),
            cadence_secs: calyx_aster::recurrence::cadence_secs(&occurrences),
            frequency: occurrences.len() as u64,
            occurrences,
            rollup_summary: None,
        }
    }
}
