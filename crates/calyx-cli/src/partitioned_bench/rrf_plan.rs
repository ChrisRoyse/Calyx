use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use bincode::config;
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{CliError, CliResult};

const KEY_PREFIX: &[u8] = b"calyx/partitioned-rrf/plan/v1/";
const VALUE_MAGIC: &[u8] = b"CRRFPL1\0";
const CF_MEMTABLE_CAP: usize = 8 * 1024 * 1024;

pub(crate) const DEFAULT_ASSOCIATION_KEY: &str = "partitioned_rrf_plan";
pub(crate) const FORMAT: &str = "calyx-partitioned-rrf-plan-v1";
pub(crate) const MODE: &str = "partitioned_rrf_plan";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Plan {
    #[serde(default)]
    pub(crate) timeline: Option<PathBuf>,
    pub(crate) slots: Vec<PlanSlot>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct PlanSlot {
    pub(crate) slot: u16,
    pub(crate) name: Option<String>,
    pub(crate) lens_id: Option<String>,
    pub(crate) weights_sha256: Option<String>,
    pub(crate) signal_kind: Option<String>,
    pub(crate) bits_about: Option<f32>,
    pub(crate) vault: PathBuf,
    pub(crate) queries: PathBuf,
    pub(crate) corpus: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct PartitionedRrfPlanRecord {
    pub(crate) format: String,
    pub(crate) mode: String,
    pub(crate) imported_plan_sha256: String,
    pub(crate) base_dir: PathBuf,
    pub(crate) plan: Plan,
}

#[derive(Clone, Debug)]
pub(crate) struct LoadedPlan {
    pub(crate) plan: Plan,
    pub(crate) plan_sha256: String,
    pub(crate) base_dir: PathBuf,
    pub(crate) db_readback: Option<PartitionedRrfPlanDbReadback>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PartitionedRrfPlanDbReadback {
    pub(crate) cf_root: String,
    pub(crate) association_key: String,
    pub(crate) row_key_sha256: String,
    pub(crate) value_bytes: usize,
    pub(crate) value_sha256: String,
    pub(crate) readback_matches: bool,
}

pub(crate) fn run_import(raw: &[String]) -> CliResult {
    let args = ImportArgs::parse(raw)?;
    let loaded = load_from_file(&args.plan)?;
    let readback = write(
        &args.cf_root,
        &args.association_key,
        &PartitionedRrfPlanRecord {
            format: FORMAT.to_string(),
            mode: MODE.to_string(),
            imported_plan_sha256: loaded.plan_sha256,
            base_dir: loaded.base_dir,
            plan: loaded.plan,
        },
    )
    .map_err(CliError::Calyx)?;
    println!(
        "partitioned_rrf_plan_db cf_root={} association_key={} value_bytes={} value_sha256={} readback_matches={}",
        readback.cf_root,
        readback.association_key,
        readback.value_bytes,
        readback.value_sha256,
        readback.readback_matches
    );
    Ok(())
}

pub(crate) fn load_from_file(path: &Path) -> CliResult<LoadedPlan> {
    let bytes = std::fs::read(path)?;
    let plan: Plan = serde_json::from_slice(&bytes).map_err(|error| {
        CliError::runtime(format!("parse rrf plan {}: {error}", path.display()))
    })?;
    validate_unique_slots(&plan)?;
    Ok(LoadedPlan {
        plan,
        plan_sha256: hex_sha256(&bytes),
        base_dir: path.parent().unwrap_or_else(|| Path::new("")).to_path_buf(),
        db_readback: None,
    })
}

pub(crate) fn load_from_db(cf_root: &Path, association_key: &str) -> CliResult<LoadedPlan> {
    let (record, readback) = read(cf_root, association_key).map_err(CliError::Calyx)?;
    if record.format != FORMAT || record.mode != MODE {
        return Err(CliError::Calyx(error(
            "CALYX_FSV_PARTITIONED_RRF_PLAN_DB_INVALID",
            "partitioned RRF plan row decoded to the wrong format or mode",
        )));
    }
    validate_unique_slots(&record.plan)?;
    Ok(LoadedPlan {
        plan: record.plan,
        plan_sha256: record.imported_plan_sha256,
        base_dir: record.base_dir,
        db_readback: Some(readback),
    })
}

pub(crate) fn resolve(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

pub(crate) fn write(
    cf_root: &Path,
    association_key: &str,
    record: &PartitionedRrfPlanRecord,
) -> Result<PartitionedRrfPlanDbReadback> {
    let row_key = row_key(association_key)?;
    let value = encode(record)?;
    let mut router = CfRouter::open(cf_root, CF_MEMTABLE_CAP)?;
    if router.get(ColumnFamily::Graph, &row_key)?.is_some() {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_PLAN_DB_EXISTS",
            "partitioned RRF plan row already exists in Graph CF",
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
                "CALYX_FSV_PARTITIONED_RRF_PLAN_DB_MISSING",
                "partitioned RRF plan row missing after Graph CF write",
            )
        })?;
    if readback != value {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_PLAN_DB_MISMATCH",
            "partitioned RRF plan Graph CF readback bytes changed after write",
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

pub(crate) fn read(
    cf_root: &Path,
    association_key: &str,
) -> Result<(PartitionedRrfPlanRecord, PartitionedRrfPlanDbReadback)> {
    let row_key = row_key(association_key)?;
    let router = CfRouter::open(cf_root, CF_MEMTABLE_CAP)?;
    let value = router.get(ColumnFamily::Graph, &row_key)?.ok_or_else(|| {
        error(
            "CALYX_FSV_PARTITIONED_RRF_PLAN_DB_MISSING",
            "partitioned RRF plan row missing in Graph CF",
        )
    })?;
    let record = decode(&value)?;
    Ok((
        record,
        readback_report(cf_root, association_key, &row_key, &value, true),
    ))
}

fn validate_unique_slots(plan: &Plan) -> CliResult {
    let mut seen = BTreeSet::new();
    for slot in &plan.slots {
        if !seen.insert(slot.slot) {
            return Err(CliError::usage(format!(
                "partitioned-rrf plan has duplicate slot {}",
                slot.slot
            )));
        }
    }
    Ok(())
}

fn row_key(association_key: &str) -> Result<Vec<u8>> {
    if association_key.trim().is_empty() {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_PLAN_DB_INVALID_KEY",
            "partitioned RRF plan association key must be non-empty",
        ));
    }
    let mut key = Vec::with_capacity(KEY_PREFIX.len() + association_key.len());
    key.extend_from_slice(KEY_PREFIX);
    key.extend_from_slice(association_key.as_bytes());
    Ok(key)
}

fn encode<T: Serialize>(record: &T) -> Result<Vec<u8>> {
    let mut bytes = VALUE_MAGIC.to_vec();
    let payload = bincode::serde::encode_to_vec(record, config::standard()).map_err(|err| {
        error(
            "CALYX_FSV_PARTITIONED_RRF_PLAN_DB_ENCODE",
            format!("encode partitioned RRF plan record failed: {err}"),
        )
    })?;
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T> {
    let payload = bytes.strip_prefix(VALUE_MAGIC).ok_or_else(|| {
        error(
            "CALYX_FSV_PARTITIONED_RRF_PLAN_DB_INVALID",
            "partitioned RRF plan row has invalid magic",
        )
    })?;
    let (record, consumed): (T, usize) =
        bincode::serde::decode_from_slice(payload, config::standard()).map_err(|err| {
            error(
                "CALYX_FSV_PARTITIONED_RRF_PLAN_DB_DECODE",
                format!("decode partitioned RRF plan record failed: {err}"),
            )
        })?;
    if consumed != payload.len() {
        return Err(error(
            "CALYX_FSV_PARTITIONED_RRF_PLAN_DB_INVALID",
            "partitioned RRF plan row has trailing bytes",
        ));
    }
    Ok(record)
}

fn readback_report(
    cf_root: &Path,
    association_key: &str,
    row_key: &[u8],
    value: &[u8],
    readback_matches: bool,
) -> PartitionedRrfPlanDbReadback {
    PartitionedRrfPlanDbReadback {
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
        remediation: "write and read partitioned RRF plans through Calyx/Aster Graph CF",
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

struct ImportArgs {
    plan: PathBuf,
    cf_root: PathBuf,
    association_key: String,
}

impl ImportArgs {
    fn parse(raw: &[String]) -> CliResult<Self> {
        let mut plan = None;
        let mut cf_root = None;
        let mut association_key = DEFAULT_ASSOCIATION_KEY.to_string();
        let mut it = raw.iter();
        while let Some(flag) = it.next() {
            let mut next = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
            };
            match flag.as_str() {
                "--plan" => plan = Some(PathBuf::from(next()?)),
                "--cf-root" => cf_root = Some(PathBuf::from(next()?)),
                "--association-key" | "--plan-key" => association_key = next()?,
                other => return Err(CliError::usage(format!("unknown flag: {other}"))),
            }
        }
        if association_key.trim().is_empty() {
            return Err(CliError::usage("--association-key must be non-empty"));
        }
        Ok(Self {
            plan: plan.ok_or_else(|| CliError::usage("--plan <json> is required"))?,
            cf_root: cf_root.ok_or_else(|| CliError::usage("--cf-root <dir> is required"))?,
            association_key,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn graph_cf_partitioned_rrf_plan_round_trips_bytes() {
        let root = temp_root("partitioned-rrf-plan-db");
        let record = record();

        let written = write(&root, "unit_plan", &record).unwrap();
        let (read_record, readback) = read(&root, "unit_plan").unwrap();

        assert!(written.readback_matches);
        assert_eq!(written.value_sha256, readback.value_sha256);
        assert_eq!(read_record.format, FORMAT);
        assert_eq!(read_record.plan.slots[0].slot, 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn graph_cf_partitioned_rrf_plan_refuses_duplicate_key() {
        let root = temp_root("partitioned-rrf-plan-db-duplicate");
        let record = record();

        write(&root, "unit_plan", &record).unwrap();
        let err = write(&root, "unit_plan", &record).unwrap_err();

        assert_eq!(err.code, "CALYX_FSV_PARTITIONED_RRF_PLAN_DB_EXISTS");
        let _ = fs::remove_dir_all(root);
    }

    fn record() -> PartitionedRrfPlanRecord {
        PartitionedRrfPlanRecord {
            format: FORMAT.to_string(),
            mode: MODE.to_string(),
            imported_plan_sha256: "00".repeat(32),
            base_dir: PathBuf::from("/tmp/calyx-plan"),
            plan: Plan {
                timeline: Some(PathBuf::from("timeline.jsonl")),
                slots: vec![PlanSlot {
                    slot: 0,
                    name: Some("unit".to_string()),
                    lens_id: Some("11".repeat(16)),
                    weights_sha256: Some("22".repeat(32)),
                    signal_kind: Some("learned_encoder".to_string()),
                    bits_about: Some(0.1),
                    vault: PathBuf::from("slot_00.ann"),
                    queries: PathBuf::from("slot_00_queries.fbin"),
                    corpus: PathBuf::from("slot_00_corpus.fbin"),
                }],
            },
        }
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
