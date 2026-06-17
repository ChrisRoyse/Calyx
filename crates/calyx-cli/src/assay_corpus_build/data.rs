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
    id: String,
    #[serde(default)]
    split: String,
    text: String,
    label: usize,
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
        if let Some(limit) = request.limit_per_class {
            let count = counts.get(&raw.label).copied().unwrap_or(0);
            if count >= limit {
                continue;
            }
        }
        *counts.entry(raw.label).or_insert(0) += 1;
        rows.push(LabeledRow {
            id: raw.id,
            split: if raw.split.trim().is_empty() {
                "unspecified".to_string()
            } else {
                raw.split
            },
            text: raw.text,
            label: raw.label,
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
    if row.id.trim().is_empty() {
        return Err(format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_ROW: line {line_idx} id is empty"
        ));
    }
    if row.text.trim().is_empty() {
        return Err(format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_ROW: line {line_idx} text is empty"
        ));
    }
    Ok(())
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
