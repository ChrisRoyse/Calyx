use std::path::PathBuf;

use serde::Serialize;

use super::{
    CALYX_TEMPORAL_LOG_BAD_TIMESTAMP, CALYX_TEMPORAL_LOG_CADENCE_MISMATCH,
    CALYX_TEMPORAL_LOG_EMPTY, CALYX_TEMPORAL_LOG_NON_MONOTONIC, temporal_error,
};

#[derive(Clone, Debug, Serialize)]
pub(super) struct LogEvent {
    pub line_number: usize,
    pub timestamp: String,
    pub epoch_secs: i64,
    pub value: String,
}

pub(super) fn parse_events(text: &str, rows: usize) -> Result<Vec<LogEvent>, String> {
    let mut events = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if index == 0 && line.to_ascii_lowercase().starts_with("timestamp,") {
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        let mut columns = line.splitn(3, ',');
        let timestamp = columns
            .next()
            .ok_or_else(|| temporal_error(CALYX_TEMPORAL_LOG_BAD_TIMESTAMP, "missing timestamp"))?
            .trim();
        let value = columns.next().unwrap_or("").trim();
        events.push(LogEvent {
            line_number: index + 1,
            timestamp: timestamp.to_string(),
            epoch_secs: parse_timestamp_utc(timestamp)?,
            value: value.to_string(),
        });
        if events.len() == rows {
            break;
        }
    }
    if events.len() < 3 {
        return Err(temporal_error(
            CALYX_TEMPORAL_LOG_EMPTY,
            format!("need at least 3 timestamp rows, found {}", events.len()),
        ));
    }
    Ok(events)
}

pub(super) fn cadence_gaps(events: &[LogEvent]) -> Result<Vec<i64>, String> {
    events
        .windows(2)
        .map(|pair| {
            let gap = pair[1].epoch_secs - pair[0].epoch_secs;
            if gap <= 0 {
                return Err(temporal_error(
                    CALYX_TEMPORAL_LOG_NON_MONOTONIC,
                    "timestamps must be strictly increasing",
                ));
            }
            Ok(gap)
        })
        .collect()
}

pub(super) fn assert_expected_cadence(gaps: &[i64], expected: i64) -> Result<(), String> {
    if expected <= 0 {
        return Err(temporal_error(
            CALYX_TEMPORAL_LOG_CADENCE_MISMATCH,
            "expected cadence must be positive",
        ));
    }
    if let Some(gap) = gaps.iter().find(|gap| **gap != expected) {
        return Err(temporal_error(
            CALYX_TEMPORAL_LOG_CADENCE_MISMATCH,
            format!("observed gap {gap} does not match expected {expected}"),
        ));
    }
    Ok(())
}

fn parse_timestamp_utc(value: &str) -> Result<i64, String> {
    if value.len() != 19 {
        return Err(temporal_error(
            CALYX_TEMPORAL_LOG_BAD_TIMESTAMP,
            format!("timestamp must be YYYY-MM-DD HH:MM:SS, got {value}"),
        ));
    }
    if !value.is_ascii() {
        return Err(temporal_error(
            CALYX_TEMPORAL_LOG_BAD_TIMESTAMP,
            format!("timestamp must be ASCII YYYY-MM-DD HH:MM:SS, got {value}"),
        ));
    }
    let year = parse_i32(&value[0..4])?;
    expect(value.as_bytes()[4], b'-', value)?;
    let month = parse_u32(&value[5..7])?;
    expect(value.as_bytes()[7], b'-', value)?;
    let day = parse_u32(&value[8..10])?;
    expect(value.as_bytes()[10], b' ', value)?;
    let hour = parse_u32(&value[11..13])?;
    expect(value.as_bytes()[13], b':', value)?;
    let minute = parse_u32(&value[14..16])?;
    expect(value.as_bytes()[16], b':', value)?;
    let second = parse_u32(&value[17..19])?;
    if !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(temporal_error(
            CALYX_TEMPORAL_LOG_BAD_TIMESTAMP,
            format!("timestamp component out of range: {value}"),
        ));
    }
    let days = days_from_civil(year, month, day);
    Ok(days * 86_400 + i64::from(hour * 3_600 + minute * 60 + second))
}

fn parse_i32(value: &str) -> Result<i32, String> {
    value
        .parse()
        .map_err(|_| temporal_error(CALYX_TEMPORAL_LOG_BAD_TIMESTAMP, "invalid year"))
}

fn parse_u32(value: &str) -> Result<u32, String> {
    value
        .parse()
        .map_err(|_| temporal_error(CALYX_TEMPORAL_LOG_BAD_TIMESTAMP, "invalid timestamp digits"))
}

fn expect(actual: u8, expected: u8, value: &str) -> Result<(), String> {
    if actual == expected {
        return Ok(());
    }
    Err(temporal_error(
        CALYX_TEMPORAL_LOG_BAD_TIMESTAMP,
        format!("timestamp separator mismatch: {value}"),
    ))
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = month as i32;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    i64::from(era * 146_097 + doe - 719_468)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub(super) struct Args {
    pub log: PathBuf,
    pub vault: PathBuf,
    pub out: PathBuf,
    pub rows: usize,
    pub expected_cadence_secs: i64,
    pub confidence_ceiling: f32,
}

impl Args {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut parsed = ParsedArgs::default();
        let mut index = 0;
        while index < args.len() {
            let Some(value) = args.get(index + 1) else {
                return Err(format!("missing value for {}", args[index]));
            };
            match args[index].as_str() {
                "--log" => parsed.log = Some(PathBuf::from(value)),
                "--vault" => parsed.vault = Some(PathBuf::from(value)),
                "--out" => parsed.out = Some(PathBuf::from(value)),
                "--rows" => parsed.rows = Some(parse_usize(value, "--rows")?),
                "--expected-cadence-secs" => {
                    parsed.expected_cadence_secs =
                        Some(parse_i64(value, "--expected-cadence-secs")?)
                }
                "--confidence-ceiling" => {
                    parsed.confidence_ceiling = Some(parse_f32(value, "--confidence-ceiling")?)
                }
                other => return Err(format!("unknown temporal-log-recurrence flag {other}")),
            }
            index += 2;
        }
        parsed.finish()
    }
}

#[derive(Default)]
struct ParsedArgs {
    log: Option<PathBuf>,
    vault: Option<PathBuf>,
    out: Option<PathBuf>,
    rows: Option<usize>,
    expected_cadence_secs: Option<i64>,
    confidence_ceiling: Option<f32>,
}

impl ParsedArgs {
    fn finish(self) -> Result<Args, String> {
        Ok(Args {
            log: self
                .log
                .ok_or_else(|| "temporal-log-recurrence requires --log <csv>".to_string())?,
            vault: self
                .vault
                .ok_or_else(|| "temporal-log-recurrence requires --vault <dir>".to_string())?,
            out: self
                .out
                .ok_or_else(|| "temporal-log-recurrence requires --out <json>".to_string())?,
            rows: self
                .rows
                .ok_or_else(|| "temporal-log-recurrence requires --rows <n>".to_string())?,
            expected_cadence_secs: self.expected_cadence_secs.ok_or_else(|| {
                "temporal-log-recurrence requires --expected-cadence-secs <secs>".to_string()
            })?,
            confidence_ceiling: self.confidence_ceiling.ok_or_else(|| {
                "temporal-log-recurrence requires --confidence-ceiling <f>".to_string()
            })?,
        })
    }
}

fn parse_usize(value: &str, flag: &str) -> Result<usize, String> {
    value
        .parse()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

fn parse_i64(value: &str, flag: &str) -> Result<i64, String> {
    value
        .parse()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

fn parse_f32(value: &str, flag: &str) -> Result<f32, String> {
    value
        .parse()
        .map_err(|error| format!("invalid {flag}: {error}"))
}
