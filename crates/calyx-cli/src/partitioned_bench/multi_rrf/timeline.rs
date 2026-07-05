use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, TEMPORAL_LANE_INACTIVE};
use serde_json::{Value, json};

use crate::error::{CliError, CliResult};
use crate::partitioned_bench::timeline_store::{
    self, TimelineDbReadback, TimelineManifestRecord, TimelineRowRecord,
};

#[derive(Clone, Debug)]
pub(super) struct Timeline {
    source: TimelineSource,
    rows: Vec<TimelineRowRecord>,
    active_order: Vec<usize>,
    duplicate_event_time_rows: usize,
    out_of_order_event_time_rows: usize,
}

#[derive(Clone, Debug)]
enum TimelineSource {
    File(PathBuf),
    GraphCf {
        cf_root: PathBuf,
        association_key: String,
        manifest: Box<TimelineManifestRecord>,
        readback: Box<TimelineDbReadback>,
    },
}

impl Timeline {
    pub(super) fn load(path: &Path, expected_rows: usize) -> CliResult<Self> {
        let import = timeline_store::load_rows_from_jsonl(path, Some(expected_rows))
            .map_err(CliError::from)?;
        Self::from_rows(TimelineSource::File(path.to_path_buf()), import.rows)
    }

    pub(super) fn load_from_db(
        cf_root: &Path,
        association_key: &str,
        expected_rows: usize,
    ) -> CliResult<Self> {
        let timeline_store::LoadedTimeline {
            manifest,
            rows,
            db_readback,
        } = timeline_store::read(cf_root, association_key).map_err(CliError::from)?;
        if rows.len() != expected_rows {
            return Err(timeline_error(
                "CALYX_FSV_PARTITIONED_RRF_TIMELINE_MISMATCH",
                format!(
                    "timeline rows={} expected corpus rows={expected_rows}",
                    rows.len()
                ),
                "write a timeline DB rowset that matches the plan corpus rows",
            ));
        }
        Self::from_rows(
            TimelineSource::GraphCf {
                cf_root: cf_root.to_path_buf(),
                association_key: association_key.to_string(),
                manifest: Box::new(manifest),
                readback: Box::new(db_readback),
            },
            rows,
        )
    }

    fn from_rows(source: TimelineSource, rows: Vec<TimelineRowRecord>) -> CliResult<Self> {
        let stats = timeline_store::stats(&rows).map_err(CliError::from)?;
        let mut active_order = (0..rows.len())
            .filter(|idx| rows[*idx].source_event_time_secs.is_some())
            .collect::<Vec<_>>();
        active_order.sort_by_key(|idx| (rows[*idx].source_event_time_secs.unwrap(), *idx));
        Ok(Self {
            source,
            rows,
            active_order,
            duplicate_event_time_rows: stats.duplicate_event_time_rows,
            out_of_order_event_time_rows: stats.out_of_order_event_time_rows,
        })
    }

    pub(super) fn report(&self) -> Value {
        let active_count = self.active_order.len();
        let mut report = json!({
            "mode": self.source.mode(),
            "counts_toward_a35": false,
            "row_count": self.rows.len(),
            "active_count": active_count,
            "inactive_count": self.rows.len().saturating_sub(active_count),
            "duplicate_event_time_rows": self.duplicate_event_time_rows,
            "out_of_order_event_time_rows": self.out_of_order_event_time_rows,
            "first_active": self.active_order.first().map(|idx| self.row_value(*idx)),
            "last_active": self.active_order.last().map(|idx| self.row_value(*idx)),
        });
        if let Some(object) = report.as_object_mut() {
            match &self.source {
                TimelineSource::File(path) => {
                    object.insert("timeline_path".to_string(), json!(path));
                    object.insert("diagnostic_only".to_string(), json!(true));
                }
                TimelineSource::GraphCf {
                    cf_root,
                    association_key,
                    manifest,
                    readback,
                } => {
                    object.insert("cf_root".to_string(), json!(cf_root));
                    object.insert("association_key".to_string(), json!(association_key));
                    object.insert("manifest".to_string(), json!(manifest));
                    object.insert("db_readback".to_string(), json!(readback));
                    object.insert("diagnostic_only".to_string(), json!(false));
                }
            }
        }
        report
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

    fn is_db_backed(&self) -> bool {
        matches!(self.source, TimelineSource::GraphCf { .. })
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
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_REQUIRED",
            "gate-bearing partitioned-rrf recall requires --timeline-cf-root",
            "write and read timeline rows through Calyx/Aster Graph CF; JSONL timelines are diagnostic only",
        ));
    };
    if !timeline.is_db_backed() {
        return Err(timeline_error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_REQUIRED",
            "gate-bearing partitioned-rrf recall cannot use a JSONL timeline as authority",
            "rerun with --timeline-cf-root and --timeline-key from stream-fbin timeline DB evidence",
        ));
    }
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

impl TimelineSource {
    fn mode(&self) -> &'static str {
        match self {
            TimelineSource::File(_) => "event_time_timeline_sidecar_diagnostic",
            TimelineSource::GraphCf { .. } => "event_time_timeline_aster_graph_cf",
        }
    }
}

fn row_value(row: &TimelineRowRecord) -> Value {
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
    use std::path::{Path, PathBuf};
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
        assert_eq!(report["mode"], "event_time_timeline_sidecar_diagnostic");
        assert_eq!(report["diagnostic_only"], true);
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

        assert_eq!(err.code(), "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_REQUIRED");
    }

    #[test]
    fn gate_rejects_file_timeline_for_recall_floor_truth() {
        let root = temp_root("partitioned-rrf-timeline-gate-file");
        let path = root.join("timeline.jsonl");
        fs::write(
            &path,
            [row(0, Some(10), "active"), row(1, Some(20), "active")].join("\n") + "\n",
        )
        .unwrap();
        let timeline = Timeline::load(&path, 2).unwrap();
        let err = enforce_gate(true, Some(&timeline), 2).unwrap_err();

        assert_eq!(err.code(), "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_REQUIRED");
        let _ = fs::remove_dir_all(root);
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
        let timeline = db_timeline_from_file(&root, &path, 2);
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
        let timeline = db_timeline_from_file(&root, &path, 2);
        let report = timeline.report();
        assert_eq!(report["mode"], "event_time_timeline_aster_graph_cf");
        assert_eq!(report["diagnostic_only"], false);
        assert!(report["db_readback"]["readback_matches"].as_bool().unwrap());

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

    fn db_timeline_from_file(root: &Path, path: &Path, rows: usize) -> Timeline {
        let import = timeline_store::load_rows_from_jsonl(path, Some(rows)).unwrap();
        let db = root.join("timeline-cf");
        timeline_store::write(
            &db,
            timeline_store::DEFAULT_ASSOCIATION_KEY,
            &import.source_sha256,
            &import.rows,
            2,
        )
        .unwrap();
        Timeline::load_from_db(&db, timeline_store::DEFAULT_ASSOCIATION_KEY, rows).unwrap()
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
