use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use calyx_aster::cf::{ColumnFamily, base_key, recurrence_prefix_range};
use calyx_aster::recurrence::{
    FREQUENCY_SCALAR, Occurrence, RollupSummary, StoredRecurrenceRow, decode_recurrence_row,
};
use calyx_aster::vault::encode::decode_constellation_base;
use calyx_core::{Constellation, CxId};
use calyx_loom::recurrence::{PeriodicRecallQuery, periodic_fit};
use serde_json::{Value, json};

use crate::cf_read::{hex_bytes, latest_cf_rows};

pub fn readback_recurrence_series(vault: &Path, cx_id: &str) -> crate::error::CliResult {
    let cx_id = CxId::from_str(cx_id).map_err(|error| format!("invalid --cx-id: {error}"))?;
    let recurrence_rows = latest_cf_rows(vault, ColumnFamily::Recurrence)?;
    let base_rows = latest_cf_rows(vault, ColumnFamily::Base)?;
    let series = read_series_rows(cx_id, &recurrence_rows, &base_rows)?;

    println!(
        "{}",
        serde_json::to_string_pretty(&series_json(vault, cx_id, &series))
            .map_err(|error| error.to_string())?
    );
    Ok(())
}

pub fn readback_periodic_recall(args: &[String]) -> crate::error::CliResult {
    let args = PeriodicRecallArgs::parse(args)?;
    let recurrence_rows = latest_cf_rows(&args.vault, ColumnFamily::Recurrence)?;
    let base_rows = latest_cf_rows(&args.vault, ColumnFamily::Base)?;
    let query = PeriodicRecallQuery::new(args.hour, args.day).map_err(|error| error.to_string())?;
    let mut hits = Vec::new();
    for cx_id in recurrence_cx_ids(&recurrence_rows) {
        let series = read_series_rows(cx_id, &recurrence_rows, &base_rows)?;
        let fit = periodic_fit(&series.occurrences);
        if !query.matches(fit) {
            continue;
        }
        hits.push(json!({
            "cx_id": cx_id.to_string(),
            "frequency": series.frequency,
            "occurrence_count": series.occurrences.len(),
            "cadence_secs": series.cadence_secs,
            "periodic_fit": fit,
            "occurrences": series.occurrences.iter().map(occurrence_json).collect::<Vec<_>>(),
        }));
    }
    let value = json!({
        "vault": args.vault.display().to_string(),
        "query": {
            "target_hour": args.hour,
            "target_day_of_week": args.day,
        },
        "hits": hits,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
    );
    Ok(())
}

struct SeriesRows {
    frequency: u64,
    cadence_secs: Option<f64>,
    occurrences: Vec<Occurrence>,
    rollup_summary: Option<RollupSummary>,
    rolled_rows: Vec<Value>,
}

fn read_series_rows(
    cx_id: CxId,
    recurrence_rows: &BTreeMap<Vec<u8>, Vec<u8>>,
    base_rows: &BTreeMap<Vec<u8>, Vec<u8>>,
) -> Result<SeriesRows, String> {
    let base = base_rows
        .get(&base_key(cx_id))
        .map(|bytes| decode_constellation_base(bytes).map_err(|error| error.to_string()))
        .transpose()?;
    let base_frequency = base
        .as_ref()
        .map_or(Ok(0), recurrence_frequency)
        .map_err(|error| error.to_string())?;

    let range = recurrence_prefix_range(cx_id);
    let mut occurrences = Vec::new();
    let mut rolled_rows = Vec::new();
    let mut rollup_summary = None;
    let mut has_tombstone = false;
    for (key, value) in recurrence_rows {
        if !range.contains(key) {
            continue;
        }
        match decode_recurrence_row(value).map_err(|error| error.to_string())? {
            StoredRecurrenceRow::Occurrence(occurrence) => occurrences.push(occurrence),
            StoredRecurrenceRow::RollupSummary(summary) => rollup_summary = Some(summary),
            StoredRecurrenceRow::RolledOccurrence { id, rolled_into } => {
                rolled_rows.push(json!({
                    "key_hex": hex_bytes(key),
                    "id": id.0,
                    "rolled_into": rolled_into.0,
                }));
            }
            StoredRecurrenceRow::Tombstone { .. } => has_tombstone = true,
        }
    }
    occurrences.sort_by_key(|occurrence| (occurrence.t_k, occurrence.id));
    let total_count = occurrences.len() as u64
        + rollup_summary
            .as_ref()
            .map_or(0, |summary| summary.count_rolled);
    let frequency = if has_tombstone {
        total_count
    } else {
        base_frequency.max(total_count)
    };
    Ok(SeriesRows {
        frequency,
        cadence_secs: cadence_secs(&occurrences),
        occurrences,
        rollup_summary,
        rolled_rows,
    })
}

fn series_json(vault: &Path, cx_id: CxId, series: &SeriesRows) -> Value {
    json!({
        "vault": vault.display().to_string(),
        "cx_id": cx_id.to_string(),
        "frequency": series.frequency,
        "occurrence_count": series.occurrences.len(),
        "cadence_secs": series.cadence_secs,
        "periodic_fit": periodic_fit(&series.occurrences),
        "rollup_summary": rollup_summary_json(series.rollup_summary.as_ref()),
        "rolled_rows": series.rolled_rows.clone(),
        "occurrences": series.occurrences.iter().map(occurrence_json).collect::<Vec<_>>(),
    })
}

fn recurrence_cx_ids(recurrence_rows: &BTreeMap<Vec<u8>, Vec<u8>>) -> Vec<CxId> {
    let mut ids = recurrence_rows
        .keys()
        .filter_map(|key| {
            let bytes = key.get(..16)?;
            let mut cx_id = [0_u8; 16];
            cx_id.copy_from_slice(bytes);
            Some(CxId::from_bytes(cx_id))
        })
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

fn recurrence_frequency(cx: &Constellation) -> calyx_core::Result<u64> {
    let Some(value) = cx.scalars.get(FREQUENCY_SCALAR) else {
        return Ok(0);
    };
    if !value.is_finite() || *value < 0.0 || value.fract() != 0.0 {
        return Err(calyx_core::CalyxError::aster_corrupt_shard(
            "recurrence frequency scalar must be a non-negative integer",
        ));
    }
    Ok(*value as u64)
}

fn cadence_secs(occurrences: &[Occurrence]) -> Option<f64> {
    if occurrences.len() < 2 {
        return None;
    }
    let mut gaps = occurrences
        .windows(2)
        .map(|pair| (pair[1].t_k.0 - pair[0].t_k.0) as f64)
        .collect::<Vec<_>>();
    gaps.sort_by(f64::total_cmp);
    let mid = gaps.len() / 2;
    Some(if gaps.len() % 2 == 0 {
        (gaps[mid - 1] + gaps[mid]) / 2.0
    } else {
        gaps[mid]
    })
}

fn rollup_summary_json(summary: Option<&RollupSummary>) -> Value {
    summary.map_or(Value::Null, |summary| {
        json!({
            "oldest_t": summary.oldest_t.0,
            "count_rolled": summary.count_rolled,
            "period_estimate_secs": summary.period_estimate_secs,
        })
    })
}

fn occurrence_json(occurrence: &Occurrence) -> Value {
    json!({
        "id": occurrence.id.0,
        "t_k": occurrence.t_k.0,
        "context_len": occurrence.context.bytes.len(),
        "context_hex": hex_bytes(&occurrence.context.bytes),
    })
}

struct PeriodicRecallArgs {
    vault: PathBuf,
    hour: Option<u8>,
    day: Option<u8>,
}

impl PeriodicRecallArgs {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut vault = None;
        let mut hour = None;
        let mut day = None;
        let mut index = 0;
        while index < args.len() {
            let Some(value) = args.get(index + 1) else {
                return Err(format!("missing value for {}", args[index]));
            };
            match args[index].as_str() {
                "--vault" => vault = Some(PathBuf::from(value)),
                "--hour" => hour = Some(parse_u8(value, "--hour")?),
                "--day" => day = Some(parse_u8(value, "--day")?),
                other => return Err(format!("unknown periodic-recall flag {other}")),
            }
            index += 2;
        }
        Ok(Self {
            vault: vault.ok_or_else(|| "periodic-recall requires --vault <dir>".to_string())?,
            hour,
            day,
        })
    }
}

fn parse_u8(value: &str, flag: &str) -> Result<u8, String> {
    value
        .parse::<u8>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}
