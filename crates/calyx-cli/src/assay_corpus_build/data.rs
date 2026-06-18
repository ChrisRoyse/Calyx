use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use serde::{Deserialize, Serialize};

use super::request::CorpusBuildRequest;

const MIN_ROWS: usize = 50;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LabeledRow {
    pub(crate) id: String,
    pub(crate) split: String,
    pub(crate) text: String,
    pub(crate) label: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct BuildRows {
    pub(crate) rows: Vec<LabeledRow>,
    pub(crate) label_counts: BTreeMap<String, usize>,
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
    text: String,
    label: RawLabel,
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
        if let Some(limit) = request.limit_per_class {
            let count = counts.get(&label).copied().unwrap_or(0);
            if count >= limit {
                continue;
            }
        }
        *counts.entry(label).or_insert(0) += 1;
        rows.push(LabeledRow {
            id,
            split: if raw.split.trim().is_empty() {
                "unspecified".to_string()
            } else {
                raw.split
            },
            text: raw.text,
            label,
        });
    }
    validate_loaded_rows(request, &rows)?;
    let label_counts = counts
        .into_iter()
        .map(|(label, count)| (label.to_string(), count))
        .collect();
    Ok(BuildRows { rows, label_counts })
}

fn validate_row(line_idx: usize, row: &RawRow) -> Result<(), String> {
    if row.text.trim().is_empty() {
        return Err(format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_ROW: line {line_idx} text is empty"
        ));
    }
    Ok(())
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
