use std::collections::BTreeMap;
#[cfg(test)]
use std::fs;
use std::io::BufRead;
use std::path::Path;

use calyx_core::{
    Anchor, CALYX_STRUCTURED_RESERVED_METADATA, CALYX_STRUCTURED_SCHEMA_MISMATCH, CalyxError,
    STRUCTURED_METADATA_PREFIX, StructuredRecord,
};
use serde::Deserialize;
use serde_json::Value;

use super::super::vault::now_ms;
use super::anchor::{parse_anchor_kind, parse_anchor_value};
use super::oracle_event::{OracleEvent, OracleEventSpec, parse_oracle_event};
use super::parse::{validate_confidence, validate_text};
use crate::error::{CliError, CliResult};

/// Default source recorded on an anchor threaded in at ingest time, when the
/// JSONL line does not name its own `source`. Distinguishes ingest-time grounding
/// from a post-hoc `calyx anchor` seal (`calyx-cli`).
const DEFAULT_INGEST_ANCHOR_SOURCE: &str = "calyx-ingest";
const CALYX_INGEST_BATCH_INVALID: &str = "CALYX_INGEST_BATCH_INVALID";
const INVALID_BATCH_REMEDIATION: &str = "repair the JSON object at the named JSONL line and parser column, preserving exactly one complete UTF-8 JSON object per line, then rerun ingest";

/// One typed anchor on a batch JSONL line. The feeder/corpus-builder decides what
/// to attach (e.g. the QA correct-answer as `label:answer`, the source domain as
/// `label:dataset`, or `test-pass` for verified-correct rows); the ingest engine
/// stays domain-agnostic and only validates + threads them onto the constellation.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnchorSpec {
    /// Anchor kind token, parsed by `parse_anchor_kind` (`test-pass`, `thumbs-up`,
    /// `thumbs-down`, `speaker-match`, `style-hold`, or `label:<axis>`).
    kind: String,
    /// Observed value on the anchor axis (e.g. the answer `"B"` for `label:answer`).
    value: String,
    /// Grounding source; defaults to `calyx-ingest`.
    #[serde(default)]
    source: Option<String>,
    /// Confidence in [0, 1]; oracles/verified labels use 1.0 (the default).
    #[serde(default)]
    confidence: Option<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchLine {
    text: String,
    /// Stable schema identity for the canonical typed record.
    #[serde(default)]
    record_schema: Option<String>,
    /// Explicit structured atoms measured independently from `text`.
    #[serde(default)]
    record: Option<Value>,
    /// Per-record source provenance (source_url, doi, pmid, license, ...). Stored
    /// verbatim on the constellation metadata map; survives raw-source deletion.
    #[serde(default)]
    metadata: Option<BTreeMap<String, String>>,
    /// Typed grounding anchors for this record. Threaded onto the constellation at
    /// ingest (same base-CF + Anchors-CF write the `calyx anchor` command performs),
    /// so the kernel can reach them via `groundedness_distance` without a separate
    /// per-row seal pass. Empty by default (ungrounded ingest, unchanged behaviour).
    #[serde(default)]
    anchors: Vec<AnchorSpec>,
    /// Optional Oracle recurrence/event structuring. When present, ingest stores
    /// oracle.domain/action metadata and appends a bounded Recurrence CF context.
    #[serde(default)]
    oracle: Option<OracleEventSpec>,
}

/// A parsed batch row: input text, provenance metadata, and the typed anchors to
/// ground it at ingest. Shared by the in-memory `read_batch_texts` and the
/// streaming ingest path.
#[derive(Debug)]
pub(super) struct BatchRow {
    pub(super) text: String,
    pub(super) metadata: BTreeMap<String, String>,
    pub(super) anchors: Vec<Anchor>,
    pub(super) oracle: Option<OracleEvent>,
    pub(super) structured: Option<StructuredRecord>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BatchValidation {
    pub line_count: usize,
    pub row_count: usize,
}

/// Parse one batch JSONL line; `None` for a blank line.
///
/// A malformed anchor (unknown kind, unparseable value, out-of-range confidence)
/// is a hard error that names the line — anchors are grounding truth, so a bad one
/// must fail loudly rather than be silently dropped (doctrine: no silent fallback).
pub(super) fn parse_batch_line(index: usize, line: &str) -> CliResult<Option<BatchRow>> {
    if line.trim().is_empty() {
        return Ok(None);
    }
    let parsed: BatchLine =
        serde_json::from_str(line).map_err(|error| invalid_batch(index + 1, error.to_string()))?;
    validate_text(&parsed.text)?;
    let structured = parse_structured_record(index, parsed.record_schema, parsed.record)?;
    let metadata = validate_provenance(index, parsed.metadata)?;
    let mut anchors = Vec::with_capacity(parsed.anchors.len());
    for spec in parsed.anchors {
        anchors.push(parse_anchor_spec(index, spec)?);
    }
    let oracle = parsed
        .oracle
        .map(|spec| parse_oracle_event(index, spec))
        .transpose()?;
    Ok(Some(BatchRow {
        text: parsed.text,
        metadata,
        anchors,
        oracle,
        structured,
    }))
}

fn parse_structured_record(
    index: usize,
    schema: Option<String>,
    record: Option<Value>,
) -> CliResult<Option<StructuredRecord>> {
    match (schema, record) {
        (None, None) => Ok(None),
        (Some(schema), Some(record)) => StructuredRecord::from_json_value(schema, &record)
            .map(Some)
            .map_err(|error| {
                CliError::from(CalyxError {
                    code: error.code,
                    message: format!(
                        "batch JSONL line {} structured record is invalid: {}",
                        index + 1,
                        error.message
                    ),
                    remediation: error.remediation,
                })
            }),
        _ => Err(CliError::from(CalyxError {
            code: CALYX_STRUCTURED_SCHEMA_MISMATCH,
            message: format!(
                "batch JSONL line {} must provide record_schema and record together",
                index + 1
            ),
            remediation: "supply both the exact stable schema id and its typed JSON object",
        })),
    }
}

/// Parser-only preflight for a JSONL batch file.
///
/// This intentionally performs no vault/model setup. It proves every non-blank
/// row can be parsed and semantically validated before ingest opens the vault or
/// initializes measurement state, so malformed input fails before side effects.
pub(super) fn validate_batch_file(path: &Path) -> CliResult<BatchValidation> {
    let file = std::fs::File::open(path)
        .map_err(|err| CliError::io(format!("open batch {}: {err}", path.display())))?;
    let reader = std::io::BufReader::new(file);
    let mut validation = BatchValidation {
        line_count: 0,
        row_count: 0,
    };
    for (index, line) in reader.lines().enumerate() {
        validation.line_count = index + 1;
        let line = line.map_err(|error| {
            if error.kind() == std::io::ErrorKind::InvalidData {
                invalid_batch(index + 1, format!("line is not valid UTF-8: {error}"))
            } else {
                CliError::io(format!("read batch line {}: {error}", index + 1))
            }
        })?;
        if parse_batch_line(index, &line)?.is_some() {
            validation.row_count += 1;
        }
    }
    Ok(validation)
}

fn invalid_batch(line: usize, detail: impl std::fmt::Display) -> CliError {
    CliError::from(CalyxError {
        code: CALYX_INGEST_BATCH_INVALID,
        message: format!("batch JSONL line {line} is invalid: {detail}"),
        remediation: INVALID_BATCH_REMEDIATION,
    })
}

fn validate_provenance(
    index: usize,
    metadata: Option<BTreeMap<String, String>>,
) -> CliResult<BTreeMap<String, String>> {
    let line = index + 1;
    let metadata = metadata.unwrap_or_default();
    if let Some(key) = metadata
        .keys()
        .find(|key| key.starts_with(STRUCTURED_METADATA_PREFIX))
    {
        return Err(CliError::from(CalyxError {
            code: CALYX_STRUCTURED_RESERVED_METADATA,
            message: format!(
                "batch JSONL line {line} metadata key {key:?} is reserved for canonical structured state"
            ),
            remediation: "remove calyx.structured.* user metadata and retry",
        }));
    }
    for required in ["source_dataset", "source_sha256", "license", "retrieval_ts"] {
        require_metadata_value(line, &metadata, required)?;
    }
    if !["source_url", "doi", "pmid", "pmcid"]
        .iter()
        .any(|key| has_metadata_value(&metadata, key))
    {
        return Err(CliError::usage(format!(
            "batch JSONL line {line} metadata requires one source locator: source_url, doi, pmid, or pmcid"
        )));
    }
    Ok(metadata)
}

fn require_metadata_value(
    line: usize,
    metadata: &BTreeMap<String, String>,
    key: &str,
) -> CliResult {
    if has_metadata_value(metadata, key) {
        Ok(())
    } else {
        Err(CliError::usage(format!(
            "batch JSONL line {line} metadata requires {key}"
        )))
    }
}

fn has_metadata_value(metadata: &BTreeMap<String, String>, key: &str) -> bool {
    metadata
        .get(key)
        .is_some_and(|value| !value.trim().is_empty())
}

/// Build a validated `Anchor` from a JSONL `AnchorSpec`, reusing the exact same
/// kind/value/confidence parsing the `calyx anchor` CLI uses, so an ingest-time
/// anchor is byte-identical to a post-hoc sealed one.
fn parse_anchor_spec(index: usize, spec: AnchorSpec) -> CliResult<Anchor> {
    let line = index + 1;
    let kind = parse_anchor_kind(&spec.kind)
        .map_err(|err| CliError::usage(format!("batch JSONL line {line} anchor kind: {err}")))?;
    let value = parse_anchor_value(&kind, &spec.kind, &spec.value)
        .map_err(|err| CliError::usage(format!("batch JSONL line {line} anchor value: {err}")))?;
    let confidence = spec.confidence.unwrap_or(1.0);
    validate_confidence(confidence)
        .map_err(|err| CliError::usage(format!("batch JSONL line {line} anchor: {err}")))?;
    Ok(Anchor {
        kind,
        value,
        source: spec
            .source
            .unwrap_or_else(|| DEFAULT_INGEST_ANCHOR_SOURCE.to_string()),
        observed_at: now_ms(),
        confidence,
    })
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
