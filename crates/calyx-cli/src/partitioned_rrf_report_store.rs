use std::path::Path;

use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const KEY_PREFIX: &[u8] = b"calyx/partitioned-rrf/report/v1/";
const VALUE_MAGIC: &[u8] = b"CRRFRP1\0";
const CF_MEMTABLE_CAP: usize = 8 * 1024 * 1024;

pub(crate) const DEFAULT_ASSOCIATION_KEY: &str = "partitioned_rrf_report";
pub(crate) const FORMAT: &str = "calyx-partitioned-rrf-report-v1";
pub(crate) const MODE: &str = "real_multi_slot_rrf_report";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct PartitionedRrfReportRecord {
    pub(crate) format: String,
    pub(crate) mode: String,
    pub(crate) report: Value,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PartitionedRrfReportDbReadback {
    pub(crate) cf_root: String,
    pub(crate) association_key: String,
    pub(crate) row_key_sha256: String,
    pub(crate) value_bytes: usize,
    pub(crate) value_sha256: String,
    pub(crate) readback_matches: bool,
}

pub(crate) fn write(
    cf_root: &Path,
    association_key: &str,
    report: &Value,
) -> Result<PartitionedRrfReportDbReadback> {
    let row_key = row_key(association_key)?;
    let record = PartitionedRrfReportRecord {
        format: FORMAT.to_string(),
        mode: MODE.to_string(),
        report: report.clone(),
    };
    let value = encode(&record)?;
    let mut router = CfRouter::open(cf_root, CF_MEMTABLE_CAP)?;
    if router.get(ColumnFamily::Graph, &row_key)?.is_some() {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_REPORT_DB_EXISTS",
            "partitioned RRF report row already exists in Graph CF",
        ));
    }
    router.put(ColumnFamily::Graph, &row_key, &value)?;
    router.flush_cf(ColumnFamily::Graph)?;
    drop(router);

    let reopened = CfRouter::open(cf_root, CF_MEMTABLE_CAP)?;
    let readback = reopened
        .get(ColumnFamily::Graph, &row_key)?
        .ok_or_else(|| {
            error(
                "CALYX_FSV_PARTITIONED_RRF_REPORT_DB_MISSING",
                "partitioned RRF report row missing after Graph CF write",
            )
        })?;
    if readback != value {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_REPORT_DB_MISMATCH",
            "partitioned RRF report Graph CF readback bytes changed after write",
        ));
    }
    let decoded: PartitionedRrfReportRecord = decode(&readback)?;
    if decoded.format != FORMAT || decoded.mode != MODE {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_REPORT_DB_INVALID",
            "partitioned RRF report row decoded to the wrong format or mode",
        ));
    }
    Ok(readback_report(
        cf_root,
        association_key,
        &row_key,
        &readback,
        true,
    ))
}

#[cfg(test)]
pub(crate) fn read(
    cf_root: &Path,
    association_key: &str,
) -> Result<(PartitionedRrfReportRecord, PartitionedRrfReportDbReadback)> {
    let row_key = row_key(association_key)?;
    let router = CfRouter::open(cf_root, CF_MEMTABLE_CAP)?;
    let value = router.get(ColumnFamily::Graph, &row_key)?.ok_or_else(|| {
        error(
            "CALYX_FSV_PARTITIONED_RRF_REPORT_DB_MISSING",
            "partitioned RRF report row missing in Graph CF",
        )
    })?;
    let record = decode(&value)?;
    Ok((
        record,
        readback_report(cf_root, association_key, &row_key, &value, true),
    ))
}

fn row_key(association_key: &str) -> Result<Vec<u8>> {
    if association_key.trim().is_empty() {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_REPORT_DB_INVALID_KEY",
            "partitioned RRF report association key must be non-empty",
        ));
    }
    let mut key = Vec::with_capacity(KEY_PREFIX.len() + association_key.len());
    key.extend_from_slice(KEY_PREFIX);
    key.extend_from_slice(association_key.as_bytes());
    Ok(key)
}

fn encode<T: Serialize>(record: &T) -> Result<Vec<u8>> {
    let mut bytes = VALUE_MAGIC.to_vec();
    let mut payload = Vec::new();
    ciborium::ser::into_writer(record, &mut payload).map_err(|err| {
        error(
            "CALYX_FSV_PARTITIONED_RRF_REPORT_DB_ENCODE",
            format!("encode partitioned RRF report record failed: {err}"),
        )
    })?;
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T> {
    let payload = bytes.strip_prefix(VALUE_MAGIC).ok_or_else(|| {
        error(
            "CALYX_FSV_PARTITIONED_RRF_REPORT_DB_INVALID",
            "partitioned RRF report row has invalid magic",
        )
    })?;
    ciborium::de::from_reader(payload).map_err(|err| {
        error(
            "CALYX_FSV_PARTITIONED_RRF_REPORT_DB_DECODE",
            format!("decode partitioned RRF report record failed: {err}"),
        )
    })
}

fn readback_report(
    cf_root: &Path,
    association_key: &str,
    row_key: &[u8],
    value: &[u8],
    readback_matches: bool,
) -> PartitionedRrfReportDbReadback {
    PartitionedRrfReportDbReadback {
        cf_root: cf_root.display().to_string(),
        association_key: association_key.to_string(),
        row_key_sha256: hex_sha256(row_key),
        value_bytes: value.len(),
        value_sha256: hex_sha256(value),
        readback_matches,
    }
}

fn error(code: &'static str, message: impl Into<String>) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation: "write and read partitioned RRF reports through Calyx/Aster Graph CF",
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::*;

    #[test]
    fn graph_cf_partitioned_rrf_report_round_trips_bytes() {
        let root = temp_root("partitioned-rrf-report-db");
        let report = json!({
            "trigger": "calyx bench partitioned-rrf",
            "mode": "real_multi_slot_rrf",
            "fused_ground_truth_recall_at_k": 0.88,
        });

        let written = write(&root, "unit_report", &report).unwrap();
        let (record, readback) = read(&root, "unit_report").unwrap();

        assert!(written.readback_matches);
        assert_eq!(written.value_sha256, readback.value_sha256);
        assert_eq!(record.format, FORMAT);
        assert_eq!(record.mode, MODE);
        assert_eq!(record.report["fused_ground_truth_recall_at_k"], 0.88);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn graph_cf_partitioned_rrf_report_refuses_duplicate_key() {
        let root = temp_root("partitioned-rrf-report-db-duplicate");
        let report = json!({"mode": "real_multi_slot_rrf"});

        write(&root, "unit_report", &report).unwrap();
        let err = write(&root, "unit_report", &report).unwrap_err();

        assert_eq!(err.code, "CALYX_FSV_PARTITIONED_RRF_REPORT_DB_EXISTS");
        let _ = fs::remove_dir_all(root);
    }

    fn temp_root(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
