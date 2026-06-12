use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::RecurrenceSeries;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Clock, CxId, Result};
use serde::{Deserialize, Serialize};

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
    pub active_support: usize,
    pub rolled_support: u64,
    pub rollup_period_estimate_secs: Option<f64>,
    pub tz_offset_secs: i32,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeBucket {
    pub hour: u8,
    pub day_of_week: u8,
    pub tz_offset_secs: i32,
}

pub fn predict_next_occurrence<C>(
    vault: &AsterVault<C>,
    cx_id: CxId,
    confidence_ceiling: f32,
) -> Result<TimePrediction>
where
    C: Clock,
{
    predict_next_occurrence_with_tz_offset(vault, cx_id, confidence_ceiling, 0)
}

pub fn predict_next_occurrence_with_tz_offset<C>(
    vault: &AsterVault<C>,
    cx_id: CxId,
    confidence_ceiling: f32,
    tz_offset_secs: i32,
) -> Result<TimePrediction>
where
    C: Clock,
{
    let series = calyx_aster::recurrence::read_series(vault, cx_id)?;
    predict_next_occurrence_from_series_with_tz_offset(&series, confidence_ceiling, tz_offset_secs)
}

pub fn predict_next_occurrence_from_series(
    series: &RecurrenceSeries,
    confidence_ceiling: f32,
) -> Result<TimePrediction> {
    predict_next_occurrence_from_series_with_tz_offset(series, confidence_ceiling, 0)
}

pub fn predict_next_occurrence_from_series_with_tz_offset(
    series: &RecurrenceSeries,
    confidence_ceiling: f32,
    tz_offset_secs: i32,
) -> Result<TimePrediction> {
    validate_confidence_ceiling(confidence_ceiling)?;
    let times = sorted_times(series);
    if times.len() < MIN_TIME_PREDICTION_OCCURRENCES {
        let rolled_support = rolled_support(series);
        if rolled_support > 0 {
            return Err(oracle_insufficient(format!(
                "rolled recurrence active support={} rolled_support={} cannot define cadence; min_active={MIN_TIME_PREDICTION_OCCURRENCES}",
                times.len(),
                rolled_support
            )));
        }
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
    let periodic_confidence = periodic_confidence_with_tz_offset(&times, tz_offset_secs);
    let confidence = confidence(
        total_support(series),
        cadence_secs,
        cadence_mad_secs,
        periodic_confidence,
        confidence_ceiling,
    );
    let half_width = cadence_mad_secs
        .max(cadence_secs * f64::from(1.0 - confidence))
        .round() as i64;
    let interval = checked_interval(t_hat, half_width)?;
    Ok(TimePrediction {
        cx_id: series.cx_id,
        sufficient: true,
        support: total_support(series),
        active_support: times.len(),
        rolled_support: rolled_support(series),
        rollup_period_estimate_secs: rollup_period_estimate_secs(series),
        tz_offset_secs,
        t_hat: EpochSecs(t_hat),
        confidence,
        confidence_ceiling,
        cadence_secs,
        cadence_mad_secs,
        interval,
        periodic_confidence,
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

fn total_support(series: &RecurrenceSeries) -> usize {
    usize::try_from(series.frequency.max(series.occurrences.len() as u64)).unwrap_or(usize::MAX)
}

fn rolled_support(series: &RecurrenceSeries) -> u64 {
    series
        .frequency
        .saturating_sub(series.occurrences.len() as u64)
}

fn rollup_period_estimate_secs(series: &RecurrenceSeries) -> Option<f64> {
    series
        .rollup_summary
        .as_ref()
        .and_then(|summary| positive_finite(summary.period_estimate_secs))
}

fn positive_finite(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
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

fn periodic_confidence_with_tz_offset(times: &[i64], tz_offset_secs: i32) -> f32 {
    hour_day_confidence_with_tz_offset(times, tz_offset_secs)
        .max(mode_confidence(times, 24, |time| {
            local_hour_and_day(time, tz_offset_secs).0
        }))
        .max(mode_confidence(times, 7, |time| {
            local_hour_and_day(time, tz_offset_secs).1
        }))
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

fn hour_day_confidence_with_tz_offset(times: &[i64], tz_offset_secs: i32) -> f32 {
    let mut counts = [0_usize; 24 * 7];
    for time in times {
        let (hour, day) = local_hour_and_day(*time, tz_offset_secs);
        counts[usize::from(day) * 24 + usize::from(hour)] += 1;
    }
    let max_count = counts.iter().copied().max().unwrap_or(0);
    max_count as f32 / times.len() as f32
}

pub fn time_bucket(time_secs: i64, tz_offset_secs: i32) -> TimeBucket {
    let local_secs = time_secs.saturating_add(i64::from(tz_offset_secs));
    let local_hour = (local_secs.rem_euclid(SECS_PER_DAY) / SECS_PER_HOUR) as u8;
    let local_day = local_secs.div_euclid(SECS_PER_DAY);
    let day_of_week = (local_day + UNIX_EPOCH_DAY_OF_WEEK_MONDAY_ZERO).rem_euclid(7) as u8;
    TimeBucket {
        hour: local_hour,
        day_of_week,
        tz_offset_secs,
    }
}

fn local_hour_and_day(time_secs: i64, tz_offset_secs: i32) -> (u8, u8) {
    let bucket = time_bucket(time_secs, tz_offset_secs);
    (bucket.hour, bucket.day_of_week)
}

fn checked_time_add(time: i64, delta: i64, message: &'static str) -> Result<i64> {
    time.checked_add(delta)
        .ok_or_else(|| oracle_insufficient(message))
}

fn checked_interval(t_hat: i64, half_width: i64) -> Result<TimePredictionInterval> {
    if half_width < 0 {
        return Err(oracle_insufficient(
            "prediction interval half-width must be non-negative",
        ));
    }
    let low = t_hat
        .checked_sub(half_width)
        .ok_or_else(|| oracle_insufficient("prediction interval low bound overflow"))?;
    let high = t_hat
        .checked_add(half_width)
        .ok_or_else(|| oracle_insufficient("prediction interval high bound overflow"))?;
    Ok(TimePredictionInterval {
        low: EpochSecs(low),
        high: EpochSecs(high),
    })
}

fn oracle_insufficient(message: impl Into<String>) -> CalyxError {
    CalyxError::oracle_insufficient(message)
}

#[cfg(test)]
#[path = "time_prediction_tests.rs"]
mod tests;
