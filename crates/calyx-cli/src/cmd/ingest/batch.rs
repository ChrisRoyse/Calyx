use std::collections::BTreeMap;
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::path::Path;

use calyx_core::Anchor;
use serde::Deserialize;

use super::super::vault::now_ms;
use super::anchor::{parse_anchor_kind, parse_anchor_value};
use super::oracle_event::{OracleEvent, OracleEventSpec, parse_oracle_event};
use super::parse::{validate_confidence, validate_text};
use crate::error::{CliError, CliResult};

/// Default source recorded on an anchor threaded in at ingest time, when the
/// JSONL line does not name its own `source`. Distinguishes ingest-time grounding
/// from a post-hoc `calyx anchor` seal (`calyx-cli`).
const DEFAULT_INGEST_ANCHOR_SOURCE: &str = "calyx-ingest";

/// One typed anchor on a batch JSONL line. The feeder/corpus-builder decides what
/// to attach (e.g. the QA correct-answer as `label:answer`, the source domain as
/// `label:dataset`, or `test-pass` for verified-correct rows); the ingest engine
/// stays domain-agnostic and only validates + threads them onto the constellation.
#[derive(Deserialize)]
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
struct BatchLine {
    text: String,
    /// Per-record source provenance (source_url, doi, pmid, license, ...). Stored
    /// verbatim on the constellation metadata map; survives raw-source deletion.
    #[serde(default)]
    metadata: BTreeMap<String, String>,
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
pub(super) type BatchRow = (
    String,
    BTreeMap<String, String>,
    Vec<Anchor>,
    Option<OracleEvent>,
);

/// Parse one batch JSONL line; `None` for a blank line.
///
/// A malformed anchor (unknown kind, unparseable value, out-of-range confidence)
/// is a hard error that names the line — anchors are grounding truth, so a bad one
/// must fail loudly rather than be silently dropped (doctrine: no silent fallback).
pub(super) fn parse_batch_line(index: usize, line: &str) -> CliResult<Option<BatchRow>> {
    if line.trim().is_empty() {
        return Ok(None);
    }
    let parsed: BatchLine = serde_json::from_str(line)
        .map_err(|err| CliError::io(format!("batch JSONL line {} is invalid: {err}", index + 1)))?;
    validate_text(&parsed.text)?;
    let mut anchors = Vec::with_capacity(parsed.anchors.len());
    for spec in parsed.anchors {
        anchors.push(parse_anchor_spec(index, spec)?);
    }
    let oracle = parsed
        .oracle
        .map(|spec| parse_oracle_event(index, spec))
        .transpose()?;
    Ok(Some((parsed.text, parsed.metadata, anchors, oracle)))
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
