use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Deserialize;

#[derive(Clone, Debug)]
pub(crate) struct LabelRows {
    pub(crate) labels: Vec<bool>,
    pub(crate) label_counts: BTreeMap<String, usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct SampleRows {
    pub(crate) indices: Vec<u64>,
    pub(crate) labels: Vec<bool>,
    pub(crate) groups: Vec<String>,
    pub(crate) positives: usize,
    pub(crate) negatives: usize,
}

impl LabelRows {
    pub(crate) fn load(path: &Path, target_class: usize) -> Result<Self, String> {
        if !path.is_file() {
            return Err(format!(
                "CALYX_FSV_ASSAY_I8BIN_CARD_NOT_FOUND: {}",
                path.display()
            ));
        }
        let file = File::open(path)
            .map_err(|error| format!("CALYX_FSV_ASSAY_I8BIN_CARD_NOT_FOUND: {error}"))?;
        let mut labels = Vec::new();
        let mut counts = BTreeMap::new();
        for (line_idx, line) in BufReader::new(file).lines().enumerate() {
            let line =
                line.map_err(|error| format!("CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_ROWS: {error}"))?;
            if line.trim().is_empty() {
                continue;
            }
            let row: RowJson = serde_json::from_str(&line).map_err(|error| {
                format!("CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_ROWS: line {line_idx}: {error}")
            })?;
            *counts.entry(row.label.to_string()).or_insert(0) += 1;
            labels.push(row.label == target_class);
        }
        if labels.is_empty() {
            return Err("CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_ROWS: no rows".to_string());
        }
        if labels.iter().all(|value| *value) || labels.iter().all(|value| !*value) {
            return Err(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_ROWS: target_class produces one class"
                    .to_string(),
            );
        }
        Ok(Self {
            labels,
            label_counts: counts,
        })
    }

    pub(crate) fn balanced_sample(&self, max_rows: usize) -> Result<SampleRows, String> {
        let positives = positions(&self.labels, true);
        let negatives = positions(&self.labels, false);
        if positives.is_empty() || negatives.is_empty() {
            return Err(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_ROWS: sample source lacks both classes"
                    .to_string(),
            );
        }
        let target_pos = (max_rows / 2).min(positives.len()).max(1);
        let target_neg = (max_rows - target_pos).min(negatives.len()).max(1);
        let mut indices = even_take(&positives, target_pos);
        indices.extend(even_take(&negatives, target_neg));
        indices.sort_unstable();
        let labels = indices
            .iter()
            .map(|&idx| self.labels[idx as usize])
            .collect::<Vec<_>>();
        let positives = labels.iter().filter(|value| **value).count();
        let negatives = labels.len().saturating_sub(positives);
        if positives == 0 || negatives == 0 {
            return Err(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_ROWS: selected sample is one class".to_string(),
            );
        }
        let groups = indices
            .iter()
            .map(|idx| format!("row_{idx}"))
            .collect::<Vec<_>>();
        Ok(SampleRows {
            indices,
            labels,
            groups,
            positives,
            negatives,
        })
    }
}

pub(crate) fn signature_indices(total_rows: usize, limit: Option<usize>) -> Vec<u64> {
    let want = limit.unwrap_or(total_rows).min(total_rows);
    even_take(&(0..total_rows as u64).collect::<Vec<_>>(), want)
}

fn positions(labels: &[bool], wanted: bool) -> Vec<u64> {
    labels
        .iter()
        .enumerate()
        .filter_map(|(idx, value)| (*value == wanted).then_some(idx as u64))
        .collect()
}

fn even_take(values: &[u64], n: usize) -> Vec<u64> {
    if n >= values.len() {
        return values.to_vec();
    }
    (0..n)
        .map(|idx| {
            let pos = idx.saturating_mul(values.len()) / n;
            values[pos.min(values.len() - 1)]
        })
        .collect()
}

#[derive(Deserialize)]
struct RowJson {
    label: usize,
}
