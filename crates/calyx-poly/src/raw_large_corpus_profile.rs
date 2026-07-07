use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::raw_source_support::{sha256_hex, string_field, write_json};
use crate::{PolyError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LargeCorpusFieldProfile {
    pub dataset: String,
    pub source: String,
    pub record_count: usize,
    pub fields: Vec<LargeCorpusFieldStats>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LargeCorpusFieldStats {
    pub name: String,
    pub present_count: usize,
    pub missing_count: usize,
    pub null_count: usize,
    pub type_counts: BTreeMap<String, usize>,
    pub json_string_count: usize,
    pub array_min_len: Option<usize>,
    pub array_max_len: Option<usize>,
    pub example_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LargeCorpusJoinProfile {
    pub schema_version: String,
    pub record_count: usize,
    pub identifier_counts: BTreeMap<String, usize>,
    pub examples: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
pub(crate) struct CorpusRecord {
    pub dataset: String,
    pub source: String,
    pub value: Value,
}

pub(crate) fn build_field_profiles(records: &[CorpusRecord]) -> Vec<LargeCorpusFieldProfile> {
    let mut by_dataset: BTreeMap<(String, String), Vec<&Value>> = BTreeMap::new();
    for record in records {
        by_dataset
            .entry((record.dataset.clone(), record.source.clone()))
            .or_default()
            .push(&record.value);
    }
    by_dataset
        .into_iter()
        .map(|((dataset, source), values)| profile_dataset(dataset, source, values))
        .collect()
}

pub(crate) fn write_field_profiles(
    root: &Path,
    profiles: &[LargeCorpusFieldProfile],
) -> Result<Vec<String>> {
    let dir = root.join("field-profile");
    fs::create_dir_all(&dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_FIELD_PROFILE_DIR_CREATE_FAILED",
            format!("create field profile dir {}: {err}", dir.display()),
        )
    })?;
    let mut paths = Vec::new();
    for profile in profiles {
        let path = dir.join(format!("{}.json", profile.dataset));
        write_json(&path, profile)?;
        paths.push(path.display().to_string());
    }
    Ok(paths)
}

pub(crate) fn build_join_profile(records: &[CorpusRecord]) -> LargeCorpusJoinProfile {
    let mut counts = BTreeMap::new();
    let mut examples: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for record in records {
        collect_join_ids(&record.value, &mut counts, &mut examples);
    }
    LargeCorpusJoinProfile {
        schema_version: "poly.large_corpus.join_profile.v1".to_string(),
        record_count: records.len(),
        identifier_counts: counts,
        examples: examples
            .into_iter()
            .map(|(key, values)| (key, values.into_iter().take(5).collect()))
            .collect(),
    }
}

fn profile_dataset(
    dataset: String,
    source: String,
    values: Vec<&Value>,
) -> LargeCorpusFieldProfile {
    let mut field_names = BTreeSet::new();
    for value in &values {
        if let Some(map) = value.as_object() {
            field_names.extend(map.keys().cloned());
        }
    }
    let fields = field_names
        .into_iter()
        .map(|field| profile_field(&field, &values))
        .collect();
    LargeCorpusFieldProfile {
        dataset,
        source,
        record_count: values.len(),
        fields,
    }
}

fn profile_field(name: &str, values: &[&Value]) -> LargeCorpusFieldStats {
    let mut present_count = 0;
    let mut null_count = 0;
    let mut json_string_count = 0;
    let mut type_counts = BTreeMap::new();
    let mut array_min_len = None;
    let mut array_max_len = None;
    let mut example_sha256 = None;
    for value in values {
        let Some(field_value) = value.get(name) else {
            continue;
        };
        present_count += 1;
        if field_value.is_null() {
            null_count += 1;
        }
        if field_value
            .as_str()
            .is_some_and(|text| serde_json::from_str::<Value>(text).is_ok())
        {
            json_string_count += 1;
        }
        if let Some(items) = field_value.as_array() {
            array_min_len =
                Some(array_min_len.map_or(items.len(), |min: usize| min.min(items.len())));
            array_max_len =
                Some(array_max_len.map_or(items.len(), |max: usize| max.max(items.len())));
        }
        *type_counts
            .entry(json_type(field_value).to_string())
            .or_insert(0) += 1;
        if example_sha256.is_none() {
            example_sha256 = serde_json::to_vec(field_value)
                .ok()
                .map(|bytes| sha256_hex(&bytes));
        }
    }
    LargeCorpusFieldStats {
        name: name.to_string(),
        present_count,
        missing_count: values.len().saturating_sub(present_count),
        null_count,
        type_counts,
        json_string_count,
        array_min_len,
        array_max_len,
        example_sha256,
    }
}

fn collect_join_ids(
    value: &Value,
    counts: &mut BTreeMap<String, usize>,
    examples: &mut BTreeMap<String, BTreeSet<String>>,
) {
    if let Some(id) = string_field(value, "id") {
        add_id("id", id, counts, examples);
    }
    for (field, label) in [
        ("event_id", "event_id"),
        ("eventId", "event_id"),
        ("condition_id", "condition_id"),
        ("conditionId", "condition_id"),
        ("question_id", "question_id"),
        ("questionID", "question_id"),
        ("market", "market"),
        ("asset", "token_or_asset_id"),
        ("asset_id", "token_or_asset_id"),
        ("token", "token_or_asset_id"),
        ("token_id", "token_or_asset_id"),
        ("proxyWallet", "proxy_wallet"),
        ("transactionHash", "transaction_hash"),
        ("gameId", "game_id"),
    ] {
        if let Some(id) = string_field(value, field) {
            add_id(label, id, counts, examples);
        }
    }
    for field in ["clobTokenIds", "outcomes", "outcomePrices"] {
        if let Some(raw) = string_field(value, field)
            && let Ok(parsed) = serde_json::from_str::<Value>(&raw)
        {
            collect_join_ids(&parsed, counts, examples);
        }
    }
    match value {
        Value::Array(items) => {
            for item in items {
                collect_join_ids(item, counts, examples);
            }
        }
        Value::Object(map) => {
            for value in map.values() {
                collect_join_ids(value, counts, examples);
            }
        }
        _ => {}
    }
}

fn add_id(
    label: &str,
    value: String,
    counts: &mut BTreeMap<String, usize>,
    examples: &mut BTreeMap<String, BTreeSet<String>>,
) {
    if value.trim().is_empty() {
        return;
    }
    *counts.entry(label.to_string()).or_insert(0) += 1;
    examples.entry(label.to_string()).or_default().insert(value);
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
