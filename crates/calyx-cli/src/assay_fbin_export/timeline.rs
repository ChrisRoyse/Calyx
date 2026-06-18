use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use calyx_core::{
    CalyxError, TEMPORAL_LANE_ACTIVE, TEMPORAL_LANE_INACTIVE, TEMPORAL_MISSING_CREATED_AT,
};
use serde::Serialize;

use crate::error::{CliError, CliResult};

use super::data::VectorRow;

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct TimelineScan {
    pub(super) active_rows: usize,
    pub(super) inactive_rows: usize,
    pub(super) duplicate_event_time_rows: usize,
    pub(super) out_of_order_event_time_rows: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct TimelineRow {
    pub(super) row_idx: usize,
    pub(super) id: String,
    pub(super) source_event_time_secs: Option<i64>,
    pub(super) source_event_time_raw: Option<String>,
    pub(super) temporal_lane_state: String,
    pub(super) temporal_inactive_reason: Option<String>,
    pub(super) source_sequence: String,
    pub(super) source_sequence_index: Option<usize>,
    pub(super) query_row: bool,
}

#[derive(Default)]
pub(super) struct TimelineScanBuilder {
    scan: TimelineScan,
    seen_times: BTreeSet<i64>,
    previous_time: Option<i64>,
}

impl TimelineScanBuilder {
    pub(super) fn push(&mut self, row: &TimelineRow) {
        match row.source_event_time_secs {
            Some(secs) => {
                self.scan.active_rows += 1;
                if !self.seen_times.insert(secs) {
                    self.scan.duplicate_event_time_rows += 1;
                }
                if self.previous_time.is_some_and(|prev| secs < prev) {
                    self.scan.out_of_order_event_time_rows += 1;
                }
                self.previous_time = Some(secs);
            }
            None => self.scan.inactive_rows += 1,
        }
    }

    pub(super) fn finish(self) -> TimelineScan {
        self.scan
    }
}

pub(super) fn timeline_row(
    row_idx: usize,
    row: &VectorRow,
    query_count: usize,
) -> CliResult<TimelineRow> {
    let event_time = row.event_time_secs()?;
    let raw = row.event_time_raw();
    let lane_state = row
        .temporal_lane_state
        .as_deref()
        .unwrap_or(if event_time.is_some() {
            TEMPORAL_LANE_ACTIVE
        } else {
            TEMPORAL_LANE_INACTIVE
        });
    match lane_state {
        TEMPORAL_LANE_ACTIVE if event_time.is_none() => {
            return Err(timeline_error(
                "CALYX_FSV_ASSAY_FBIN_EXPORT_TEMPORAL_INVALID",
                format!("row {row_idx} is active but missing source_event_time_secs"),
                "rebuild corpus-build output so active temporal rows carry event time",
            ));
        }
        TEMPORAL_LANE_INACTIVE if event_time.is_some() => {
            return Err(timeline_error(
                "CALYX_FSV_ASSAY_FBIN_EXPORT_TEMPORAL_INVALID",
                format!("row {row_idx} is inactive but carries source_event_time_secs"),
                "do not mix inactive temporal state with fabricated timestamps",
            ));
        }
        TEMPORAL_LANE_ACTIVE | TEMPORAL_LANE_INACTIVE => {}
        other => {
            return Err(timeline_error(
                "CALYX_FSV_ASSAY_FBIN_EXPORT_TEMPORAL_INVALID",
                format!("row {row_idx} has unknown temporal_lane_state {other:?}"),
                "use temporal_lane_state active or inactive",
            ));
        }
    }
    Ok(TimelineRow {
        row_idx,
        id: row.id.clone(),
        source_event_time_secs: event_time,
        source_event_time_raw: raw.map(str::to_string),
        temporal_lane_state: lane_state.to_string(),
        temporal_inactive_reason: inactive_reason(lane_state, row),
        source_sequence: row
            .source_sequence
            .clone()
            .unwrap_or_else(|| "vectors_jsonl_order".to_string()),
        source_sequence_index: row.source_sequence_index,
        query_row: row_idx < query_count,
    })
}

pub(super) fn open_writer(path: &Path) -> CliResult<BufWriter<File>> {
    File::create(path)
        .map(BufWriter::new)
        .map_err(super::io_error)
}

pub(super) fn write_row(writer: &mut BufWriter<File>, row: &TimelineRow) -> CliResult {
    serde_json::to_writer(&mut *writer, row).map_err(CliError::from)?;
    writer.write_all(b"\n").map_err(super::io_error)
}

fn inactive_reason(lane_state: &str, row: &VectorRow) -> Option<String> {
    if lane_state == TEMPORAL_LANE_ACTIVE {
        None
    } else {
        row.temporal_inactive_reason
            .clone()
            .or_else(|| Some(TEMPORAL_MISSING_CREATED_AT.to_string()))
    }
}

fn timeline_error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> CliError {
    CliError::Calyx(CalyxError {
        code,
        message: message.into(),
        remediation,
    })
}
