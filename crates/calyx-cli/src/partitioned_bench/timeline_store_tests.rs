use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::TEMPORAL_LANE_ACTIVE;

use super::*;

#[test]
fn graph_cf_timeline_round_trips_chunked_bytes() {
    let root = temp_root("partitioned-rrf-timeline-db");
    let rows = rows(5);

    let written = write(&root, "unit_timeline", &"00".repeat(32), &rows, 2).unwrap();
    let loaded = read(&root, "unit_timeline").unwrap();

    assert!(written.readback_matches);
    assert_eq!(
        written.manifest_value_sha256,
        loaded.db_readback.manifest_value_sha256
    );
    assert_eq!(loaded.manifest.chunk_count, 3);
    assert_eq!(loaded.rows, rows);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn graph_cf_timeline_refuses_duplicate_key() {
    let root = temp_root("partitioned-rrf-timeline-db-duplicate");
    let rows = rows(2);

    write(&root, "unit_timeline", &"00".repeat(32), &rows, 2).unwrap();
    let err = write(&root, "unit_timeline", &"00".repeat(32), &rows, 2).unwrap_err();

    assert_eq!(err.code, "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_EXISTS");
    let _ = fs::remove_dir_all(root);
}

fn rows(count: usize) -> Vec<TimelineRowRecord> {
    (0..count)
        .map(|idx| TimelineRowRecord {
            row_idx: idx,
            id: format!("row-{idx}"),
            source_event_time_secs: Some(1_704_153_600 + idx as i64),
            source_event_time_raw: Some((1_704_153_600 + idx as i64).to_string()),
            temporal_lane_state: TEMPORAL_LANE_ACTIVE.to_string(),
            temporal_inactive_reason: None,
            source_sequence: "jsonl_line".to_string(),
            source_sequence_index: Some(idx),
            query_row: idx == 0,
        })
        .collect()
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
