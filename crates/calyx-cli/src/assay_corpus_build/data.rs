use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{
    Input, Modality, TEMPORAL_LANE_ACTIVE, TEMPORAL_LANE_INACTIVE, TEMPORAL_MISSING_CREATED_AT,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::assay_anchor_audit::AnchorAudit;
use crate::migrate::temporal::parse_event_time_secs;

use super::request::CorpusBuildRequest;

const MIN_ROWS: usize = 50;
const SOURCE_SEQUENCE: &str = "jsonl_line";

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LabeledRow {
    pub(crate) id: String,
    pub(crate) split: String,
    pub(crate) text: String,
    pub(crate) input_bytes: Vec<u8>,
    pub(crate) input_pointer: String,
    pub(crate) label: usize,
    pub(crate) event_time_secs: Option<i64>,
    pub(crate) event_time_raw: Option<String>,
    pub(crate) temporal_lane_state: String,
    pub(crate) temporal_inactive_reason: Option<String>,
    pub(crate) source_sequence: String,
    pub(crate) source_sequence_index: usize,
    pub(crate) anchor_audit: AnchorAudit,
}

#[derive(Clone, Debug)]
pub(crate) struct BuildRows {
    pub(crate) rows: Vec<LabeledRow>,
    pub(crate) label_counts: BTreeMap<String, usize>,
    pub(crate) anchor_audit: AnchorAudit,
}

#[derive(Deserialize)]
struct RawRow {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    row: Option<usize>,
    #[serde(default)]
    split: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    input_path: Option<PathBuf>,
    label: RawLabel,
    #[serde(default)]
    event_time: Option<Value>,
    #[serde(default)]
    event_time_secs: Option<Value>,
    #[serde(default)]
    source_event_time_secs: Option<Value>,
    #[serde(default)]
    created_at: Option<Value>,
    #[serde(default)]
    timestamp: Option<Value>,
    #[serde(default)]
    anchor_audit: Option<AnchorAudit>,
    #[serde(default)]
    anchor_leaks_into_input: Option<bool>,
    #[serde(default)]
    trivial_anchor: Option<bool>,
    #[serde(default)]
    grounded_gate_eligible: Option<bool>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawLabel {
    Number(usize),
    String(String),
}

pub(crate) fn load_rows(request: &CorpusBuildRequest) -> Result<BuildRows, String> {
    let text = fs::read_to_string(&request.rows_jsonl).map_err(|error| {
        format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_ROW_IO: {}: {error}",
            request.rows_jsonl.display()
        )
    })?;
    let mut rows = Vec::new();
    let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
    let mut row_audits = Vec::new();
    for (line_idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let raw: RawRow = serde_json::from_str(line).map_err(|error| {
            format!("CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_ROW: line {line_idx}: {error}")
        })?;
        validate_row(line_idx, &raw)?;
        let id = row_id(line_idx, &raw)?;
        let label = row_label(line_idx, &raw.label)?;
        let input = row_input(line_idx, &raw, &request.rows_jsonl)?;
        if let Some(limit) = request.limit_per_class {
            let count = counts.get(&label).copied().unwrap_or(0);
            if count >= limit {
                continue;
            }
        }
        *counts.entry(label).or_insert(0) += 1;
        let temporal = row_temporal(line_idx, &raw)?;
        let anchor_audit = AnchorAudit::from_parts(
            raw.anchor_audit.clone(),
            raw.anchor_leaks_into_input,
            raw.trivial_anchor,
            raw.grounded_gate_eligible,
        );
        row_audits.push(anchor_audit.clone());
        rows.push(LabeledRow {
            id,
            split: if raw.split.trim().is_empty() {
                "unspecified".to_string()
            } else {
                raw.split
            },
            text: input.text,
            input_bytes: input.bytes,
            input_pointer: input.pointer,
            label,
            event_time_secs: temporal.event_time_secs,
            event_time_raw: temporal.event_time_raw,
            temporal_lane_state: temporal.lane_state,
            temporal_inactive_reason: temporal.inactive_reason,
            source_sequence: SOURCE_SEQUENCE.to_string(),
            source_sequence_index: line_idx,
            anchor_audit,
        });
    }
    validate_loaded_rows(request, &rows)?;
    let label_counts = counts
        .into_iter()
        .map(|(label, count)| (label.to_string(), count))
        .collect();
    Ok(BuildRows {
        rows,
        label_counts,
        anchor_audit: AnchorAudit::merge_rows(&row_audits),
    })
}

struct RowTemporal {
    event_time_secs: Option<i64>,
    event_time_raw: Option<String>,
    lane_state: String,
    inactive_reason: Option<String>,
}

fn row_temporal(line_idx: usize, row: &RawRow) -> Result<RowTemporal, String> {
    let Some((field, value)) = temporal_candidate(row) else {
        return Ok(inactive_temporal());
    };
    if value.is_null() {
        return Ok(inactive_temporal());
    }
    let raw = temporal_raw_text(line_idx, field, value)?;
    let secs = parse_event_time_secs(&raw, line_idx as u64, field).map_err(|error| {
        format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_TIMESTAMP: line {line_idx} {}",
            error.message()
        )
    })?;
    let event_time_secs = i64::try_from(secs).map_err(|_| {
        format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_TIMESTAMP: line {line_idx} {field} exceeds i64"
        )
    })?;
    Ok(RowTemporal {
        event_time_secs: Some(event_time_secs),
        event_time_raw: Some(raw),
        lane_state: TEMPORAL_LANE_ACTIVE.to_string(),
        inactive_reason: None,
    })
}

fn inactive_temporal() -> RowTemporal {
    RowTemporal {
        event_time_secs: None,
        event_time_raw: None,
        lane_state: TEMPORAL_LANE_INACTIVE.to_string(),
        inactive_reason: Some(TEMPORAL_MISSING_CREATED_AT.to_string()),
    }
}

impl LabeledRow {
    pub(crate) fn input_for(&self, modality: Modality) -> Input {
        Input::new(modality, self.input_bytes.clone()).with_pointer(self.input_pointer.clone())
    }
}

struct RowInput {
    bytes: Vec<u8>,
    pointer: String,
    text: String,
}

fn temporal_candidate(row: &RawRow) -> Option<(&'static str, &Value)> {
    [
        ("event_time", row.event_time.as_ref()),
        ("event_time_secs", row.event_time_secs.as_ref()),
        (
            "source_event_time_secs",
            row.source_event_time_secs.as_ref(),
        ),
        ("created_at", row.created_at.as_ref()),
        ("timestamp", row.timestamp.as_ref()),
    ]
    .into_iter()
    .find_map(|(field, value)| value.map(|value| (field, value)))
}

fn temporal_raw_text(line_idx: usize, field: &str, value: &Value) -> Result<String, String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => number
            .as_u64()
            .map(|value| value.to_string())
            .ok_or_else(|| invalid_temporal_type(line_idx, field)),
        _ => Err(invalid_temporal_type(line_idx, field)),
    }
}

fn invalid_temporal_type(line_idx: usize, field: &str) -> String {
    format!(
        "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_TIMESTAMP: line {line_idx} {field} must be a Unix timestamp integer or timestamp string"
    )
}

fn validate_row(line_idx: usize, row: &RawRow) -> Result<(), String> {
    let has_text = row
        .text
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty());
    let has_path = row
        .input_path
        .as_ref()
        .is_some_and(|path| !path.as_os_str().is_empty());
    if has_text == has_path {
        return Err(format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_ROW: line {line_idx} requires exactly one of text or input_path"
        ));
    }
    Ok(())
}

fn row_input(line_idx: usize, row: &RawRow, rows_jsonl: &Path) -> Result<RowInput, String> {
    if let Some(text) = row.text.as_deref().filter(|value| !value.trim().is_empty()) {
        return Ok(RowInput {
            bytes: text.as_bytes().to_vec(),
            pointer: format!("jsonl://line/{line_idx}#field=text"),
            text: text.to_string(),
        });
    }
    let path = row.input_path.as_ref().ok_or_else(|| {
        format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_ROW: line {line_idx} requires text or input_path"
        )
    })?;
    let resolved = resolve_input_path(rows_jsonl, path);
    let bytes = fs::read(&resolved).map_err(|error| {
        format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_INPUT_IO: line {line_idx} read {} failed: {error}",
            resolved.display()
        )
    })?;
    if bytes.is_empty() {
        return Err(format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_ROW: line {line_idx} input_path {} is empty",
            resolved.display()
        ));
    }
    Ok(RowInput {
        bytes,
        pointer: format!("file://{}", resolved.display()),
        text: String::new(),
    })
}

fn resolve_input_path(rows_jsonl: &Path, input_path: &Path) -> PathBuf {
    if input_path.is_absolute() {
        return input_path.to_path_buf();
    }
    rows_jsonl
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .join(input_path)
}

fn row_id(line_idx: usize, row: &RawRow) -> Result<String, String> {
    row.id
        .as_deref()
        .or(row.source.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| row.row.map(|idx| format!("row:{idx}")))
        .ok_or_else(|| {
            format!(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_ROW: line {line_idx} requires id, source, or row"
            )
        })
}

fn row_label(line_idx: usize, label: &RawLabel) -> Result<usize, String> {
    match label {
        RawLabel::Number(value) => Ok(*value),
        RawLabel::String(value) => value.trim().parse::<usize>().map_err(|error| {
            format!(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_ROW: line {line_idx} label must be usize: {error}"
            )
        }),
    }
}

fn validate_loaded_rows(request: &CorpusBuildRequest, rows: &[LabeledRow]) -> Result<(), String> {
    if rows.len() < MIN_ROWS {
        return Err(format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_ROWS: need >={MIN_ROWS} rows, got {}",
            rows.len()
        ));
    }
    let labels: BTreeSet<usize> = rows.iter().map(|row| row.label).collect();
    if labels.len() < 2 {
        return Err(format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_ROWS: need at least two labels, got {}",
            labels.len()
        ));
    }
    let positives = rows
        .iter()
        .filter(|row| row.label == request.target_class)
        .count();
    if positives == 0 || positives == rows.len() {
        return Err(format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_ROWS: target_class={} positives={positives} total={}",
            request.target_class,
            rows.len()
        ));
    }
    Ok(())
}
