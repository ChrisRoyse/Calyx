use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, SlotVector, VaultStore};
use serde::{Deserialize, Serialize};

use super::dual_write::{CALYX_SHADOW_WRITE_FAILED, aster_dir};
use crate::error::{CliError, CliResult};
use crate::migrate;
use crate::migrate::adapter::BASE_SLOT;
use crate::migrate::manifest::{MigrationManifest, hex_encode};
use crate::migrate::reader::{ChunkRow, open_sqlite, stream_rows};
use crate::output::print_json;

pub(crate) const CALYX_RECALL_PARITY_BELOW_BASELINE: &str = "CALYX_RECALL_PARITY_BELOW_BASELINE";
pub(crate) const CALYX_INVALID_VECTOR: &str = "CALYX_INVALID_VECTOR";
pub(crate) const CALYX_INVALID_TOP_K: &str = "CALYX_INVALID_TOP_K";
pub(crate) const PARITY_REPORT_NAME: &str = "PARITY_REPORT.jsonl";

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct QuerySpec {
    #[serde(alias = "query_vector", alias = "vector")]
    pub(crate) query_vec: Vec<f32>,
    #[serde(default, alias = "expected")]
    pub(crate) expected_chunk_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct QueryParity {
    pub(crate) query_hash: String,
    pub(crate) sqlite_top: Vec<String>,
    pub(crate) calyx_top: Vec<String>,
    pub(crate) sqlite_recall: f64,
    pub(crate) calyx_recall: f64,
    pub(crate) delta: f64,
    pub(crate) gate: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ParityReport {
    pub(crate) sqlite_path: PathBuf,
    pub(crate) calyx_dir: PathBuf,
    pub(crate) aster_dir: PathBuf,
    pub(crate) top_k: usize,
    pub(crate) queries: Vec<QueryParity>,
    pub(crate) gate: String,
    pub(crate) report_path: PathBuf,
}

pub(crate) struct RecallComparator;

impl RecallComparator {
    pub(crate) fn compare(
        sqlite: &Path,
        calyx_dir: &Path,
        queries: &[QuerySpec],
        top_k: usize,
    ) -> CliResult<ParityReport> {
        if top_k == 0 {
            return Err(error(
                CALYX_INVALID_TOP_K,
                "top_k must be greater than zero",
                "rerun recall-compare with --top-k >= 1",
            )
            .into());
        }
        let conn = open_sqlite(sqlite)?;
        let rows = stream_rows(&conn)?;
        let dim = rows.first().map(|row| row.embedding.len()).unwrap_or(0);
        let aster = aster_dir(calyx_dir);
        let manifest = MigrationManifest::load(&aster)?;
        let vault = migrate::open_vault(&aster, &manifest)?;
        let adapter = migrate::adapter(&manifest)?;
        let snapshot = vault.snapshot();
        let mut parity_rows = Vec::new();
        for query in queries {
            validate_query(query, dim)?;
            let sqlite_top = top_rows(&rows, &query.query_vec, top_k);
            let calyx_vectors = calyx_rows(&vault, &adapter, &rows, snapshot)?;
            let calyx_top = top_rows(&calyx_vectors, &query.query_vec, top_k);
            let baseline = expected_ids(query, &sqlite_top);
            let sqlite_recall = recall(&sqlite_top, &baseline);
            let calyx_recall = recall(&calyx_top, &baseline);
            let gate = if calyx_recall + f64::EPSILON >= sqlite_recall {
                "PASS"
            } else {
                "FAIL"
            };
            parity_rows.push(QueryParity {
                query_hash: hex_encode(blake3::hash(&query_bytes(&query.query_vec)).as_bytes()),
                sqlite_top: ids(&sqlite_top),
                calyx_top: ids(&calyx_top),
                sqlite_recall,
                calyx_recall,
                delta: calyx_recall - sqlite_recall,
                gate: gate.to_string(),
            });
        }
        let gate = if parity_rows.iter().all(|row| row.gate == "PASS") {
            "PASS"
        } else {
            "FAIL"
        };
        let report_path = write_report(calyx_dir, top_k, &parity_rows, gate)?;
        Ok(ParityReport {
            sqlite_path: sqlite.to_path_buf(),
            calyx_dir: calyx_dir.to_path_buf(),
            aster_dir: aster,
            top_k,
            queries: parity_rows,
            gate: gate.to_string(),
            report_path,
        })
    }
}

pub(crate) fn run_recall_compare(args: &[String]) -> CliResult {
    let args = parse_args(args)?;
    let queries = read_queries(&args.queries)?;
    let report = RecallComparator::compare(&args.sqlite, &args.calyx, &queries, args.top_k)?;
    let failed = report.gate != "PASS";
    print_json(&report)?;
    if failed {
        return Err(error(
            CALYX_RECALL_PARITY_BELOW_BASELINE,
            "calyx recall fell below sqlite baseline",
            "inspect PARITY_REPORT.jsonl, then replay dual-write before enabling Calyx reads",
        )
        .into());
    }
    Ok(())
}

fn validate_query(query: &QuerySpec, dim: usize) -> CliResult {
    if query.query_vec.len() != dim
        || query.query_vec.iter().any(|value| !value.is_finite())
        || norm(&query.query_vec) == 0.0
    {
        return Err(error(
            CALYX_INVALID_VECTOR,
            format!(
                "query vector dim {} expected {dim} finite nonzero values",
                query.query_vec.len()
            ),
            "provide a finite nonzero query vector matching the source embedding dimension",
        )
        .into());
    }
    Ok(())
}

fn calyx_rows(
    vault: &calyx_aster::vault::AsterVault,
    adapter: &crate::migrate::adapter::VaultSqliteAdapter,
    rows: &[ChunkRow],
    snapshot: u64,
) -> CliResult<Vec<ChunkRow>> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(SlotVector::Dense { data, .. }) =
            vault.read_slot_vector_at(snapshot, adapter.cx_id(row), BASE_SLOT)?
        else {
            return Err(error(
                CALYX_SHADOW_WRITE_FAILED,
                format!("missing Calyx base slot for {}", row.chunk_id),
                "rerun dual-write and read back the Aster slot column family",
            )
            .into());
        };
        let mut copy = row.clone();
        copy.embedding = data;
        out.push(copy);
    }
    Ok(out)
}

fn top_rows(rows: &[ChunkRow], query: &[f32], top_k: usize) -> Vec<ChunkRow> {
    let mut scored = rows
        .iter()
        .map(|row| (cosine(query, &row.embedding), row))
        .collect::<Vec<_>>();
    scored.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .partial_cmp(left_score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.chunk_id.cmp(&right.chunk_id))
    });
    scored
        .into_iter()
        .take(top_k)
        .map(|(_, row)| row.clone())
        .collect()
}

fn cosine(left: &[f32], right: &[f32]) -> f64 {
    let dot = left
        .iter()
        .zip(right)
        .map(|(left, right)| f64::from(*left) * f64::from(*right))
        .sum::<f64>();
    let denom = norm(left) * norm(right);
    if denom == 0.0 { 0.0 } else { dot / denom }
}

fn norm(vector: &[f32]) -> f64 {
    vector
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>()
        .sqrt()
}

fn expected_ids(query: &QuerySpec, sqlite_top: &[ChunkRow]) -> BTreeSet<String> {
    let ids = if query.expected_chunk_ids.is_empty() {
        ids(sqlite_top)
    } else {
        query.expected_chunk_ids.clone()
    };
    ids.into_iter().collect()
}

fn recall(top: &[ChunkRow], expected: &BTreeSet<String>) -> f64 {
    if expected.is_empty() {
        return 1.0;
    }
    let hits = top
        .iter()
        .filter(|row| expected.contains(&row.chunk_id))
        .count();
    hits as f64 / expected.len() as f64
}

fn ids(rows: &[ChunkRow]) -> Vec<String> {
    rows.iter().map(|row| row.chunk_id.clone()).collect()
}

fn query_bytes(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn write_report(
    calyx_dir: &Path,
    top_k: usize,
    queries: &[QueryParity],
    gate: &str,
) -> CliResult<PathBuf> {
    fs::create_dir_all(calyx_dir)
        .map_err(|err| CliError::io(format!("create {}: {err}", calyx_dir.display())))?;
    let path = calyx_dir.join(PARITY_REPORT_NAME);
    let mut file = File::create(&path)
        .map_err(|err| CliError::io(format!("create {}: {err}", path.display())))?;
    let value = serde_json::json!({"top_k": top_k, "gate": gate, "queries": queries});
    serde_json::to_writer(&mut file, &value)
        .map_err(|err| CliError::io(format!("write parity report: {err}")))?;
    file.write_all(b"\n")
        .map_err(|err| CliError::io(format!("write parity newline: {err}")))?;
    file.sync_all()
        .map_err(|err| CliError::io(format!("sync {}: {err}", path.display())))?;
    Ok(path)
}

fn read_queries(path: &Path) -> CliResult<Vec<QuerySpec>> {
    let file = File::open(path)
        .map_err(|err| CliError::io(format!("open queries {}: {err}", path.display())))?;
    BufReader::new(file)
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| match line {
            Ok(line) if line.trim().is_empty() => None,
            other => Some((idx, other)),
        })
        .map(|(idx, line)| {
            let line =
                line.map_err(|err| CliError::io(format!("read query line {}: {err}", idx + 1)))?;
            serde_json::from_str(&line)
                .map_err(|err| CliError::usage(format!("parse query line {}: {err}", idx + 1)))
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Args {
    sqlite: PathBuf,
    calyx: PathBuf,
    queries: PathBuf,
    top_k: usize,
}

fn parse_args(args: &[String]) -> CliResult<Args> {
    let mut sqlite = None;
    let mut calyx = None;
    let mut queries = None;
    let mut top_k = 10usize;
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--sqlite" => sqlite = Some(value(args, idx, "--sqlite")?.into()),
            "--calyx" => calyx = Some(value(args, idx, "--calyx")?.into()),
            "--queries" => queries = Some(value(args, idx, "--queries")?.into()),
            "--top-k" => {
                top_k = value(args, idx, "--top-k")?
                    .parse()
                    .map_err(|err| CliError::usage(format!("parse --top-k: {err}")))?;
            }
            other => {
                return Err(CliError::usage(format!(
                    "unknown recall-compare arg {other}"
                )));
            }
        }
        idx += 2;
    }
    Ok(Args {
        sqlite: sqlite.ok_or_else(|| CliError::usage("recall-compare requires --sqlite <db>"))?,
        calyx: calyx.ok_or_else(|| CliError::usage("recall-compare requires --calyx <dir>"))?,
        queries: queries
            .ok_or_else(|| CliError::usage("recall-compare requires --queries <jsonl>"))?,
        top_k,
    })
}

fn value<'a>(args: &'a [String], idx: usize, flag: &str) -> CliResult<&'a str> {
    args.get(idx + 1)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

fn error(code: &'static str, message: impl Into<String>, remediation: &'static str) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
