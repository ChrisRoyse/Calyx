use std::collections::BTreeSet;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::recurrence::{self, Occurrence, RecurrenceReadStats, RecurrenceSeries};
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
    pub read_stats: RecurrenceReadStats,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeriodicTimeBucket {
    pub target_hour: u8,
    pub target_day_of_week: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeriodicFit {
    pub target_hour: Option<u8>,
    pub target_day_of_week: Option<u8>,
    pub target_hour_day: Option<PeriodicTimeBucket>,
    pub tz_offset_secs: i32,
    pub dominant_period_secs: Option<f64>,
    pub support: usize,
    pub active_support: usize,
    pub rolled_support: u64,
    pub rollup_period_estimate_secs: Option<f64>,
    pub hour_confidence: f32,
    pub day_confidence: f32,
    pub hour_day_confidence: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeriodicRecallQuery {
    pub target_hour: Option<u8>,
    pub target_day_of_week: Option<u8>,
    pub tz_offset_secs: i32,
}

impl PeriodicRecallQuery {
    pub fn new(target_hour: Option<u8>, target_day_of_week: Option<u8>) -> Result<Self> {
        Self::with_tz_offset(target_hour, target_day_of_week, 0)
    }

    pub fn with_tz_offset(
        target_hour: Option<u8>,
        target_day_of_week: Option<u8>,
        tz_offset_secs: i32,
    ) -> Result<Self> {
        if target_hour.is_some_and(|hour| hour > 23) {
            return Err(period_error("target_hour must be in 0..=23"));
        }
        if target_day_of_week.is_some_and(|day| day > 6) {
            return Err(period_error("target_day_of_week must be in 0..=6"));
        }
        if target_hour.is_none() && target_day_of_week.is_none() {
            return Err(period_error(
                "periodic recall requires target_hour or target_day_of_week",
            ));
        }
        Ok(Self {
            target_hour,
            target_day_of_week,
            tz_offset_secs,
        })
    }

    pub fn matches(self, fit: PeriodicFit) -> bool {
        if fit.active_support < 2 {
            return false;
        }
        match (self.target_hour, self.target_day_of_week) {
            (Some(hour), Some(day)) => {
                fit.target_hour_day
                    == Some(PeriodicTimeBucket {
                        target_hour: hour,
                        target_day_of_week: day,
                    })
            }
            (Some(hour), None) => fit.target_hour == Some(hour),
            (None, Some(day)) => fit.target_day_of_week == Some(day),
            (None, None) => false,
        }
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeriodicRecallStats {
    pub index_rows_visited: usize,
    pub candidate_series_count: usize,
    pub series_read_count: usize,
    pub series_range_rows_visited: usize,
    pub series_rows_decoded: usize,
    pub matching_series_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeriodicRecallReadback {
    pub query: PeriodicRecallQuery,
    pub hits: Vec<PeriodicRecallHit>,
    pub stats: PeriodicRecallStats,
}

pub fn recurrence_series<C>(vault: &AsterVault<C>, cx_id: CxId) -> Result<RecurrenceRead>
where
    C: Clock,
{
    recurrence_series_with_tz_offset(vault, cx_id, 0)
}

pub fn recurrence_series_with_tz_offset<C>(
    vault: &AsterVault<C>,
    cx_id: CxId,
    tz_offset_secs: i32,
) -> Result<RecurrenceRead>
where
    C: Clock,
{
    let readback = recurrence::read_series_readback(vault, cx_id)?;
    let series = readback.series;
    let periodic_fit = periodic_fit_for_series_with_tz_offset(&series, tz_offset_secs);
    Ok(RecurrenceRead {
        series,
        periodic_fit,
        read_stats: readback.stats,
    })
}

pub fn periodic_fit(occurrences: &[Occurrence]) -> PeriodicFit {
    periodic_fit_with_tz_offset(occurrences, 0)
}

pub fn periodic_fit_with_tz_offset(occurrences: &[Occurrence], tz_offset_secs: i32) -> PeriodicFit {
    let active_support = occurrences.len();
    let mut ordered = occurrences.to_vec();
    ordered.sort_by_key(|occurrence| (occurrence.t_k, occurrence.id));
    let (target_hour, hour_confidence) = mode(&ordered, 24, |occurrence| {
        local_hour_and_day(occurrence.t_k.0, tz_offset_secs).0
    });
    let (target_day_of_week, day_confidence) = mode(&ordered, 7, |occurrence| {
        local_hour_and_day(occurrence.t_k.0, tz_offset_secs).1
    });
    let (target_hour_day, hour_day_confidence) = hour_day_mode(&ordered, tz_offset_secs);
    PeriodicFit {
        target_hour,
        target_day_of_week,
        target_hour_day,
        tz_offset_secs,
        dominant_period_secs: recurrence::cadence_secs(&ordered),
        support: active_support,
        active_support,
        rolled_support: 0,
        rollup_period_estimate_secs: None,
        hour_confidence,
        day_confidence,
        hour_day_confidence,
    }
}

fn periodic_fit_for_series_with_tz_offset(
    series: &RecurrenceSeries,
    tz_offset_secs: i32,
) -> PeriodicFit {
    let mut fit = periodic_fit_with_tz_offset(&series.occurrences, tz_offset_secs);
    let total_support = series.frequency.max(fit.active_support as u64);
    fit.support = usize::try_from(total_support).unwrap_or(usize::MAX);
    fit.rolled_support = total_support.saturating_sub(fit.active_support as u64);
    fit.rollup_period_estimate_secs = series
        .rollup_summary
        .as_ref()
        .and_then(|summary| positive_finite(summary.period_estimate_secs));
    fit
}

pub fn periodic_recall<C>(
    vault: &AsterVault<C>,
    query: PeriodicRecallQuery,
) -> Result<Vec<PeriodicRecallHit>>
where
    C: Clock,
{
    Ok(periodic_recall_readback(vault, query)?.hits)
}

pub fn periodic_recall_readback<C>(
    vault: &AsterVault<C>,
    query: PeriodicRecallQuery,
) -> Result<PeriodicRecallReadback>
where
    C: Clock,
{
    let index = recurrence_cx_ids(vault)?;
    let mut hits = Vec::new();
    let mut stats = PeriodicRecallStats {
        index_rows_visited: index.rows_visited,
        candidate_series_count: index.ids.len(),
        ..PeriodicRecallStats::default()
    };
    for cx_id in index.ids {
        let read = recurrence_series_with_tz_offset(vault, cx_id, query.tz_offset_secs)?;
        stats.series_read_count += 1;
        stats.series_range_rows_visited += read.read_stats.range_scan_rows;
        stats.series_rows_decoded += read.read_stats.decoded_rows;
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
    stats.matching_series_count = hits.len();
    Ok(PeriodicRecallReadback { query, hits, stats })
}

struct RecurrenceCxIdIndex {
    ids: BTreeSet<CxId>,
    rows_visited: usize,
}

fn recurrence_cx_ids<C>(vault: &AsterVault<C>) -> Result<RecurrenceCxIdIndex>
where
    C: Clock,
{
    let mut ids = BTreeSet::new();
    let rows = vault.scan_cf_at(vault.snapshot(), ColumnFamily::Recurrence)?;
    let rows_visited = rows.len();
    for (key, _) in rows {
        if key.len() < CX_ID_BYTES {
            continue;
        }
        let mut bytes = [0_u8; CX_ID_BYTES];
        bytes.copy_from_slice(&key[..CX_ID_BYTES]);
        ids.insert(CxId::from_bytes(bytes));
    }
    Ok(RecurrenceCxIdIndex { ids, rows_visited })
}

fn mode<F>(occurrences: &[Occurrence], domain: usize, value: F) -> (Option<u8>, f32)
where
    F: Fn(&Occurrence) -> u8,
{
    if occurrences.len() < 2 {
        return (None, 0.0);
    }
    let mut counts = vec![0_usize; domain];
    for occurrence in occurrences {
        counts[usize::from(value(occurrence))] += 1;
    }
    let max_count = counts.iter().copied().max().expect("non-empty domain");
    let tied = counts.iter().filter(|count| **count == max_count).count() > 1;
    let confidence = max_count as f32 / occurrences.len() as f32;
    if tied {
        return (None, confidence);
    }
    let bucket = counts
        .iter()
        .enumerate()
        .find_map(|(bucket, count)| (*count == max_count).then_some(bucket))
        .expect("non-empty domain");
    (Some(bucket as u8), confidence)
}

fn hour_day_mode(
    occurrences: &[Occurrence],
    tz_offset_secs: i32,
) -> (Option<PeriodicTimeBucket>, f32) {
    if occurrences.len() < 2 {
        return (None, 0.0);
    }
    let mut counts = [0_usize; 24 * 7];
    for occurrence in occurrences {
        let (hour, day) = local_hour_and_day(occurrence.t_k.0, tz_offset_secs);
        counts[usize::from(day) * 24 + usize::from(hour)] += 1;
    }
    let max_count = counts.iter().copied().max().expect("non-empty domain");
    let tied = counts.iter().filter(|count| **count == max_count).count() > 1;
    let confidence = max_count as f32 / occurrences.len() as f32;
    if tied {
        return (None, confidence);
    }
    let bucket = counts
        .iter()
        .enumerate()
        .find_map(|(bucket, count)| (*count == max_count).then_some(bucket))
        .expect("non-empty domain");
    (
        Some(PeriodicTimeBucket {
            target_hour: (bucket % 24) as u8,
            target_day_of_week: (bucket / 24) as u8,
        }),
        confidence,
    )
}

pub fn periodic_time_bucket(time_secs: i64, tz_offset_secs: i32) -> PeriodicTimeBucket {
    let local_secs = time_secs.saturating_add(i64::from(tz_offset_secs));
    let local_hour = (local_secs.rem_euclid(SECS_PER_DAY) / SECS_PER_HOUR) as u8;
    let local_day = local_secs.div_euclid(SECS_PER_DAY);
    let day_of_week = (local_day + UNIX_EPOCH_DAY_OF_WEEK_MONDAY_ZERO).rem_euclid(7) as u8;
    PeriodicTimeBucket {
        target_hour: local_hour,
        target_day_of_week: day_of_week,
    }
}

fn local_hour_and_day(time_secs: i64, tz_offset_secs: i32) -> (u8, u8) {
    let bucket = periodic_time_bucket(time_secs, tz_offset_secs);
    (bucket.target_hour, bucket.target_day_of_week)
}

fn positive_finite(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

fn period_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_TEMPORAL_INVALID_PERIOD,
        message: message.into(),
        remediation: "set target_hour 0..=23 and day_of_week 0..=6",
    }
}
