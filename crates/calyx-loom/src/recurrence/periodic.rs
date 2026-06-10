use std::collections::BTreeSet;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::recurrence::{self, Occurrence, RecurrenceSeries};
use calyx_aster::vault::AsterVault;
use calyx_core::{CALYX_TEMPORAL_INVALID_PERIOD, CalyxError, Clock, CxId, Result, VaultStore};
use serde::{Deserialize, Serialize};

const CX_ID_BYTES: usize = 16;
const SECS_PER_HOUR: i64 = 3_600;
const SECS_PER_DAY: i64 = 86_400;
const UNIX_EPOCH_DAY_OF_WEEK_MONDAY_ZERO: i64 = 3;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecurrenceRead {
    pub series: RecurrenceSeries,
    pub periodic_fit: PeriodicFit,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeriodicFit {
    pub target_hour: Option<u8>,
    pub target_day_of_week: Option<u8>,
    pub dominant_period_secs: Option<f64>,
    pub support: usize,
    pub hour_confidence: f32,
    pub day_confidence: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeriodicRecallQuery {
    pub target_hour: Option<u8>,
    pub target_day_of_week: Option<u8>,
}

impl PeriodicRecallQuery {
    pub fn new(target_hour: Option<u8>, target_day_of_week: Option<u8>) -> Result<Self> {
        if target_hour.is_some_and(|hour| hour > 23) {
            return Err(period_error("target_hour must be in 0..=23"));
        }
        if target_day_of_week.is_some_and(|day| day > 6) {
            return Err(period_error("target_day_of_week must be in 0..=6"));
        }
        Ok(Self {
            target_hour,
            target_day_of_week,
        })
    }

    pub fn matches(self, fit: PeriodicFit) -> bool {
        if fit.support < 2 {
            return false;
        }
        self.target_hour
            .is_none_or(|hour| fit.target_hour == Some(hour))
            && self
                .target_day_of_week
                .is_none_or(|day| fit.target_day_of_week == Some(day))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeriodicRecallHit {
    pub cx_id: CxId,
    pub frequency: u64,
    pub occurrence_count: usize,
    pub cadence_secs: Option<f64>,
    pub periodic_fit: PeriodicFit,
}

pub fn recurrence_series<C>(vault: &AsterVault<C>, cx_id: CxId) -> Result<RecurrenceRead>
where
    C: Clock,
{
    let series = recurrence::read_series(vault, cx_id)?;
    let periodic_fit = periodic_fit(&series.occurrences);
    Ok(RecurrenceRead {
        series,
        periodic_fit,
    })
}

pub fn periodic_fit(occurrences: &[Occurrence]) -> PeriodicFit {
    let support = occurrences.len();
    let (target_hour, hour_confidence) = mode(occurrences, 24, |occurrence| {
        local_hour_and_day(occurrence.t_k.0).0
    });
    let (target_day_of_week, day_confidence) = mode(occurrences, 7, |occurrence| {
        local_hour_and_day(occurrence.t_k.0).1
    });
    PeriodicFit {
        target_hour,
        target_day_of_week,
        dominant_period_secs: recurrence::cadence_secs(occurrences),
        support,
        hour_confidence,
        day_confidence,
    }
}

pub fn periodic_recall<C>(
    vault: &AsterVault<C>,
    query: PeriodicRecallQuery,
) -> Result<Vec<PeriodicRecallHit>>
where
    C: Clock,
{
    let mut hits = Vec::new();
    for cx_id in recurrence_cx_ids(vault)? {
        let read = recurrence_series(vault, cx_id)?;
        if !query.matches(read.periodic_fit) {
            continue;
        }
        hits.push(PeriodicRecallHit {
            cx_id,
            frequency: read.series.frequency,
            occurrence_count: read.series.occurrences.len(),
            cadence_secs: read.series.cadence_secs,
            periodic_fit: read.periodic_fit,
        });
    }
    hits.sort_by_key(|hit| hit.cx_id);
    Ok(hits)
}

fn recurrence_cx_ids<C>(vault: &AsterVault<C>) -> Result<BTreeSet<CxId>>
where
    C: Clock,
{
    let mut ids = BTreeSet::new();
    for (key, _) in vault.scan_cf_at(vault.snapshot(), ColumnFamily::Recurrence)? {
        if key.len() < CX_ID_BYTES {
            continue;
        }
        let mut bytes = [0_u8; CX_ID_BYTES];
        bytes.copy_from_slice(&key[..CX_ID_BYTES]);
        ids.insert(CxId::from_bytes(bytes));
    }
    Ok(ids)
}

fn mode<F>(occurrences: &[Occurrence], domain: usize, value: F) -> (Option<u8>, f32)
where
    F: Fn(&Occurrence) -> u8,
{
    if occurrences.is_empty() {
        return (None, 0.0);
    }
    let mut counts = vec![0_usize; domain];
    for occurrence in occurrences {
        counts[usize::from(value(occurrence))] += 1;
    }
    let (bucket, count) = counts
        .into_iter()
        .enumerate()
        .max_by_key(|(bucket, count)| (*count, std::cmp::Reverse(*bucket)))
        .expect("non-empty domain");
    (Some(bucket as u8), count as f32 / occurrences.len() as f32)
}

fn local_hour_and_day(time_secs: i64) -> (u8, u8) {
    let local_hour = (time_secs.rem_euclid(SECS_PER_DAY) / SECS_PER_HOUR) as u8;
    let local_day = time_secs.div_euclid(SECS_PER_DAY);
    let day_of_week = (local_day + UNIX_EPOCH_DAY_OF_WEEK_MONDAY_ZERO).rem_euclid(7) as u8;
    (local_hour, day_of_week)
}

fn period_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_TEMPORAL_INVALID_PERIOD,
        message: message.into(),
        remediation: "set target_hour 0..=23 and day_of_week 0..=6",
    }
}
