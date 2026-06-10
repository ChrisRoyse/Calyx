use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::recurrence::{FREQUENCY_SCALAR, read_series};
use calyx_aster::vault::{AsterVault, encode};
use calyx_core::{Clock as CoreClock, CxId, DecayFunction, RecurrenceBoostConfig, Result};
use serde::{Deserialize, Serialize};

use crate::error::{CALYX_SEXTANT_RECURRENCE_READ_ERROR, sextant_error};

use super::score_e2_recency;

const RECURRENCE_HALF_LIFE_SECS: u64 = 3_600;
const FREQ_BONUS_MAX: u64 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecurrenceBoostEvidence {
    pub frequency: u64,
    pub frequency_bonus: f32,
    pub frequency_component: f32,
    pub last_occurrence_t: Option<i64>,
    pub recency_component: f32,
    pub total: f32,
}

pub fn recurrence_boost_score<C>(
    cx_id: CxId,
    vault: &AsterVault<C>,
    query_time_secs: i64,
    config: &RecurrenceBoostConfig,
) -> Result<f32>
where
    C: CoreClock,
{
    Ok(recurrence_boost_evidence(cx_id, vault, query_time_secs, config)?.total)
}

pub fn recurrence_boost_evidence<C>(
    cx_id: CxId,
    vault: &AsterVault<C>,
    query_time_secs: i64,
    config: &RecurrenceBoostConfig,
) -> Result<RecurrenceBoostEvidence>
where
    C: CoreClock,
{
    config.validate()?;
    let frequency = read_frequency(vault, cx_id)?;
    let series = read_series(vault, cx_id).map_err(|error| {
        recurrence_read_error(format!("read recurrence series for {cx_id}: {error}"))
    })?;
    let last_occurrence_t = series
        .occurrences
        .iter()
        .map(|occurrence| occurrence.t_k.0)
        .max();
    Ok(recurrence_boost_from_parts(
        frequency,
        last_occurrence_t,
        query_time_secs,
        config,
    ))
}

pub fn recurrence_boost_from_parts(
    frequency: u64,
    last_occurrence_t: Option<i64>,
    query_time_secs: i64,
    config: &RecurrenceBoostConfig,
) -> RecurrenceBoostEvidence {
    let frequency_bonus = frequency_kernel_bonus(frequency);
    let frequency_component = frequency_bonus * config.frequency_weight;
    let recency_component = last_occurrence_t.map_or(0.0, |last| {
        score_e2_recency(
            last,
            query_time_secs,
            &DecayFunction::Exponential {
                half_life_secs: RECURRENCE_HALF_LIFE_SECS,
            },
        ) * config.recency_weight
    });
    let total = (frequency_component + recency_component)
        .min(config.max_recurrence_boost)
        .clamp(0.0, config.max_recurrence_boost);
    RecurrenceBoostEvidence {
        frequency,
        frequency_bonus,
        frequency_component,
        last_occurrence_t,
        recency_component,
        total,
    }
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

fn read_frequency<C>(vault: &AsterVault<C>, cx_id: CxId) -> Result<u64>
where
    C: CoreClock,
{
    let bytes = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Base, &base_key(cx_id))
        .map_err(|error| recurrence_read_error(format!("read base CF for {cx_id}: {error}")))?
        .ok_or_else(|| recurrence_read_error(format!("base CF row missing for {cx_id}")))?;
    let cx = encode::decode_constellation_base(&bytes)
        .map_err(|error| recurrence_read_error(format!("decode base CF for {cx_id}: {error}")))?;
    let value = cx
        .scalars
        .get(FREQUENCY_SCALAR)
        .ok_or_else(|| recurrence_read_error(format!("{FREQUENCY_SCALAR} missing for {cx_id}")))?;
    if !value.is_finite() || *value < 0.0 || value.fract() != 0.0 || *value > u64::MAX as f64 {
        return Err(recurrence_read_error(format!(
            "{FREQUENCY_SCALAR} must be a non-negative integer for {cx_id}"
        )));
    }
    Ok(*value as u64)
}

fn recurrence_read_error(message: impl Into<String>) -> calyx_core::CalyxError {
    sextant_error(CALYX_SEXTANT_RECURRENCE_READ_ERROR, message)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn recurrence_boost_stays_in_configured_range(
            frequency in any::<u64>(),
            last in proptest::option::of(0_i64..2_000_000),
            query in 0_i64..2_000_000,
            max_boost in 0.0_f32..=0.10,
        ) {
            let config = RecurrenceBoostConfig::new(0.05, 0.05, max_boost)
                .expect("generated valid config");
            let evidence = recurrence_boost_from_parts(frequency, last, query, &config);
            prop_assert!((0.0..=max_boost).contains(&evidence.total));
        }
    }
}
