use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, TEMPORAL_LANE_ACTIVE, TEMPORAL_LANE_INACTIVE};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{CliError, CliResult};

#[derive(Clone, Debug)]
pub(super) struct Timeline {
    path: PathBuf,
    rows: Vec<TimelineRow>,
    active_order: Vec<usize>,
    duplicate_event_time_rows: usize,
    out_of_order_event_time_rows: usize,
}

#[derive(Clone, Debug, Deserialize)]
struct TimelineRow {
    row_idx: usize,
    id: String,
    #[serde(default)]
    source_event_time_secs: Option<i64>,
    #[serde(default)]
    source_event_time_raw: Option<String>,
    temporal_lane_state: String,
    #[serde(default)]
    temporal_inactive_reason: Option<String>,
    source_sequence: String,
    #[serde(default)]
    source_sequence_index: Option<usize>,
    #[serde(default)]
    query_row: bool,
}

impl Timeline {
    pub(super) fn load(path: &Path, expected_rows: usize) -> CliResult<Self> {
        let text = fs::read_to_string(path).map_err(|error| {
            timeline_error(
                "CALYX_FSV_PARTITIONED_RRF_TIMELINE_IO",
                format!("read {} failed: {error}", path.display()),
                "pass a valid timeline sidecar produced by assay export-fbin",
            )
        })?;
        let mut rows = Vec::new();
        let mut row_ids = BTreeSet::new();
        let mut seen_times = BTreeSet::new();
        let mut duplicates = 0usize;
        let mut out_of_order = 0usize;
        let mut previous_time = None;
        for (line_idx, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let row: TimelineRow = serde_json::from_str(line).map_err(|error| {
                timeline_error(
                    "CALYX_FSV_PARTITIONED_RRF_TIMELINE_INVALID",
                    format!("line {line_idx}: {error}"),
                    "fix timeline.jsonl before running the fused RRF gate",
                )
            })?;
            validate_row(line_idx, &row)?;
            if row.row_idx != rows.len() {
                return Err(timeline_error(
                    "CALYX_FSV_PARTITIONED_RRF_TIMELINE_INVALID",
                    format!(
                        "line {line_idx} row_idx={} expected={}",
                        row.row_idx,
                        rows.len()
                    ),
                    "timeline row_idx must match fbin row order exactly",
                ));
            }
            if !row_ids.insert(row.row_idx) {
                return Err(timeline_error(
                    "CALYX_FSV_PARTITIONED_RRF_TIMELINE_INVALID",
                    format!("duplicate row_idx {}", row.row_idx),
                    "timeline row_idx values must be unique",
                ));
            }
            if let Some(secs) = row.source_event_time_secs {
                if !seen_times.insert(secs) {
                    duplicates += 1;
                }
                if previous_time.is_some_and(|prev| secs < prev) {
                    out_of_order += 1;
                }
                previous_time = Some(secs);
            }
            rows.push(row);
        }
        if rows.len() != expected_rows {
            return Err(timeline_error(
                "CALYX_FSV_PARTITIONED_RRF_TIMELINE_MISMATCH",
                format!(
                    "timeline rows={} expected corpus rows={expected_rows}",
                    rows.len()
                ),
                "export-fbin must write one timeline row per corpus row",
            ));
        }
        let mut active_order = (0..rows.len())
            .filter(|idx| rows[*idx].source_event_time_secs.is_some())
            .collect::<Vec<_>>();
        active_order.sort_by_key(|idx| (rows[*idx].source_event_time_secs.unwrap(), *idx));
        Ok(Self {
            path: path.to_path_buf(),
            rows,
            active_order,
            duplicate_event_time_rows: duplicates,
            out_of_order_event_time_rows: out_of_order,
        })
    }

    pub(super) fn report(&self) -> Value {
        let active_count = self.active_order.len();
        json!({
            "mode": "event_time_timeline_sidecar",
            "counts_toward_a35": false,
            "timeline_path": self.path,
            "row_count": self.rows.len(),
            "active_count": active_count,
            "inactive_count": self.rows.len().saturating_sub(active_count),
            "duplicate_event_time_rows": self.duplicate_event_time_rows,
            "out_of_order_event_time_rows": self.out_of_order_event_time_rows,
            "first_active": self.active_order.first().map(|idx| self.row_value(*idx)),
            "last_active": self.active_order.last().map(|idx| self.row_value(*idx)),
        })
    }

    pub(super) fn row_value(&self, row_idx: usize) -> Value {
        self.rows
            .get(row_idx)
            .map(row_value)
            .unwrap_or_else(|| json!({ "row_idx": row_idx, "missing": true }))
    }

    pub(super) fn rows_value(&self, row_ids: &[u64]) -> Value {
        Value::Array(
            row_ids
                .iter()
                .map(|id| usize::try_from(*id).ok())
                .map(|idx| {
                    idx.map_or_else(
                        || json!({ "row_idx": null, "missing": true }),
                        |idx| self.row_value(idx),
                    )
                })
                .collect(),
        )
    }

    pub(super) fn time_walk(&self, row_idx: usize) -> Value {
        let Some(row) = self.rows.get(row_idx) else {
            return json!({ "query_row_idx": row_idx, "state": "missing" });
        };
        if row.source_event_time_secs.is_none() {
            return json!({
                "query_row_idx": row_idx,
                "state": TEMPORAL_LANE_INACTIVE,
                "reason": row.temporal_inactive_reason,
            });
        }
        let position = self
            .active_order
            .iter()
            .position(|idx| *idx == row_idx)
            .unwrap_or(0);
        let previous = position.checked_sub(1).map(|idx| self.active_order[idx]);
        let next = self.active_order.get(position + 1).copied();
        let as_of = self.active_order[..=position]
            .iter()
            .copied()
            .rev()
            .take(5)
            .collect::<Vec<_>>();
        let after = self.active_order[position + 1..]
            .iter()
            .copied()
            .take(5)
            .collect::<Vec<_>>();
        json!({
            "mode": "forward_backward_event_time_walk",
            "query_row": row_value(row),
            "previous_event": previous.map(|idx| self.row_value(idx)),
            "current_event": row_value(row),
            "next_event": next.map(|idx| self.row_value(idx)),
            "as_of_row_ids_desc": as_of,
            "after_row_ids_asc": after,
        })
    }
}

pub(super) fn enforce_gate(
    required: bool,
    timeline: Option<&Timeline>,
    truth_n: usize,
) -> CliResult {
    if !required || truth_n == 0 {
        return Ok(());
    }
    let Some(timeline) = timeline else {
        return Err(timeline_error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_REQUIRED",
            "gate-bearing partitioned-rrf recall requires plan.timeline",
            "provide a timeline.jsonl with real source_event_time_secs for every corpus row",
        ));
    };
    if timeline.active_order.len() != timeline.rows.len() {
        return Err(timeline_error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_INACTIVE",
            format!(
                "timeline active rows {} != corpus rows {}",
                timeline.active_order.len(),
                timeline.rows.len()
            ),
            "use timestamped source data or run only as diagnostic/non-gate evidence",
        ));
    }
    Ok(())
}

pub(super) fn resolve_plan_path(plan_path: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        plan_path
            .parent()
            .map(|parent| parent.join(path))
            .unwrap_or_else(|| path.to_path_buf())
    }
}

fn validate_row(line_idx: usize, row: &TimelineRow) -> CliResult {
    if row.id.trim().is_empty() || row.source_sequence.trim().is_empty() {
        return Err(timeline_error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_INVALID",
            format!("line {line_idx} has empty id or source_sequence"),
            "timeline rows must preserve id and source sequence",
        ));
    }
    match row.temporal_lane_state.as_str() {
        TEMPORAL_LANE_ACTIVE if row.source_event_time_secs.is_none() => Err(timeline_error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_INVALID",
            format!("line {line_idx} is active but missing source_event_time_secs"),
            "active temporal rows must carry event time",
        )),
        TEMPORAL_LANE_INACTIVE if row.source_event_time_secs.is_some() => Err(timeline_error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_INVALID",
            format!("line {line_idx} is inactive but carries source_event_time_secs"),
            "inactive rows must not carry fabricated event time",
        )),
        TEMPORAL_LANE_ACTIVE | TEMPORAL_LANE_INACTIVE => Ok(()),
        other => Err(timeline_error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_INVALID",
            format!("line {line_idx} has unknown temporal_lane_state {other:?}"),
            "temporal_lane_state must be active or inactive",
        )),
    }
}

fn row_value(row: &TimelineRow) -> Value {
    json!({
        "row_idx": row.row_idx,
        "id": row.id,
        "source_event_time_secs": row.source_event_time_secs,
        "source_event_time_raw": row.source_event_time_raw,
        "temporal_lane_state": row.temporal_lane_state,
        "temporal_inactive_reason": row.temporal_inactive_reason,
        "source_sequence": row.source_sequence,
        "source_sequence_index": row.source_sequence_index,
        "query_row": row.query_row,
    })
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn timeline_reports_walk_duplicates_and_out_of_order_rows() {
        let root = temp_root("partitioned-rrf-timeline");
        let path = root.join("timeline.jsonl");
        fs::write(
            &path,
            [
                row(0, Some(30), "active"),
                row(1, Some(10), "active"),
                row(2, Some(10), "active"),
                row(3, None, "inactive"),
            ]
            .join("\n")
                + "\n",
        )
        .unwrap();

        let timeline = Timeline::load(&path, 4).unwrap();

        let report = timeline.report();
        assert_eq!(report["active_count"], 3);
        assert_eq!(report["inactive_count"], 1);
        assert_eq!(report["duplicate_event_time_rows"], 1);
        assert_eq!(report["out_of_order_event_time_rows"], 1);
        let walk = timeline.time_walk(1);
        assert_eq!(walk["after_row_ids_asc"][0], 2);
        assert_eq!(walk["after_row_ids_asc"][1], 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn gate_requires_timeline_for_recall_floor_truth() {
        let err = enforce_gate(true, None, 2).unwrap_err();

        assert_eq!(err.code(), "CALYX_FSV_PARTITIONED_RRF_TIMELINE_REQUIRED");
    }

    #[test]
    fn gate_rejects_inactive_rows() {
        let root = temp_root("partitioned-rrf-timeline-gate-inactive");
        let path = root.join("timeline.jsonl");
        fs::write(
            &path,
            [row(0, Some(10), "active"), row(1, None, "inactive")].join("\n") + "\n",
        )
        .unwrap();
        let timeline = Timeline::load(&path, 2).unwrap();
        let err = enforce_gate(true, Some(&timeline), 2).unwrap_err();

        assert_eq!(err.code(), "CALYX_FSV_PARTITIONED_RRF_TIMELINE_INACTIVE");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn gate_accepts_all_active_rows() {
        let root = temp_root("partitioned-rrf-timeline-gate-active");
        let path = root.join("timeline.jsonl");
        fs::write(
            &path,
            [row(0, Some(10), "active"), row(1, Some(20), "active")].join("\n") + "\n",
        )
        .unwrap();
        let timeline = Timeline::load(&path, 2).unwrap();

        enforce_gate(true, Some(&timeline), 2).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    fn row(idx: usize, event_time: Option<i64>, state: &str) -> String {
        json!({
            "row_idx": idx,
            "id": format!("row-{idx}"),
            "source_event_time_secs": event_time,
            "source_event_time_raw": event_time.map(|value| value.to_string()),
            "temporal_lane_state": state,
            "temporal_inactive_reason": if event_time.is_some() { Value::Null } else { json!("source_missing_created_at") },
            "source_sequence": "jsonl_line",
            "source_sequence_index": idx,
            "query_row": idx == 0,
        })
        .to_string()
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
