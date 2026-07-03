use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};

use serde_json::Value;

use super::root::{RootIndex, artifact, artifact_meta, sha256_hex};
use super::{FAMILY, LincsSourceReport};
use crate::cmd::lincs_reversal::model::{LincsGraphDraft, clean_label};
use crate::error::{CliError, CliResult};

#[derive(Clone, Debug)]
pub(super) struct JsonRow {
    pub(super) line: usize,
    pub(super) value: Value,
    pub(super) sha256: String,
}

pub(super) fn read_jsonl_rows(
    index: &RootIndex,
    rel: &str,
    report: &mut LincsSourceReport,
) -> CliResult<Vec<JsonRow>> {
    let path = index.root.join(rel);
    let file = fs::File::open(&path)?;
    let reader = BufReader::new(file);
    let mut rows = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&line).map_err(|error| {
            CliError::runtime(format!(
                "parse {} line {} as JSON: {error}",
                path.display(),
                idx + 1
            ))
        })?;
        rows.push(JsonRow {
            line: idx + 1,
            sha256: sha256_hex(line.as_bytes()),
            value,
        });
    }
    report
        .parsed_row_counts
        .insert(format!("{FAMILY}:{rel}"), rows.len());
    Ok(rows)
}

pub(super) fn add_row_node(
    index: &RootIndex,
    draft: &mut LincsGraphDraft,
    rel: &str,
    row_type: &str,
    row: &JsonRow,
) -> CliResult<String> {
    draft.bump_source_row(FAMILY);
    let mut metadata = BTreeMap::from([
        ("family".to_string(), FAMILY.to_string()),
        ("source_file".to_string(), rel.to_string()),
        ("line".to_string(), row.line.to_string()),
        ("row_type".to_string(), row_type.to_string()),
        ("row_sha256".to_string(), row.sha256.clone()),
    ]);
    for field in [
        "signature_id",
        "disease_name",
        "geo_id",
        "pert_id",
        "pert_desc",
        "candidate_drug",
        "reason",
    ] {
        if let Some(value) = str_field(&row.value, field) {
            metadata.insert(field.to_string(), clean_label(&value));
        }
    }
    let row_key = draft.add_node(
        format!("source_row:{FAMILY}:{rel}:{}", row.line),
        row_type,
        format!("{FAMILY} {rel}:{}", row.line),
        metadata,
    );
    let artifact = artifact(index, rel)?;
    draft.add_edge(
        &row_key,
        "derived_from",
        &artifact.stable_key,
        artifact_meta(artifact),
    );
    Ok(row_key)
}

pub(super) fn row_meta(row: &JsonRow) -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::from([
        ("family".to_string(), FAMILY.to_string()),
        ("row_sha256".to_string(), row.sha256.clone()),
        ("line".to_string(), row.line.to_string()),
    ]);
    for field in ["signature_id", "rank", "score", "reason"] {
        if let Some(value) = str_field(&row.value, field) {
            metadata.insert(field.to_string(), clean_label(&value));
        }
    }
    metadata
}

pub(super) fn fields(value: &Value, names: &[&str]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for name in names {
        if let Some(value) = str_field(value, name) {
            out.insert((*name).to_string(), clean_label(&value));
        }
    }
    out
}

pub(super) fn required_field(value: &Value, field: &str) -> CliResult<String> {
    str_field(value, field).ok_or_else(|| CliError::runtime(format!("LINCS row missing {field}")))
}

pub(super) fn str_field(value: &Value, field: &str) -> Option<String> {
    match value.get(field)? {
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

pub(super) fn nested_array_len(value: &Value, object: &str, field: &str) -> usize {
    value
        .get(object)
        .and_then(|inner| inner.get(field))
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}
