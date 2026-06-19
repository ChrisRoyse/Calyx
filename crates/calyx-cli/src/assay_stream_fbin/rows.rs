use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufRead, BufReader};

use calyx_core::{TEMPORAL_LANE_ACTIVE, TEMPORAL_LANE_INACTIVE, TEMPORAL_MISSING_CREATED_AT};
use serde::Deserialize;
use serde_json::Value;

use crate::error::{CliError, CliResult};
use crate::migrate::temporal::parse_event_time_secs;

use super::args::Args;
use super::{io_error, local_error};

const MIN_ROWS: usize = 50;
#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub(crate) id: String,
    pub(crate) text: String,
    pub(crate) label: usize,
    pub(crate) event_time_secs: Option<i64>,
    pub(crate) event_time_raw: Option<String>,
    pub(crate) temporal_lane_state: String,
    pub(crate) temporal_inactive_reason: Option<String>,
    pub(crate) source_sequence_index: usize,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub(crate) struct TimelineStats {
    pub(crate) active_rows: usize,
    pub(crate) inactive_rows: usize,
    pub(crate) duplicate_event_time_rows: usize,
    pub(crate) out_of_order_event_time_rows: usize,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct RowStats {
    pub(crate) rows: usize,
    pub(crate) label_counts: BTreeMap<String, usize>,
    pub(crate) target_class: usize,
    pub(crate) positives: usize,
    pub(crate) temporal: TimelineStats,
}

#[derive(Deserialize)]
struct RawRow {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    row: Option<usize>,
    text: String,
    label: RawLabel,
    #[serde(default)]
    event_time: Option<Value>,
    #[serde(default)]
    event_time_secs: Option<Value>,
    #[serde(default)]
    source_event_time_secs: Option<Value>,
    #[serde(default)]
    created_at: Option<Value>,
    #[serde(default)]
    timestamp: Option<Value>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawLabel {
    Number(usize),
    String(String),
}

pub(crate) fn scan(args: &Args) -> CliResult<RowStats> {
    let mut limiter = RowLimiter::new(args.limit_per_class);
    let mut labels: BTreeMap<usize, usize> = BTreeMap::new();
    let mut temporal = TimelineAccumulator::default();
    let mut rows = 0usize;
    for_each_line(args, |line_idx, line| {
        let row = parse(line_idx, line)?;
        if !limiter.accept(row.label) {
            return Ok(());
        }
        *labels.entry(row.label).or_insert(0) += 1;
        temporal.push(&row);
        rows += 1;
        Ok(())
    })?;
    validate_stats(args, rows, &labels)?;
    let positives = labels.get(&args.target_class).copied().unwrap_or(0);
    Ok(RowStats {
        rows,
        label_counts: labels
            .into_iter()
            .map(|(label, count)| (label.to_string(), count))
            .collect(),
        target_class: args.target_class,
        positives,
        temporal: temporal.finish(),
    })
}

pub(crate) fn for_each_selected(
    args: &Args,
    mut f: impl FnMut(usize, Row) -> CliResult,
) -> CliResult {
    let mut limiter = RowLimiter::new(args.limit_per_class);
    let mut row_idx = 0usize;
    for_each_line(args, |line_idx, line| {
        let row = parse(line_idx, line)?;
        if !limiter.accept(row.label) {
            return Ok(());
        }
        f(row_idx, row)?;
        row_idx += 1;
        Ok(())
    })
}

fn for_each_line(args: &Args, mut f: impl FnMut(usize, &str) -> CliResult) -> CliResult {
    let file = File::open(&args.rows_jsonl).map_err(io_error)?;
    for (line_idx, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(io_error)?;
        if !line.trim().is_empty() {
            f(line_idx, &line)?;
        }
    }
    Ok(())
}

fn parse(line_idx: usize, line: &str) -> CliResult<Row> {
    let raw: RawRow = serde_json::from_str(line).map_err(|error| {
        local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_INVALID_ROW",
            format!("line {line_idx}: {error}"),
            "fix rows-jsonl so every row has id/source, text, label, and optional event_time",
        )
    })?;
    if raw.text.trim().is_empty() {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_INVALID_ROW",
            format!("line {line_idx} text is empty"),
            "remove empty rows or provide source text before streaming FBIN",
        ));
    }
    let id = row_id(line_idx, &raw)?;
    let label = row_label(line_idx, &raw.label)?;
    let temporal = row_temporal(line_idx, &raw)?;
    Ok(Row {
        id,
        text: raw.text,
        label,
        event_time_secs: temporal.event_time_secs,
        event_time_raw: temporal.event_time_raw,
        temporal_lane_state: temporal.lane_state,
        temporal_inactive_reason: temporal.inactive_reason,
        source_sequence_index: line_idx,
    })
}

struct RowTemporal {
    event_time_secs: Option<i64>,
    event_time_raw: Option<String>,
    lane_state: String,
    inactive_reason: Option<String>,
}

fn row_temporal(line_idx: usize, row: &RawRow) -> CliResult<RowTemporal> {
    let Some((field, value)) = temporal_candidate(row) else {
        return Ok(inactive_temporal());
    };
    if value.is_null() {
        return Ok(inactive_temporal());
    }
    let raw = temporal_raw_text(line_idx, field, value)?;
    let secs = parse_event_time_secs(&raw, line_idx as u64, field).map_err(|error| {
        local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_INVALID_TIMESTAMP",
            format!("line {line_idx} {}", error.message()),
            "use a Unix timestamp integer or parseable timestamp string",
        )
    })?;
    let event_time_secs = i64::try_from(secs).map_err(|_| {
        local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_INVALID_TIMESTAMP",
            format!("line {line_idx} {field} exceeds i64"),
            "use an event timestamp within the supported range",
        )
    })?;
    Ok(RowTemporal {
        event_time_secs: Some(event_time_secs),
        event_time_raw: Some(raw),
        lane_state: TEMPORAL_LANE_ACTIVE.to_string(),
        inactive_reason: None,
    })
}

fn inactive_temporal() -> RowTemporal {
    RowTemporal {
        event_time_secs: None,
        event_time_raw: None,
        lane_state: TEMPORAL_LANE_INACTIVE.to_string(),
        inactive_reason: Some(TEMPORAL_MISSING_CREATED_AT.to_string()),
    }
}

fn temporal_candidate(row: &RawRow) -> Option<(&'static str, &Value)> {
    [
        ("event_time", row.event_time.as_ref()),
        ("event_time_secs", row.event_time_secs.as_ref()),
        (
            "source_event_time_secs",
            row.source_event_time_secs.as_ref(),
        ),
        ("created_at", row.created_at.as_ref()),
        ("timestamp", row.timestamp.as_ref()),
    ]
    .into_iter()
    .find_map(|(field, value)| value.map(|value| (field, value)))
}

fn temporal_raw_text(line_idx: usize, field: &str, value: &Value) -> CliResult<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => number
            .as_u64()
            .map(|value| value.to_string())
            .ok_or_else(|| invalid_temporal_type(line_idx, field)),
        _ => Err(invalid_temporal_type(line_idx, field)),
    }
}

fn invalid_temporal_type(line_idx: usize, field: &str) -> CliError {
    local_error(
        "CALYX_FSV_ASSAY_STREAM_FBIN_INVALID_TIMESTAMP",
        format!("line {line_idx} {field} must be a Unix timestamp integer or timestamp string"),
        "write event times as ISO-8601 strings or integer Unix seconds",
    )
}

fn row_id(line_idx: usize, row: &RawRow) -> CliResult<String> {
    row.id
        .as_deref()
        .or(row.source.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| row.row.map(|idx| format!("row:{idx}")))
        .ok_or_else(|| {
            local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_INVALID_ROW",
                format!("line {line_idx} requires id, source, or row"),
                "provide a stable row identifier",
            )
        })
}

fn row_label(line_idx: usize, label: &RawLabel) -> CliResult<usize> {
    match label {
        RawLabel::Number(value) => Ok(*value),
        RawLabel::String(value) => value.trim().parse::<usize>().map_err(|error| {
            local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_INVALID_ROW",
                format!("line {line_idx} label must be usize: {error}"),
                "write labels as integer class identifiers",
            )
        }),
    }
}

fn validate_stats(args: &Args, rows: usize, labels: &BTreeMap<usize, usize>) -> CliResult {
    if rows < MIN_ROWS {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_INVALID_ROWS",
            format!("need >={MIN_ROWS} rows, got {rows}"),
            "stream a real corpus slice with enough rows for retrieval FSV",
        ));
    }
    if args.query_count > rows {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_QUERY_TOO_LARGE",
            format!("query_count={} exceeds rows={rows}", args.query_count),
            "choose query-count <= streamed row count",
        ));
    }
    let label_set = labels.keys().copied().collect::<BTreeSet<_>>();
    let positives = labels.get(&args.target_class).copied().unwrap_or(0);
    if label_set.len() < 2 || positives == 0 || positives == rows {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_INVALID_ROWS",
            format!(
                "target_class={} positives={positives} total={rows} labels={}",
                args.target_class,
                label_set.len()
            ),
            "stream at least two labels with non-empty target and non-target rows",
        ));
    }
    Ok(())
}

struct RowLimiter {
    limit: Option<usize>,
    counts: BTreeMap<usize, usize>,
}

impl RowLimiter {
    fn new(limit: Option<usize>) -> Self {
        Self {
            limit,
            counts: BTreeMap::new(),
        }
    }

    fn accept(&mut self, label: usize) -> bool {
        let count = self.counts.get(&label).copied().unwrap_or(0);
        if self.limit.is_some_and(|limit| count >= limit) {
            return false;
        }
        self.counts.insert(label, count + 1);
        true
    }
}

#[derive(Default)]
struct TimelineAccumulator {
    stats: TimelineStats,
    seen_times: BTreeSet<i64>,
    previous_time: Option<i64>,
}

impl TimelineAccumulator {
    fn push(&mut self, row: &Row) {
        match row.event_time_secs {
            Some(secs) => {
                self.stats.active_rows += 1;
                if !self.seen_times.insert(secs) {
                    self.stats.duplicate_event_time_rows += 1;
                }
                if self.previous_time.is_some_and(|prev| secs < prev) {
                    self.stats.out_of_order_event_time_rows += 1;
                }
                self.previous_time = Some(secs);
            }
            None => self.stats.inactive_rows += 1,
        }
    }

    fn finish(self) -> TimelineStats {
        self.stats
    }
}
