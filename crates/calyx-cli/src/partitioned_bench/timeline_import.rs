use std::path::PathBuf;

use crate::error::{CliError, CliResult};
use crate::partitioned_bench::timeline_store::{self, DEFAULT_ASSOCIATION_KEY, DEFAULT_CHUNK_ROWS};

pub(crate) fn run(raw: &[String]) -> CliResult {
    let args = Args::parse(raw)?;
    let import = timeline_store::load_rows_from_jsonl(&args.timeline, args.expected_rows)
        .map_err(CliError::Calyx)?;
    let stats = timeline_store::stats(&import.rows).map_err(CliError::Calyx)?;
    let readback = timeline_store::write(
        &args.cf_root,
        &args.association_key,
        &import.source_sha256,
        &import.rows,
        args.chunk_rows,
    )
    .map_err(CliError::Calyx)?;

    println!(
        "partitioned_rrf_timeline_db cf_root={} association_key={} row_count={} active_count={} duplicate_event_time_rows={} out_of_order_event_time_rows={} chunk_count={} manifest_value_bytes={} manifest_value_sha256={} chunk_value_bytes={} chunk_value_sha256={} source_sha256={} readback_matches={}",
        readback.cf_root,
        readback.association_key,
        readback.row_count,
        stats.active_count,
        stats.duplicate_event_time_rows,
        stats.out_of_order_event_time_rows,
        readback.chunk_count,
        readback.manifest_value_bytes,
        readback.manifest_value_sha256,
        readback.chunk_value_bytes,
        readback.chunk_value_sha256,
        import.source_sha256,
        readback.readback_matches
    );
    Ok(())
}

#[derive(Clone, Debug)]
struct Args {
    timeline: PathBuf,
    cf_root: PathBuf,
    association_key: String,
    expected_rows: Option<usize>,
    chunk_rows: usize,
}

impl Args {
    fn parse(raw: &[String]) -> CliResult<Self> {
        let mut timeline = None;
        let mut cf_root = None;
        let mut association_key = DEFAULT_ASSOCIATION_KEY.to_string();
        let mut expected_rows = None;
        let mut chunk_rows = DEFAULT_CHUNK_ROWS;
        let mut it = raw.iter();
        while let Some(flag) = it.next() {
            let mut next = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
            };
            match flag.as_str() {
                "--timeline" => timeline = Some(PathBuf::from(next()?)),
                "--cf-root" => cf_root = Some(PathBuf::from(next()?)),
                "--association-key" | "--timeline-key" => association_key = next()?,
                "--expected-rows" => {
                    expected_rows = Some(super::parse(&next()?, "--expected-rows")?)
                }
                "--chunk-rows" => chunk_rows = super::parse(&next()?, "--chunk-rows")?,
                other => return Err(CliError::usage(format!("unknown flag: {other}"))),
            }
        }
        if association_key.trim().is_empty() {
            return Err(CliError::usage("--timeline-key must be non-empty"));
        }
        if chunk_rows == 0 {
            return Err(CliError::usage("--chunk-rows must be > 0"));
        }
        Ok(Self {
            timeline: timeline.ok_or_else(|| CliError::usage("--timeline <jsonl> is required"))?,
            cf_root: cf_root.ok_or_else(|| CliError::usage("--cf-root <aster-dir> is required"))?,
            association_key,
            expected_rows,
            chunk_rows,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use calyx_core::TEMPORAL_LANE_ACTIVE;

    use super::*;
    use crate::partitioned_bench::timeline_store::TimelineRowRecord;

    #[test]
    fn parses_db_timeline_import_args() {
        let args = Args::parse(&strings([
            "--timeline",
            "timeline.jsonl",
            "--cf-root",
            "timeline-cf",
            "--timeline-key",
            "issue791_timeline",
            "--expected-rows",
            "50",
            "--chunk-rows",
            "7",
        ]))
        .unwrap();

        assert_eq!(args.timeline, PathBuf::from("timeline.jsonl"));
        assert_eq!(args.cf_root, PathBuf::from("timeline-cf"));
        assert_eq!(args.association_key, "issue791_timeline");
        assert_eq!(args.expected_rows, Some(50));
        assert_eq!(args.chunk_rows, 7);
    }

    #[test]
    fn run_import_writes_graph_cf_timeline() {
        let root = temp_root("partitioned-rrf-timeline-import");
        let timeline = root.join("timeline.jsonl");
        fs::write(&timeline, jsonl_rows(3)).unwrap();
        let cf_root = root.join("timeline-cf");

        run(&strings([
            "--timeline".into(),
            timeline.display().to_string(),
            "--cf-root".into(),
            cf_root.display().to_string(),
            "--timeline-key".into(),
            "unit_timeline".into(),
            "--expected-rows".into(),
            "3".into(),
            "--chunk-rows".into(),
            "2".into(),
        ]))
        .unwrap();

        let loaded = timeline_store::read(&cf_root, "unit_timeline").unwrap();
        assert_eq!(loaded.rows.len(), 3);
        assert_eq!(loaded.manifest.chunk_count, 2);
        assert!(loaded.db_readback.readback_matches);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn run_import_refuses_duplicate_key() {
        let root = temp_root("partitioned-rrf-timeline-import-duplicate");
        let timeline = root.join("timeline.jsonl");
        fs::write(&timeline, jsonl_rows(2)).unwrap();
        let cf_root = root.join("timeline-cf");
        let args = strings([
            "--timeline".into(),
            timeline.display().to_string(),
            "--cf-root".into(),
            cf_root.display().to_string(),
            "--timeline-key".into(),
            "unit_timeline".into(),
            "--expected-rows".into(),
            "2".into(),
        ]);

        run(&args).unwrap();
        let err = run(&args).unwrap_err();

        assert_eq!(err.code(), "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_EXISTS");
        let _ = fs::remove_dir_all(root);
    }

    fn jsonl_rows(count: usize) -> String {
        rows(count)
            .into_iter()
            .map(|row| serde_json::to_string(&row).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
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

    fn strings<I, S>(items: I) -> Vec<String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        items.into_iter().map(Into::into).collect()
    }
}
