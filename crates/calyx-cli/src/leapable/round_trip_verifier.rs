mod benchmark;
mod calyx;
mod cli;
mod source;

use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Constellation};
use serde::{Deserialize, Serialize};

use crate::error::{CliError, CliResult};
use crate::migrate::manifest::hex_encode;
use crate::migrate::reader::{ChunkRow, open_sqlite};

pub(crate) const CALYX_ROUND_TRIP_MISMATCH: &str = "CALYX_ROUND_TRIP_MISMATCH";
pub(crate) const CALYX_ROUND_TRIP_GATE_FAILED: &str = "CALYX_ROUND_TRIP_GATE_FAILED";
pub(crate) const CALYX_CONTRACT_NAME_MISMATCH: &str = "CALYX_CONTRACT_NAME_MISMATCH";
pub(crate) const CALYX_AB_RECALL_BELOW_BASELINE: &str = "CALYX_AB_RECALL_BELOW_BASELINE";
pub(crate) const CALYX_LATENCY_REGRESSION: &str = "CALYX_LATENCY_REGRESSION";
pub(crate) const CALYX_ROUND_TRIP_QUERY_INVALID: &str = "CALYX_ROUND_TRIP_QUERY_INVALID";
pub(crate) const CALYX_MANIFEST_CORRUPT: &str = "CALYX_MANIFEST_CORRUPT";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MismatchDetail {
    pub(crate) code: String,
    pub(crate) chunk_id: String,
    pub(crate) field: String,
    pub(crate) expected_hash: String,
    pub(crate) actual_hash: String,
    pub(crate) expected_value: Option<String>,
    pub(crate) actual_value: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct VerifyReport {
    pub(crate) sqlite_path: PathBuf,
    pub(crate) calyx_dir: PathBuf,
    pub(crate) aster_dir: PathBuf,
    pub(crate) database_name: Option<String>,
    pub(crate) total: usize,
    pub(crate) matched: usize,
    pub(crate) mismatches: Vec<MismatchDetail>,
    pub(crate) gate: String,
}

impl VerifyReport {
    pub(crate) fn gate_passes(&self) -> bool {
        self.mismatches.is_empty() && self.matched == self.total
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct QueryVec {
    #[serde(alias = "query_vector", alias = "vector")]
    pub(crate) query_vec: Vec<f32>,
    #[serde(default, alias = "expected")]
    pub(crate) expected_chunk_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct RecallQueryResult {
    pub(crate) query_hash: String,
    pub(crate) sqlite_top: Vec<String>,
    pub(crate) calyx_top: Vec<String>,
    pub(crate) sqlite_recall: f64,
    pub(crate) calyx_recall: f64,
    pub(crate) latency_sqlite_us: u64,
    pub(crate) latency_calyx_us: u64,
    pub(crate) gate: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct RecallBenchmark {
    pub(crate) sqlite_mean_recall: f64,
    pub(crate) calyx_mean_recall: f64,
    pub(crate) latency_sqlite_p99_us: u64,
    pub(crate) latency_calyx_p99_us: u64,
    pub(crate) latency_budget_us: u64,
    pub(crate) queries: Vec<RecallQueryResult>,
    pub(crate) gate: String,
}

pub(crate) struct RoundTripVerifier;

impl RoundTripVerifier {
    pub(crate) fn verify(sqlite_path: &Path, calyx_dir: &Path) -> Result<VerifyReport, CalyxError> {
        let conn = open_sqlite(sqlite_path).map_err(cli_to_calyx)?;
        let rows = source::source_rows(&conn)?;
        let (aster_dir, vault, _manifest) = calyx::open_calyx(calyx_dir)?;
        let index = calyx::calyx_index(&vault)?;
        let mut mismatches = Vec::new();
        let mut matched = 0;
        for row in &rows {
            let before = mismatches.len();
            calyx::verify_row(row, &index, &mut mismatches);
            if mismatches.len() == before {
                matched += 1;
            }
        }
        let gate = if mismatches.is_empty() && matched == rows.len() {
            "PASS"
        } else {
            "FAIL"
        };
        Ok(VerifyReport {
            sqlite_path: sqlite_path.to_path_buf(),
            calyx_dir: calyx_dir.to_path_buf(),
            aster_dir,
            database_name: rows.first().map(|row| row.row.database_name.clone()),
            total: rows.len(),
            matched,
            mismatches,
            gate: gate.to_string(),
        })
    }

    pub(crate) fn benchmark_recall(
        sqlite_path: &Path,
        calyx_dir: &Path,
        queries: &[QueryVec],
        top_k: usize,
    ) -> Result<RecallBenchmark, CalyxError> {
        benchmark::benchmark_recall(sqlite_path, calyx_dir, queries, top_k)
    }
}

pub(crate) fn run_verify_round_trip(args: &[String]) -> CliResult {
    cli::run_verify_round_trip(args)
}

#[derive(Clone)]
pub(super) struct SourceRow {
    pub(super) row: ChunkRow,
    pub(super) text_hash: [u8; 32],
}

pub(super) struct CalyxEntry {
    pub(super) cx: Constellation,
    pub(super) vector: Option<Vec<f32>>,
}

#[derive(Clone)]
pub(super) struct BenchmarkRow {
    pub(super) chunk_id: String,
    pub(super) embedding: Vec<f32>,
}

pub(super) fn mismatch(
    code: &str,
    chunk_id: &str,
    field: &str,
    expected_hash: &[u8; 32],
    actual_hash: &[u8; 32],
    expected_value: Option<String>,
    actual_value: Option<String>,
) -> MismatchDetail {
    MismatchDetail {
        code: code.to_string(),
        chunk_id: chunk_id.to_string(),
        field: field.to_string(),
        expected_hash: hex_encode(expected_hash),
        actual_hash: hex_encode(actual_hash),
        expected_value,
        actual_value,
    }
}

pub(super) fn vector_hash(vector: &[f32]) -> [u8; 32] {
    blake3_bytes(&query_bytes(vector))
}

pub(super) fn query_bytes(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

pub(super) fn blake3_bytes(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

pub(super) fn cosine(left: &[f32], right: &[f32]) -> f64 {
    let dot = left
        .iter()
        .zip(right)
        .map(|(left, right)| f64::from(*left) * f64::from(*right))
        .sum::<f64>();
    dot / (norm(left) * norm(right)).max(f64::MIN_POSITIVE)
}

pub(super) fn norm(vector: &[f32]) -> f64 {
    vector
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>()
        .sqrt()
}

pub(super) fn cli_to_calyx(error: CliError) -> CalyxError {
    CalyxError {
        code: error.code(),
        message: error.message().to_string(),
        remediation: error.remediation(),
    }
}

pub(super) fn manifest_corrupt(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_MANIFEST_CORRUPT,
        message,
        "repair or regenerate the Calyx migration/shadow manifest before verifying",
    )
}

pub(super) fn error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
