use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use super::model::{EvidenceGraphDraft, clean_label, concept_node, simple_node};
use crate::error::{CliError, CliResult};

mod clinicaltrials;
mod dgidb;
mod pubtator;
mod root;

pub(crate) use root::VerifiedRootReport;
use root::{RootIndex, normalize_rel, read_json_file, sha256_hex, verify_root};

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SourceLoadReport {
    pub roots: Vec<VerifiedRootReport>,
    pub parsed_row_counts: BTreeMap<String, usize>,
}

pub(crate) fn load_sources(
    pubtator_root: &Path,
    clinicaltrials_root: &Path,
    dgidb_root: &Path,
) -> CliResult<(EvidenceGraphDraft, SourceLoadReport)> {
    let mut draft = EvidenceGraphDraft::default();
    let mut report = SourceLoadReport {
        roots: Vec::new(),
        parsed_row_counts: BTreeMap::new(),
    };
    let pubtator = verify_root("pubtator_pubmed", pubtator_root, &mut draft, &mut report)?;
    pubtator::ingest(&pubtator, &mut draft, &mut report)?;
    let clinical = verify_root(
        "clinicaltrials",
        clinicaltrials_root,
        &mut draft,
        &mut report,
    )?;
    clinicaltrials::ingest(&clinical, &mut draft, &mut report)?;
    let dgidb = verify_root("dgidb", dgidb_root, &mut draft, &mut report)?;
    dgidb::ingest(&dgidb, &mut draft, &mut report)?;
    for path in [
        "pubtator_positive",
        "pubtator_negative",
        "clinicaltrials_positive",
        "clinicaltrials_negative",
        "clinicaltrials_outcome",
        "dgidb_positive",
        "dgidb_negative",
    ] {
        draft.require_path(path)?;
    }
    Ok((draft, report))
}

pub(super) fn ingest_generic_unhandled_jsonl(
    index: &RootIndex,
    draft: &mut EvidenceGraphDraft,
    report: &mut SourceLoadReport,
    handled: &BTreeSet<String>,
) -> CliResult {
    let rels: Vec<String> = index
        .artifacts
        .keys()
        .filter(|rel| rel.starts_with("parsed/") && rel.ends_with(".jsonl"))
        .filter(|rel| !handled.contains(*rel))
        .cloned()
        .collect();
    for rel in rels {
        for row in read_jsonl_rows(index, &rel, report)? {
            let row_key = add_row_node(index, draft, &rel, row.line, "source_row", &row)?;
            link_raw_path(index, draft, &row_key, &row.value)?;
            add_generic_inferred_edges(draft, &row_key, &row);
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(super) struct JsonRow {
    pub(super) line: usize,
    pub(super) value: Value,
    pub(super) sha256: String,
}

pub(super) fn read_jsonl_rows(
    index: &RootIndex,
    rel: &str,
    report: &mut SourceLoadReport,
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
        .insert(format!("{}:{rel}", index.family), rows.len());
    Ok(rows)
}

pub(super) fn add_row_node(
    index: &RootIndex,
    draft: &mut EvidenceGraphDraft,
    rel: &str,
    line: usize,
    row_type: &str,
    row: &JsonRow,
) -> CliResult<String> {
    draft.bump_source_row(&index.family);
    let mut metadata = BTreeMap::from([
        ("family".to_string(), index.family.clone()),
        ("source_file".to_string(), rel.to_string()),
        ("line".to_string(), line.to_string()),
        ("row_type".to_string(), row_type.to_string()),
        ("row_sha256".to_string(), row.sha256.clone()),
    ]);
    for field in [
        "seed_id",
        "source",
        "status",
        "reason",
        "raw_path",
        "nct_id",
        "pmid",
        "sourceDbName",
    ] {
        if let Some(value) = str_field(&row.value, field) {
            metadata.insert(field.to_string(), clean_label(&value));
        }
    }
    let row_key = draft.add_node(
        format!("source_row:{}:{rel}:{line}", index.family),
        row_type,
        format!("{} {rel}:{line}", index.family),
        metadata,
    );
    let artifact = index.artifacts.get(rel).ok_or_else(|| {
        CliError::runtime(format!(
            "{} source row file {rel} missing from persisted readback manifest",
            index.family
        ))
    })?;
    draft.add_edge(
        &row_key,
        "derived_from",
        &artifact.stable_key,
        BTreeMap::new(),
    );
    Ok(row_key)
}

fn add_generic_inferred_edges(draft: &mut EvidenceGraphDraft, row_key: &str, row: &JsonRow) {
    if let (Some(left_id), Some(right_id)) = (
        str_field(&row.value, "left_id"),
        str_field(&row.value, "right_id"),
    ) {
        let left = concept_node(draft, Some(&left_id), &left_id, "biomedical");
        let right = concept_node(draft, Some(&right_id), &right_id, "biomedical");
        draft.add_edge(
            &left,
            "evidence_row_subject",
            row_key,
            edge_meta(row, "generic"),
        );
        draft.add_edge(
            row_key,
            "evidence_row_object",
            &right,
            edge_meta(row, "generic"),
        );
    }
    if let (Some(drug), Some(gene)) = (str_field(&row.value, "drug"), str_field(&row.value, "gene"))
    {
        let drug = concept_node(draft, None, &drug, "drug");
        let gene = concept_node(draft, None, &gene, "gene");
        draft.add_edge(
            &drug,
            "evidence_row_subject",
            row_key,
            edge_meta(row, "generic"),
        );
        draft.add_edge(
            row_key,
            "evidence_row_object",
            &gene,
            edge_meta(row, "generic"),
        );
    }
    if let (Some(intervention), Some(condition)) = (
        str_field(&row.value, "intervention"),
        str_field(&row.value, "condition"),
    ) {
        let intervention = concept_node(draft, None, &intervention, "drug");
        let condition = concept_node(draft, None, &condition, "disease");
        draft.add_edge(
            &intervention,
            "evidence_row_subject",
            row_key,
            edge_meta(row, "generic"),
        );
        draft.add_edge(
            row_key,
            "evidence_row_object",
            &condition,
            edge_meta(row, "generic"),
        );
    }
    if let Some(pmid) = str_field(&row.value, "pmid") {
        let publication = simple_node(draft, "publication", "pmid", &pmid, &pmid);
        draft.add_edge(row_key, "published_in", &publication, BTreeMap::new());
    }
    if let Some(nct) = str_field(&row.value, "nct_id") {
        let trial = simple_node(draft, "trial", "nct", &nct, &nct);
        draft.add_edge(row_key, "observed_in_trial", &trial, BTreeMap::new());
    }
}

pub(super) fn link_raw_paths(
    index: &RootIndex,
    draft: &mut EvidenceGraphDraft,
    row_key: &str,
    value: &Value,
) -> CliResult {
    let Some(paths) = value.get("raw_paths").and_then(Value::as_object) else {
        return link_raw_path(index, draft, row_key, value);
    };
    for raw_path in paths.values().filter_map(Value::as_str) {
        link_artifact(index, draft, row_key, raw_path, None)?;
    }
    Ok(())
}

pub(super) fn link_raw_path(
    index: &RootIndex,
    draft: &mut EvidenceGraphDraft,
    row_key: &str,
    value: &Value,
) -> CliResult {
    let Some(raw_path) = str_field(value, "raw_path") else {
        return Ok(());
    };
    link_artifact(
        index,
        draft,
        row_key,
        &raw_path,
        str_field(value, "raw_sha256").as_deref(),
    )
}

fn link_artifact(
    index: &RootIndex,
    draft: &mut EvidenceGraphDraft,
    row_key: &str,
    rel: &str,
    expected_sha: Option<&str>,
) -> CliResult {
    let rel = normalize_rel(rel);
    let artifact = index.artifacts.get(&rel).ok_or_else(|| {
        CliError::runtime(format!(
            "{} referenced artifact {rel} is missing from persisted readback manifest",
            index.family
        ))
    })?;
    if expected_sha.is_some_and(|value| value != artifact.sha256) {
        return Err(CliError::runtime(format!(
            "{} referenced artifact {rel} hash mismatch: row expected {:?} manifest read {}",
            index.family, expected_sha, artifact.sha256
        )));
    }
    draft.add_edge(
        row_key,
        "derived_from",
        &artifact.stable_key,
        BTreeMap::from([
            ("relative_path".to_string(), artifact.rel.clone()),
            ("sha256".to_string(), artifact.sha256.clone()),
            ("bytes".to_string(), artifact.bytes.to_string()),
        ]),
    );
    Ok(())
}

pub(super) fn edge_meta(row: &JsonRow, family: &str) -> BTreeMap<String, String> {
    let mut meta = BTreeMap::from([
        ("family".to_string(), family.to_string()),
        ("row_sha256".to_string(), row.sha256.clone()),
        ("line".to_string(), row.line.to_string()),
    ]);
    for field in ["seed_id", "raw_path", "status", "source", "reason"] {
        if let Some(value) = str_field(&row.value, field) {
            meta.insert(field.to_string(), clean_label(&value));
        }
    }
    meta
}

pub(super) fn str_field(value: &Value, field: &str) -> Option<String> {
    match value.get(field)? {
        Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

pub(super) fn field_or(value: &Value, field: &str, fallback: &str) -> String {
    str_field(value, field).unwrap_or_else(|| fallback.to_string())
}

pub(super) fn bool_field(value: &Value, field: &str) -> Option<bool> {
    value.get(field).and_then(Value::as_bool)
}

pub(super) fn array_values<'a>(value: &'a Value, field: &str) -> Vec<&'a Value> {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

pub(super) fn array_strings(value: &Value, field: &str) -> Vec<String> {
    array_values(value, field)
        .into_iter()
        .filter_map(|item| match item {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
        .collect()
}
