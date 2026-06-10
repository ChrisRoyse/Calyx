use calyx_aster::dedup::{EpochSecs, compression_ratio};
use calyx_aster::recurrence::read_series;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Clock, CxId, Result};
use serde::{Deserialize, Serialize};

pub const FREQ_BONUS_MAX: u64 = 10_000;
pub const CALYX_ANNEAL_INVALID_CADENCE: &str = "CALYX_ANNEAL_INVALID_CADENCE";

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecurrenceSchedule {
    pub cx_id: CxId,
    pub importance_weight: f32,
    pub next_expected_t: Option<EpochSecs>,
    pub refresh_priority: RefreshPriority,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefreshPriority {
    Hot,
    Warm,
    Cold,
    OneTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetentionTier {
    Memtable,
    SstableTier1,
    Archive,
}

pub fn recurrence_schedule_for<C>(
    cx_id: CxId,
    vault: &AsterVault<C>,
    _clock: &dyn Clock,
) -> Result<RecurrenceSchedule>
where
    C: Clock,
{
    let frequency = compression_ratio(cx_id, vault)?.original_count;
    let series = read_series(vault, cx_id)?;
    let cadence = cadence_from_series(&series);
    let last_occurrence_t = series
        .occurrences
        .iter()
        .map(|occurrence| occurrence.t_k)
        .max();
    Ok(RecurrenceSchedule {
        cx_id,
        importance_weight: frequency_kernel_bonus(frequency),
        next_expected_t: next_expected_t(last_occurrence_t, cadence)?,
        refresh_priority: refresh_priority(cadence),
    })
}

pub fn anneal_retention_tier<C>(
    cx_id: CxId,
    vault: &AsterVault<C>,
    clock: &dyn Clock,
) -> Result<RetentionTier>
where
    C: Clock,
{
    let schedule = recurrence_schedule_for(cx_id, vault, clock)?;
    Ok(retention_tier(schedule.refresh_priority))
}

#[inline]
pub fn frequency_kernel_bonus(frequency: u64) -> f32 {
    if frequency == 0 {
        return 0.0;
    }
    let capped = frequency.min(FREQ_BONUS_MAX) as f32;
    let denom = (FREQ_BONUS_MAX as f32 + 1.0).ln();
    ((capped + 1.0).ln() / denom).min(1.0)
}

fn cadence_from_series(series: &calyx_aster::recurrence::RecurrenceSeries) -> Option<f64> {
    series.cadence_secs.or_else(|| {
        series.rollup_summary.as_ref().and_then(|summary| {
            (summary.period_estimate_secs > 0.0).then_some(summary.period_estimate_secs)
        })
    })
}

fn next_expected_t(
    last_occurrence_t: Option<EpochSecs>,
    cadence: Option<f64>,
) -> Result<Option<EpochSecs>> {
    let Some(last) = last_occurrence_t else {
        return Ok(None);
    };
    let Some(cadence) = cadence else {
        return Ok(None);
    };
    if !cadence.is_finite() || cadence < 0.0 || cadence.ceil() > i64::MAX as f64 {
        return Err(CalyxError {
            code: CALYX_ANNEAL_INVALID_CADENCE,
            message: "recurrence cadence_secs must be a finite non-negative value".to_string(),
            remediation: "rebuild recurrence series with monotonic occurrence timestamps",
        });
    }
    let offset = cadence.ceil() as i64;
    let next = last
        .0
        .checked_add(offset)
        .ok_or_else(|| CalyxError::aster_corrupt_shard("next recurrence timestamp overflow"))?;
    Ok(Some(EpochSecs(next)))
}

fn refresh_priority(cadence: Option<f64>) -> RefreshPriority {
    match cadence {
        None => RefreshPriority::OneTime,
        Some(value) if value < 3_600.0 => RefreshPriority::Hot,
        Some(value) if value < 86_400.0 => RefreshPriority::Warm,
        Some(_) => RefreshPriority::Cold,
    }
}

fn retention_tier(priority: RefreshPriority) -> RetentionTier {
    match priority {
        RefreshPriority::Hot => RetentionTier::Memtable,
        RefreshPriority::Warm => RetentionTier::SstableTier1,
        RefreshPriority::Cold | RefreshPriority::OneTime => RetentionTier::Archive,
    }
}
