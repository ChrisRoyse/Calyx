use std::collections::BTreeMap;
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::path::Path;

use serde::Deserialize;

use super::parse::validate_text;
use crate::error::{CliError, CliResult};

#[derive(Deserialize)]
struct BatchLine {
    text: String,
    /// Per-record source provenance (source_url, doi, pmid, license, ...). Stored
    /// verbatim on the constellation metadata map; survives raw-source deletion.
    #[serde(default)]
    metadata: BTreeMap<String, String>,
}

pub(super) type BatchRow = (String, BTreeMap<String, String>);

/// Parse one batch JSONL line; `None` for a blank line. Shared by the in-memory
/// `read_batch_texts` and the streaming ingest path.
pub(super) fn parse_batch_line(index: usize, line: &str) -> CliResult<Option<BatchRow>> {
    if line.trim().is_empty() {
        return Ok(None);
    }
    let parsed: BatchLine = serde_json::from_str(line)
        .map_err(|err| CliError::io(format!("batch JSONL line {} is invalid: {err}", index + 1)))?;
    validate_text(&parsed.text)?;
    Ok(Some((parsed.text, parsed.metadata)))
}

#[cfg(test)]
pub(super) fn read_batch_texts(path: &Path) -> CliResult<Vec<BatchRow>> {
    let raw = fs::read_to_string(path)?;
    let mut rows = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if let Some(row) = parse_batch_line(index, line)? {
            rows.push(row);
        }
    }
    Ok(rows)
}
