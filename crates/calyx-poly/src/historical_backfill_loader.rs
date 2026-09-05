//! Historical resolved-market dump loader for terminal/reference use (#35).
//!
//! These rows are closed-market history. They may support reference graphs and calibration sanity
//! checks, but they must never enter a pre-resolution snapshot corpus.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{PolyError, Result};

pub const HISTORICAL_TERMINAL_REFERENCE_SCHEMA_VERSION: &str =
    "poly.historical_terminal_reference.v1";
pub const HISTORICAL_TERMINAL_REFERENCE_FILE: &str = "historical-terminal-reference-corpus.json";

pub const ERR_HISTORICAL_BACKFILL_IO: &str = "POLY_HISTORICAL_BACKFILL_IO";
pub const ERR_HISTORICAL_BACKFILL_JSONL: &str = "POLY_HISTORICAL_BACKFILL_JSONL";
pub const ERR_HISTORICAL_BACKFILL_INVALID_ROW: &str = "POLY_HISTORICAL_BACKFILL_INVALID_ROW";
pub const ERR_HISTORICAL_BACKFILL_DUPLICATE: &str = "POLY_HISTORICAL_BACKFILL_DUPLICATE";
pub const ERR_HISTORICAL_BACKFILL_ROUTE_FORBIDDEN: &str =
    "POLY_HISTORICAL_BACKFILL_ROUTE_FORBIDDEN";
pub const ERR_HISTORICAL_BACKFILL_READBACK_MISMATCH: &str =
    "POLY_HISTORICAL_BACKFILL_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoricalTerminalRecord {
    pub source_dataset: String,
    pub source_row_index: usize,
    pub venue: String,
    pub ticker: String,
    pub title: String,
    pub category: String,
    pub volume: f64,
    pub predicted_price_raw: Option<f64>,
    pub predicted_price_t24h_raw: Option<f64>,
    pub resolved_outcome: u8,
    pub resolved_at: String,
    pub terminal: bool,
    pub reference_only: bool,
    pub pre_resolution_eligible: bool,
    pub row_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoricalTerminalCorpus {
    pub schema_version: String,
    pub corpus_kind: String,
    pub source_dataset: String,
    pub source_url: String,
    pub source_path: String,
    pub source_sha256: String,
    pub source_bytes: u64,
    pub source_line_count: usize,
    pub rows_seen: usize,
    pub rows_loaded: usize,
    pub skipped_non_polymarket: usize,
    pub duplicate_exact: usize,
    pub truncated_by_limit: bool,
    pub records: Vec<HistoricalTerminalRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoricalBackfillReport {
    pub schema_version: String,
    pub source_path: String,
    pub corpus_path: String,
    pub rows_seen: usize,
    pub rows_loaded: usize,
    pub skipped_non_polymarket: usize,
    pub duplicate_exact: usize,
    pub truncated_by_limit: bool,
    pub readback_matched: bool,
    pub corpus: HistoricalTerminalCorpus,
}

pub fn load_historical_terminal_reference_corpus(
    source_path: &Path,
    output_dir: &Path,
    source_dataset: &str,
    source_url: &str,
    max_polymarket_rows: usize,
) -> Result<HistoricalBackfillReport> {
    if max_polymarket_rows == 0 {
        return Err(PolyError::raw_source(
            ERR_HISTORICAL_BACKFILL_INVALID_ROW,
            "historical loader requires max_polymarket_rows > 0",
        ));
    }

    let bytes = fs::read(source_path).map_err(|e| {
        PolyError::raw_source(
            ERR_HISTORICAL_BACKFILL_IO,
            format!("read {}: {e}", source_path.display()),
        )
    })?;
    let source_sha256 = sha256_hex(&bytes);
    let text = String::from_utf8(bytes.clone()).map_err(|e| {
        PolyError::raw_source(
            ERR_HISTORICAL_BACKFILL_JSONL,
            format!("{} is not valid UTF-8 JSONL: {e}", source_path.display()),
        )
    })?;

    let source_line_count = text.lines().count();
    let mut records = Vec::new();
    let mut skipped_non_polymarket = 0usize;
    let mut duplicate_exact = 0usize;
    let mut rows_seen = 0usize;
    let mut truncated_by_limit = false;
    let mut seen: BTreeMap<String, DuplicateFingerprint> = BTreeMap::new();

    for (idx, line) in text.lines().enumerate() {
        if records.len() == max_polymarket_rows {
            truncated_by_limit = idx < source_line_count;
            break;
        }
        rows_seen = idx + 1;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line).map_err(|e| {
            PolyError::raw_source(
                ERR_HISTORICAL_BACKFILL_JSONL,
                format!(
                    "{} line {} is not a JSON object: {e}",
                    source_path.display(),
                    idx + 1
                ),
            )
        })?;
        let venue = required_string(&value, "venue", idx + 1)?;
        if !venue.eq_ignore_ascii_case("polymarket") {
            skipped_non_polymarket += 1;
            continue;
        }
        let record = terminal_record(&value, source_dataset, idx + 1, line)?;
        let fingerprint = DuplicateFingerprint::from(&record);
        match seen.get(&record.ticker) {
            Some(prev) if prev == &fingerprint => {
                duplicate_exact += 1;
                continue;
            }
            Some(_) => {
                return Err(PolyError::raw_source(
                    ERR_HISTORICAL_BACKFILL_DUPLICATE,
                    format!(
                        "ticker {} appears with conflicting terminal fields",
                        record.ticker
                    ),
                ));
            }
            None => {
                seen.insert(record.ticker.clone(), fingerprint);
            }
        }
        records.push(record);
    }

    if records.is_empty() {
        return Err(PolyError::raw_source(
            ERR_HISTORICAL_BACKFILL_INVALID_ROW,
            "historical loader found no Polymarket terminal rows",
        ));
    }

    let corpus = HistoricalTerminalCorpus {
        schema_version: HISTORICAL_TERMINAL_REFERENCE_SCHEMA_VERSION.to_string(),
        corpus_kind: "terminal_reference".to_string(),
        source_dataset: source_dataset.to_string(),
        source_url: source_url.to_string(),
        source_path: source_path.display().to_string(),
        source_sha256,
        source_bytes: bytes.len() as u64,
        source_line_count,
        rows_seen,
        rows_loaded: records.len(),
        skipped_non_polymarket,
        duplicate_exact,
        truncated_by_limit,
        records,
    };

    fs::create_dir_all(output_dir).map_err(|e| {
        PolyError::raw_source(
            ERR_HISTORICAL_BACKFILL_IO,
            format!("create {}: {e}", output_dir.display()),
        )
    })?;
    let corpus_path = output_dir.join(HISTORICAL_TERMINAL_REFERENCE_FILE);
    fs::write(
        &corpus_path,
        serde_json::to_vec_pretty(&corpus).map_err(|e| {
            PolyError::raw_source(
                ERR_HISTORICAL_BACKFILL_IO,
                format!("encode historical terminal corpus: {e}"),
            )
        })?,
    )
    .map_err(|e| {
        PolyError::raw_source(
            ERR_HISTORICAL_BACKFILL_IO,
            format!("write {}: {e}", corpus_path.display()),
        )
    })?;
    let readback = read_historical_terminal_reference_corpus(&corpus_path)?;
    if readback != corpus {
        return Err(PolyError::raw_source(
            ERR_HISTORICAL_BACKFILL_READBACK_MISMATCH,
            format!("{} did not read back as written", corpus_path.display()),
        ));
    }

    Ok(HistoricalBackfillReport {
        schema_version: HISTORICAL_TERMINAL_REFERENCE_SCHEMA_VERSION.to_string(),
        source_path: source_path.display().to_string(),
        corpus_path: corpus_path.display().to_string(),
        rows_seen: corpus.rows_seen,
        rows_loaded: corpus.rows_loaded,
        skipped_non_polymarket: corpus.skipped_non_polymarket,
        duplicate_exact: corpus.duplicate_exact,
        truncated_by_limit: corpus.truncated_by_limit,
        readback_matched: true,
        corpus,
    })
}

pub fn read_historical_terminal_reference_corpus(path: &Path) -> Result<HistoricalTerminalCorpus> {
    let bytes = fs::read(path).map_err(|e| {
        PolyError::raw_source(
            ERR_HISTORICAL_BACKFILL_IO,
            format!("read {}: {e}", path.display()),
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|e| {
        PolyError::raw_source(
            ERR_HISTORICAL_BACKFILL_JSONL,
            format!("decode {}: {e}", path.display()),
        )
    })
}

pub fn ensure_historical_record_pre_resolution_eligible(
    record: &HistoricalTerminalRecord,
) -> Result<()> {
    if record.terminal || record.reference_only || !record.pre_resolution_eligible {
        return Err(PolyError::raw_source(
            ERR_HISTORICAL_BACKFILL_ROUTE_FORBIDDEN,
            format!(
                "historical terminal row {} is terminal/reference and cannot enter a \
                 pre-resolution corpus",
                record.ticker
            ),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DuplicateFingerprint {
    title: String,
    category: String,
    resolved_outcome: u8,
    resolved_at: String,
}

impl From<&HistoricalTerminalRecord> for DuplicateFingerprint {
    fn from(record: &HistoricalTerminalRecord) -> Self {
        Self {
            title: record.title.clone(),
            category: record.category.clone(),
            resolved_outcome: record.resolved_outcome,
            resolved_at: record.resolved_at.clone(),
        }
    }
}

fn terminal_record(
    value: &Value,
    source_dataset: &str,
    source_row_index: usize,
    raw_line: &str,
) -> Result<HistoricalTerminalRecord> {
    let volume = required_number(value, "volume", source_row_index)?;
    let resolved_outcome = required_outcome(value, source_row_index)?;
    Ok(HistoricalTerminalRecord {
        source_dataset: source_dataset.to_string(),
        source_row_index,
        venue: "polymarket".to_string(),
        ticker: required_string(value, "ticker", source_row_index)?,
        title: required_string(value, "title", source_row_index)?,
        category: required_string(value, "category", source_row_index)?,
        volume,
        predicted_price_raw: optional_number(value, "predicted_price", source_row_index)?,
        predicted_price_t24h_raw: optional_number(value, "predicted_price_t24h", source_row_index)?,
        resolved_outcome,
        resolved_at: required_string(value, "resolved_at", source_row_index)?,
        terminal: true,
        reference_only: true,
        pre_resolution_eligible: false,
        row_sha256: sha256_hex(raw_line.as_bytes()),
    })
}

fn required_string(value: &Value, field: &str, line: usize) -> Result<String> {
    match value.get(field).and_then(Value::as_str) {
        Some(s) if !s.trim().is_empty() => Ok(s.to_string()),
        _ => Err(invalid_row(
            line,
            format!("missing non-empty string field {field}"),
        )),
    }
}

fn required_number(value: &Value, field: &str, line: usize) -> Result<f64> {
    match value.get(field).and_then(Value::as_f64) {
        Some(n) if n.is_finite() => Ok(n),
        _ => Err(invalid_row(
            line,
            format!("missing finite number field {field}"),
        )),
    }
}

fn optional_number(value: &Value, field: &str, line: usize) -> Result<Option<f64>> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_f64() {
            Some(n) if n.is_finite() => Ok(Some(n)),
            _ => Err(invalid_row(
                line,
                format!("field {field} is not a finite number"),
            )),
        },
    }
}

fn required_outcome(value: &Value, line: usize) -> Result<u8> {
    match value.get("resolved_outcome").and_then(Value::as_u64) {
        Some(0) => Ok(0),
        Some(1) => Ok(1),
        _ => Err(invalid_row(
            line,
            "resolved_outcome must be integer 0 or 1".to_string(),
        )),
    }
}

fn invalid_row(line: usize, message: String) -> PolyError {
    PolyError::raw_source(
        ERR_HISTORICAL_BACKFILL_INVALID_ROW,
        format!("historical JSONL line {line}: {message}"),
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
