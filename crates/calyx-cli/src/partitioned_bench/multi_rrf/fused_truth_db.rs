use std::collections::BTreeSet;
use std::path::Path;

use bincode::config;
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::CalyxError;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{Plan, ground_truth, report};
use crate::error::{CliError, CliResult};

const KEY_PREFIX: &[u8] = b"calyx/partitioned-rrf/fused-truth/v1/";
const VALUE_MAGIC: &[u8] = b"CRRFFT1\0";
const CF_MEMTABLE_CAP: usize = 8 * 1024 * 1024;

pub(super) const DEFAULT_ASSOCIATION_KEY: &str = "partitioned_rrf_fused_truth";

#[derive(Clone, Debug)]
pub(super) struct DbFusedTruth {
    rows: Vec<Vec<u64>>,
    source: Value,
    scale_suitable: bool,
}

#[derive(Clone, Debug)]
pub(super) struct Context<'a> {
    pub(super) cf_root: &'a Path,
    pub(super) association_key: &'a str,
    pub(super) plan_path: &'a Path,
    pub(super) plan_sha256: &'a str,
    pub(super) plan: &'a Plan,
    pub(super) truth_n: usize,
    pub(super) k: usize,
    pub(super) truth_depth: usize,
    pub(super) corpus_rows: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct FusedTruthRecord {
    format: String,
    mode: String,
    row_id_space: String,
    plan_sha256: String,
    query_count: usize,
    k: usize,
    truth_depth: usize,
    corpus_rows: usize,
    reference_backend: String,
    scale_suitable: bool,
    slots: Vec<ground_truth::TruthSlot>,
    rows: Vec<Vec<u64>>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct FusedTruthDbReadback {
    cf_root: String,
    association_key: String,
    row_key_sha256: String,
    value_bytes: usize,
    value_sha256: String,
    readback_matches: bool,
}

impl DbFusedTruth {
    pub(super) fn load(ctx: Context<'_>) -> CliResult<Self> {
        let row_key = row_key(ctx.association_key)?;
        let router = CfRouter::open(ctx.cf_root, CF_MEMTABLE_CAP).map_err(CliError::from)?;
        let value = router
            .get(ColumnFamily::Graph, &row_key)
            .map_err(CliError::from)?
            .ok_or_else(|| {
                ft_error(
                    "CALYX_FSV_PARTITIONED_RRF_FUSED_TRUTH_DB_MISSING",
                    "fused truth row missing in Graph CF",
                    "write the fused RRF truth row through Calyx/Aster Graph CF",
                )
            })?;
        let record: FusedTruthRecord = decode(&value)?;
        validate_record(&record, &ctx)?;
        let rows = load_rows(&record, &ctx)?;
        let readback = readback_report(ctx.cf_root, ctx.association_key, &row_key, &value, true);
        let source = json!({
            "mode": "precomputed_fused_rrf_aster_cf",
            "metric_class": report::METRIC_CLASS,
            "metric_scope": report::METRIC_SCOPE,
            "truth_reference_class": report::TRUTH_REFERENCE_CLASS,
            "valid_real_outcome": false,
            "grounded_phase_exit_eligible": false,
            "format": ground_truth::FORMAT,
            "cf_root": ctx.cf_root,
            "association_key": ctx.association_key,
            "db_readback": readback,
            "plan": ctx.plan_path,
            "plan_sha256": record.plan_sha256,
            "row_id_space": record.row_id_space,
            "rows": record.query_count,
            "width": record.k,
            "query_count_used": ctx.truth_n,
            "k_used": ctx.k,
            "truth_depth": record.truth_depth,
            "corpus_rows": record.corpus_rows,
            "reference_backend": record.reference_backend,
            "scale_suitable": record.scale_suitable,
            "slots": record.slots,
        });
        Ok(Self {
            rows,
            source,
            scale_suitable: record.scale_suitable,
        })
    }

    pub(super) fn row_ids(&self, query_idx: usize) -> &[u64] {
        &self.rows[query_idx]
    }

    pub(super) fn source(&self) -> Value {
        self.source.clone()
    }

    pub(super) fn scale_suitable(&self) -> bool {
        self.scale_suitable
    }
}

pub(super) fn write(rows: &[Vec<u64>], ctx: Context<'_>, scale_suitable: bool) -> CliResult<Value> {
    validate_generated_rows(rows, &ctx)?;
    let row_key = row_key(ctx.association_key)?;
    let record = FusedTruthRecord {
        format: ground_truth::FORMAT.to_string(),
        mode: ground_truth::MODE.to_string(),
        row_id_space: ground_truth::ROW_ID_SPACE.to_string(),
        plan_sha256: ctx.plan_sha256.to_string(),
        query_count: rows.len(),
        k: ctx.k,
        truth_depth: ctx.truth_depth,
        corpus_rows: ctx.corpus_rows,
        reference_backend: "calyx-bench-partitioned-rrf-fused-db-v1".to_string(),
        scale_suitable,
        slots: ground_truth::plan_slots(ctx.plan),
        rows: rows.to_vec(),
    };
    let value = encode(&record)?;
    let mut router = CfRouter::open(ctx.cf_root, CF_MEMTABLE_CAP).map_err(CliError::from)?;
    if router
        .get(ColumnFamily::Graph, &row_key)
        .map_err(CliError::from)?
        .is_some()
    {
        return Err(ft_error(
            "CALYX_FSV_PARTITIONED_RRF_FUSED_TRUTH_DB_EXISTS",
            "fused truth row already exists in Graph CF",
            "write to a fresh association key so stale truth cannot be overwritten silently",
        ));
    }
    router
        .put(ColumnFamily::Graph, &row_key, &value)
        .map_err(CliError::from)?;
    router
        .flush_cf(ColumnFamily::Graph)
        .map_err(CliError::from)?;
    drop(router);

    let reopened = CfRouter::open(ctx.cf_root, CF_MEMTABLE_CAP).map_err(CliError::from)?;
    let readback = reopened
        .get(ColumnFamily::Graph, &row_key)
        .map_err(CliError::from)?
        .ok_or_else(|| {
            ft_error(
                "CALYX_FSV_PARTITIONED_RRF_FUSED_TRUTH_DB_MISSING",
                "fused truth row missing after Graph CF write",
                "retry the write and read back the Calyx/Aster Graph CF row",
            )
        })?;
    if readback != value {
        return Err(ft_error(
            "CALYX_FSV_PARTITIONED_RRF_FUSED_TRUTH_DB_MISMATCH",
            "fused truth Graph CF readback bytes changed after write",
            "rerun after checking the Calyx/Aster Graph CF store",
        ));
    }
    let readback = readback_report(ctx.cf_root, ctx.association_key, &row_key, &readback, true);
    Ok(json!({
        "mode": "generated_fused_rrf_aster_cf",
        "metric_class": report::METRIC_CLASS,
        "metric_scope": report::METRIC_SCOPE,
        "truth_reference_class": report::TRUTH_REFERENCE_CLASS,
        "valid_real_outcome": false,
        "grounded_phase_exit_eligible": false,
        "format": ground_truth::FORMAT,
        "cf_root": ctx.cf_root,
        "association_key": ctx.association_key,
        "db_readback": readback,
        "plan_sha256": ctx.plan_sha256,
        "rows": rows.len(),
        "width": ctx.k,
        "truth_depth": ctx.truth_depth,
        "corpus_rows": ctx.corpus_rows,
        "reference_backend": "calyx-bench-partitioned-rrf-fused-db-v1",
        "scale_suitable": scale_suitable,
    }))
}

fn validate_record(record: &FusedTruthRecord, ctx: &Context<'_>) -> CliResult {
    if record.format != ground_truth::FORMAT
        || record.mode != ground_truth::MODE
        || record.row_id_space != ground_truth::ROW_ID_SPACE
    {
        return Err(ft_error(
            "CALYX_FSV_PARTITIONED_RRF_FUSED_TRUTH_DB_NOT_FUSED_PANEL",
            "fused truth row is not a fused RRF panel truth record",
            "write a fused RRF truth row for this partitioned plan",
        ));
    }
    if record.plan_sha256 != ctx.plan_sha256 {
        return Err(stale("plan_sha256", &record.plan_sha256, ctx.plan_sha256));
    }
    if record.query_count < ctx.truth_n || record.k < ctx.k || record.corpus_rows != ctx.corpus_rows
    {
        return Err(ft_error(
            "CALYX_FSV_PARTITIONED_RRF_FUSED_TRUTH_DB_MISMATCH",
            format!(
                "DB truth query_count/k/corpus_rows = {}/{}/{} but run needs {}/{}/{}",
                record.query_count,
                record.k,
                record.corpus_rows,
                ctx.truth_n,
                ctx.k,
                ctx.corpus_rows
            ),
            "regenerate fused truth for this query count, k, and corpus row count",
        ));
    }
    if record.slots != ground_truth::plan_slots(ctx.plan) {
        return Err(ft_error(
            "CALYX_FSV_PARTITIONED_RRF_FUSED_TRUTH_DB_STALE",
            "DB fused truth lens roster does not match the current RRF plan",
            "regenerate fused truth after changing lenses, weights, or slot order",
        ));
    }
    Ok(())
}

fn load_rows(record: &FusedTruthRecord, ctx: &Context<'_>) -> CliResult<Vec<Vec<u64>>> {
    if record.rows.len() < ctx.truth_n {
        return Err(ft_error(
            "CALYX_FSV_PARTITIONED_RRF_FUSED_TRUTH_DB_MISMATCH",
            format!(
                "DB truth rows={} but run needs {}",
                record.rows.len(),
                ctx.truth_n
            ),
            "regenerate fused truth with enough query rows",
        ));
    }
    record
        .rows
        .iter()
        .take(ctx.truth_n)
        .enumerate()
        .map(|(idx, row)| validate_row(row, idx, ctx))
        .collect()
}

fn validate_generated_rows(rows: &[Vec<u64>], ctx: &Context<'_>) -> CliResult {
    if rows.len() < ctx.truth_n {
        return Err(ft_error(
            "CALYX_FSV_PARTITIONED_RRF_FUSED_TRUTH_DB_INVALID",
            format!(
                "generated fused truth rows={} but run needs {}",
                rows.len(),
                ctx.truth_n
            ),
            "rerun fused recall with enough ground-truth query rows",
        ));
    }
    for (idx, row) in rows.iter().take(ctx.truth_n).enumerate() {
        validate_row(row, idx, ctx)?;
    }
    Ok(())
}

fn validate_row(row: &[u64], idx: usize, ctx: &Context<'_>) -> CliResult<Vec<u64>> {
    if row.len() < ctx.k {
        return Err(ft_error(
            "CALYX_FSV_PARTITIONED_RRF_FUSED_TRUTH_DB_MISMATCH",
            format!(
                "DB truth row {idx} width={} is smaller than k {}",
                row.len(),
                ctx.k
            ),
            "regenerate fused truth with width at least k",
        ));
    }
    let mut seen = BTreeSet::new();
    for &id in row.iter().take(ctx.k) {
        if id >= ctx.corpus_rows as u64 || !seen.insert(id) {
            return Err(ft_error(
                "CALYX_FSV_PARTITIONED_RRF_FUSED_TRUTH_DB_INVALID",
                format!("DB truth row {idx} has invalid or repeated id {id}"),
                "regenerate fused truth with unique corpus row ids in range",
            ));
        }
    }
    Ok(row.iter().take(ctx.k).copied().collect())
}

fn row_key(association_key: &str) -> CliResult<Vec<u8>> {
    if association_key.trim().is_empty() {
        return Err(ft_error(
            "CALYX_FSV_PARTITIONED_RRF_FUSED_TRUTH_DB_INVALID_KEY",
            "fused truth association key must be non-empty",
            "pass a non-empty --fused-ground-truth-key or --write-fused-ground-truth-key",
        ));
    }
    let mut key = Vec::with_capacity(KEY_PREFIX.len() + association_key.len());
    key.extend_from_slice(KEY_PREFIX);
    key.extend_from_slice(association_key.as_bytes());
    Ok(key)
}

fn encode<T: Serialize>(record: &T) -> CliResult<Vec<u8>> {
    let mut bytes = VALUE_MAGIC.to_vec();
    let payload = bincode::serde::encode_to_vec(record, config::standard()).map_err(|err| {
        ft_error(
            "CALYX_FSV_PARTITIONED_RRF_FUSED_TRUTH_DB_ENCODE",
            format!("encode fused truth record failed: {err}"),
            "serialize a valid fused RRF truth record",
        )
    })?;
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> CliResult<T> {
    let payload = bytes.strip_prefix(VALUE_MAGIC).ok_or_else(|| {
        ft_error(
            "CALYX_FSV_PARTITIONED_RRF_FUSED_TRUTH_DB_INVALID",
            "fused truth row has invalid magic",
            "read the Graph CF row written by this Calyx fused-truth path",
        )
    })?;
    let (record, consumed): (T, usize) =
        bincode::serde::decode_from_slice(payload, config::standard()).map_err(|err| {
            ft_error(
                "CALYX_FSV_PARTITIONED_RRF_FUSED_TRUTH_DB_DECODE",
                format!("decode fused truth record failed: {err}"),
                "rewrite the fused truth Graph CF row",
            )
        })?;
    if consumed != payload.len() {
        return Err(ft_error(
            "CALYX_FSV_PARTITIONED_RRF_FUSED_TRUTH_DB_INVALID",
            "fused truth row has trailing bytes",
            "rewrite the fused truth Graph CF row",
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
) -> FusedTruthDbReadback {
    FusedTruthDbReadback {
        cf_root: cf_root.display().to_string(),
        association_key: association_key.to_string(),
        row_key_sha256: hex_sha256(row_key),
        value_bytes: value.len(),
        value_sha256: hex_sha256(value),
        readback_matches,
    }
}

fn stale(field: &'static str, expected: &str, actual: &str) -> CliError {
    ft_error(
        "CALYX_FSV_PARTITIONED_RRF_FUSED_TRUTH_DB_STALE",
        format!("{field} DB={expected} actual={actual}"),
        "regenerate fused truth from the current plan and source rows",
    )
}

fn ft_error(code: &'static str, message: impl Into<String>, remediation: &'static str) -> CliError {
    CliError::Calyx(CalyxError {
        code,
        message: message.into(),
        remediation,
    })
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("hex write");
    }
    out
}

#[cfg(test)]
#[path = "fused_truth_db_tests.rs"]
mod tests;
