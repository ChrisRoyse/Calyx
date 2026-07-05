use std::collections::BTreeSet;
use std::path::Path;

use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{Result, TEMPORAL_LANE_ACTIVE, TEMPORAL_LANE_INACTIVE};
use serde::{Deserialize, Serialize};

#[path = "timeline_store/codec.rs"]
mod codec;

use codec::{chunk_key, chunk_values_sha256, decode, encode, error, hex_sha256, manifest_key};

const CF_MEMTABLE_CAP: usize = 8 * 1024 * 1024;

pub(crate) const DEFAULT_ASSOCIATION_KEY: &str = "partitioned_rrf_timeline";
pub(crate) const DEFAULT_CHUNK_ROWS: usize = 8192;
pub(crate) const FORMAT: &str = "calyx-partitioned-rrf-timeline-v1";
pub(crate) const MODE: &str = "partitioned_rrf_timeline";
pub(crate) const ROW_ID_SPACE: &str = "partitioned_rrf_plan_corpus_row_idx";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct TimelineRowRecord {
    pub(crate) row_idx: usize,
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) source_event_time_secs: Option<i64>,
    #[serde(default)]
    pub(crate) source_event_time_raw: Option<String>,
    pub(crate) temporal_lane_state: String,
    #[serde(default)]
    pub(crate) temporal_inactive_reason: Option<String>,
    pub(crate) source_sequence: String,
    #[serde(default)]
    pub(crate) source_sequence_index: Option<usize>,
    #[serde(default)]
    pub(crate) query_row: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct TimelineManifestRecord {
    pub(crate) format: String,
    pub(crate) mode: String,
    pub(crate) row_id_space: String,
    pub(crate) imported_timeline_sha256: String,
    pub(crate) row_count: usize,
    pub(crate) active_count: usize,
    pub(crate) inactive_count: usize,
    pub(crate) duplicate_event_time_rows: usize,
    pub(crate) out_of_order_event_time_rows: usize,
    pub(crate) chunk_rows: usize,
    pub(crate) chunk_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct TimelineChunkRecord {
    pub(crate) format: String,
    pub(crate) mode: String,
    pub(crate) association_key: String,
    pub(crate) chunk_index: usize,
    pub(crate) first_row_idx: usize,
    pub(crate) rows: Vec<TimelineRowRecord>,
}

#[derive(Clone, Debug)]
pub(crate) struct TimelineFileImport {
    pub(crate) rows: Vec<TimelineRowRecord>,
    pub(crate) source_sha256: String,
}

#[derive(Clone, Debug)]
pub(crate) struct LoadedTimeline {
    pub(crate) manifest: TimelineManifestRecord,
    pub(crate) rows: Vec<TimelineRowRecord>,
    pub(crate) db_readback: TimelineDbReadback,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct TimelineDbReadback {
    pub(crate) cf_root: String,
    pub(crate) association_key: String,
    pub(crate) manifest_key_sha256: String,
    pub(crate) manifest_value_bytes: usize,
    pub(crate) manifest_value_sha256: String,
    pub(crate) chunk_count: usize,
    pub(crate) chunk_value_bytes: usize,
    pub(crate) chunk_value_sha256: String,
    pub(crate) row_count: usize,
    pub(crate) readback_matches: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TimelineStats {
    pub(crate) active_count: usize,
    pub(crate) duplicate_event_time_rows: usize,
    pub(crate) out_of_order_event_time_rows: usize,
}

pub(crate) fn load_rows_from_jsonl(
    path: &Path,
    expected_rows: Option<usize>,
) -> Result<TimelineFileImport> {
    let bytes = std::fs::read(path).map_err(|err| {
        error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_IMPORT_IO",
            format!("read {} failed: {err}", path.display()),
        )
    })?;
    let text = std::str::from_utf8(&bytes).map_err(|err| {
        error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_IMPORT_INVALID",
            format!("{} is not utf8: {err}", path.display()),
        )
    })?;
    let mut rows = Vec::new();
    for (line_idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: TimelineRowRecord = serde_json::from_str(line).map_err(|err| {
            error(
                "CALYX_FSV_PARTITIONED_RRF_TIMELINE_IMPORT_INVALID",
                format!("{} line {line_idx}: {err}", path.display()),
            )
        })?;
        rows.push(row);
    }
    if let Some(expected) = expected_rows
        && rows.len() != expected
    {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_IMPORT_MISMATCH",
            format!(
                "timeline rows={} expected corpus rows={expected}",
                rows.len()
            ),
        ));
    }
    validate_rows(&rows)?;
    Ok(TimelineFileImport {
        rows,
        source_sha256: hex_sha256(&bytes),
    })
}

pub(crate) fn write(
    cf_root: &Path,
    association_key: &str,
    imported_timeline_sha256: &str,
    rows: &[TimelineRowRecord],
    chunk_rows: usize,
) -> Result<TimelineDbReadback> {
    let manifest_key = manifest_key(association_key)?;
    let chunk_rows = validate_chunk_rows(chunk_rows)?;
    let stats = validate_rows(rows)?;
    let chunk_count = rows.len().div_ceil(chunk_rows);
    let manifest = TimelineManifestRecord {
        format: FORMAT.to_string(),
        mode: MODE.to_string(),
        row_id_space: ROW_ID_SPACE.to_string(),
        imported_timeline_sha256: imported_timeline_sha256.to_string(),
        row_count: rows.len(),
        active_count: stats.active_count,
        inactive_count: rows.len().saturating_sub(stats.active_count),
        duplicate_event_time_rows: stats.duplicate_event_time_rows,
        out_of_order_event_time_rows: stats.out_of_order_event_time_rows,
        chunk_rows,
        chunk_count,
    };
    let manifest_value = encode(&manifest)?;
    let chunks = encode_chunks(association_key, rows, chunk_rows)?;
    let mut router = CfRouter::open(cf_root, CF_MEMTABLE_CAP)?;
    fail_if_exists(&router, &manifest_key)?;
    for (chunk_key, _) in &chunks {
        fail_if_exists(&router, chunk_key)?;
    }
    for (chunk_key, value) in &chunks {
        router.put(ColumnFamily::Graph, chunk_key, value)?;
    }
    router.put(ColumnFamily::Graph, &manifest_key, &manifest_value)?;
    router.flush_cf(ColumnFamily::Graph)?;
    drop(router);

    let reopened = CfRouter::open(cf_root, CF_MEMTABLE_CAP)?;
    let manifest_readback = read_exact(&reopened, &manifest_key, "MISSING_AFTER_WRITE")?;
    if manifest_readback != manifest_value {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_MISMATCH",
            "timeline manifest Graph CF readback bytes changed after write",
        ));
    }
    let mut chunk_values = Vec::with_capacity(chunks.len());
    for (chunk_key, expected) in &chunks {
        let readback = read_exact(&reopened, chunk_key, "CHUNK_MISSING_AFTER_WRITE")?;
        if readback != *expected {
            return Err(error(
                "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_MISMATCH",
                "timeline chunk Graph CF readback bytes changed after write",
            ));
        }
        chunk_values.push(readback);
    }
    Ok(readback_report(
        cf_root,
        association_key,
        &manifest_key,
        &manifest_readback,
        &chunk_values,
        rows.len(),
        true,
    ))
}

pub(crate) fn read(cf_root: &Path, association_key: &str) -> Result<LoadedTimeline> {
    let manifest_key = manifest_key(association_key)?;
    let router = CfRouter::open(cf_root, CF_MEMTABLE_CAP)?;
    let manifest_value = read_exact(&router, &manifest_key, "MISSING")?;
    let manifest: TimelineManifestRecord = decode(&manifest_value)?;
    validate_manifest(&manifest)?;
    let mut rows = Vec::with_capacity(manifest.row_count);
    let mut chunk_values = Vec::with_capacity(manifest.chunk_count);
    for chunk_index in 0..manifest.chunk_count {
        let chunk_key = chunk_key(association_key, chunk_index)?;
        let value = read_exact(&router, &chunk_key, "CHUNK_MISSING")?;
        let chunk: TimelineChunkRecord = decode(&value)?;
        validate_chunk(&chunk, association_key, chunk_index, rows.len())?;
        rows.extend(chunk.rows);
        chunk_values.push(value);
    }
    if rows.len() != manifest.row_count {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_INVALID",
            format!(
                "timeline DB rows={} manifest row_count={}",
                rows.len(),
                manifest.row_count
            ),
        ));
    }
    let stats = validate_rows(&rows)?;
    if stats.active_count != manifest.active_count
        || rows.len().saturating_sub(stats.active_count) != manifest.inactive_count
        || stats.duplicate_event_time_rows != manifest.duplicate_event_time_rows
        || stats.out_of_order_event_time_rows != manifest.out_of_order_event_time_rows
    {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_INVALID",
            "timeline manifest stats do not match chunk rows",
        ));
    }
    let db_readback = readback_report(
        cf_root,
        association_key,
        &manifest_key,
        &manifest_value,
        &chunk_values,
        rows.len(),
        true,
    );
    Ok(LoadedTimeline {
        manifest,
        rows,
        db_readback,
    })
}

pub(crate) fn stats(rows: &[TimelineRowRecord]) -> Result<TimelineStats> {
    validate_rows(rows)
}

fn validate_rows(rows: &[TimelineRowRecord]) -> Result<TimelineStats> {
    let mut row_ids = BTreeSet::new();
    let mut seen_times = BTreeSet::new();
    let mut duplicates = 0usize;
    let mut out_of_order = 0usize;
    let mut previous_time = None;
    for (line_idx, row) in rows.iter().enumerate() {
        if row.row_idx != line_idx {
            return Err(error(
                "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_INVALID",
                format!("row_idx={} expected sequence index={line_idx}", row.row_idx),
            ));
        }
        if !row_ids.insert(row.row_idx) {
            return Err(error(
                "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_INVALID",
                format!("duplicate row_idx {}", row.row_idx),
            ));
        }
        validate_lane(line_idx, row)?;
        if let Some(secs) = row.source_event_time_secs {
            if !seen_times.insert(secs) {
                duplicates += 1;
            }
            if previous_time.is_some_and(|prev| secs < prev) {
                out_of_order += 1;
            }
            previous_time = Some(secs);
        }
    }
    Ok(TimelineStats {
        active_count: rows
            .iter()
            .filter(|row| row.source_event_time_secs.is_some())
            .count(),
        duplicate_event_time_rows: duplicates,
        out_of_order_event_time_rows: out_of_order,
    })
}

fn validate_lane(line_idx: usize, row: &TimelineRowRecord) -> Result<()> {
    if row.id.trim().is_empty() || row.source_sequence.trim().is_empty() {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_INVALID",
            format!("line {line_idx} has empty id or source_sequence"),
        ));
    }
    match row.temporal_lane_state.as_str() {
        TEMPORAL_LANE_ACTIVE if row.source_event_time_secs.is_none() => Err(error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_INVALID",
            format!("line {line_idx} is active but missing source_event_time_secs"),
        )),
        TEMPORAL_LANE_INACTIVE if row.source_event_time_secs.is_some() => Err(error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_INVALID",
            format!("line {line_idx} is inactive but carries source_event_time_secs"),
        )),
        TEMPORAL_LANE_ACTIVE | TEMPORAL_LANE_INACTIVE => Ok(()),
        other => Err(error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_INVALID",
            format!("line {line_idx} has unknown temporal_lane_state {other:?}"),
        )),
    }
}

fn validate_manifest(manifest: &TimelineManifestRecord) -> Result<()> {
    if manifest.format != FORMAT || manifest.mode != MODE || manifest.row_id_space != ROW_ID_SPACE {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_INVALID",
            "timeline manifest decoded to the wrong format, mode, or row id space",
        ));
    }
    validate_chunk_rows(manifest.chunk_rows)?;
    if manifest.chunk_count != manifest.row_count.div_ceil(manifest.chunk_rows) {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_INVALID",
            "timeline manifest chunk_count does not match row_count/chunk_rows",
        ));
    }
    Ok(())
}

fn validate_chunk(
    chunk: &TimelineChunkRecord,
    association_key: &str,
    chunk_index: usize,
    expected_first_row: usize,
) -> Result<()> {
    if chunk.format != FORMAT
        || chunk.mode != MODE
        || chunk.association_key != association_key
        || chunk.chunk_index != chunk_index
        || chunk.first_row_idx != expected_first_row
    {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_INVALID",
            "timeline chunk decoded to the wrong format, key, or row offset",
        ));
    }
    Ok(())
}

fn encode_chunks(
    association_key: &str,
    rows: &[TimelineRowRecord],
    chunk_rows: usize,
) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    rows.chunks(chunk_rows)
        .enumerate()
        .map(|(chunk_index, rows)| {
            let first_row_idx = rows.first().map_or(0, |row| row.row_idx);
            let record = TimelineChunkRecord {
                format: FORMAT.to_string(),
                mode: MODE.to_string(),
                association_key: association_key.to_string(),
                chunk_index,
                first_row_idx,
                rows: rows.to_vec(),
            };
            Ok((chunk_key(association_key, chunk_index)?, encode(&record)?))
        })
        .collect()
}

fn validate_chunk_rows(chunk_rows: usize) -> Result<usize> {
    if chunk_rows == 0 {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_INVALID",
            "timeline chunk_rows must be > 0",
        ));
    }
    Ok(chunk_rows)
}

fn fail_if_exists(router: &CfRouter, row_key: &[u8]) -> Result<()> {
    if router.get(ColumnFamily::Graph, row_key)?.is_some() {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_EXISTS",
            "timeline row already exists in Graph CF",
        ));
    }
    Ok(())
}

fn read_exact(router: &CfRouter, row_key: &[u8], missing_suffix: &str) -> Result<Vec<u8>> {
    router.get(ColumnFamily::Graph, row_key)?.ok_or_else(|| {
        error(
            match missing_suffix {
                "MISSING" => "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_MISSING",
                "MISSING_AFTER_WRITE" => {
                    "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_MISSING_AFTER_WRITE"
                }
                "CHUNK_MISSING" => "CALYX_FSV_PARTITIONED_RRF_TIMELINE_CHUNK_DB_MISSING",
                "CHUNK_MISSING_AFTER_WRITE" => {
                    "CALYX_FSV_PARTITIONED_RRF_TIMELINE_CHUNK_DB_MISSING_AFTER_WRITE"
                }
                _ => "CALYX_FSV_PARTITIONED_RRF_TIMELINE_DB_MISSING",
            },
            "timeline row missing in Graph CF",
        )
    })
}

fn readback_report(
    cf_root: &Path,
    association_key: &str,
    manifest_key: &[u8],
    manifest_value: &[u8],
    chunk_values: &[Vec<u8>],
    row_count: usize,
    readback_matches: bool,
) -> TimelineDbReadback {
    let chunk_value_bytes = chunk_values.iter().map(Vec::len).sum();
    TimelineDbReadback {
        cf_root: cf_root.display().to_string(),
        association_key: association_key.to_string(),
        manifest_key_sha256: hex_sha256(manifest_key),
        manifest_value_bytes: manifest_value.len(),
        manifest_value_sha256: hex_sha256(manifest_value),
        chunk_count: chunk_values.len(),
        chunk_value_bytes,
        chunk_value_sha256: chunk_values_sha256(chunk_values),
        row_count,
        readback_matches,
    }
}

#[cfg(test)]
#[path = "timeline_store_tests.rs"]
mod tests;
