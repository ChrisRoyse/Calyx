use std::path::Path;

use serde_json::Value;

use crate::error::{CliError, CliResult};

use super::model::Candidate;

pub(super) fn collect_named_entities(row: &Value, key: &str, out: &mut Vec<String>) {
    match row.get(key) {
        Some(Value::String(text)) => push_unique(out, text.clone()),
        Some(Value::Array(values)) => {
            for value in values {
                match value {
                    Value::String(text) => push_unique(out, text.clone()),
                    Value::Object(map) => {
                        for nested in ["name", "normalized_name", "symbol", "term"] {
                            if let Some(Value::String(text)) = map.get(nested) {
                                push_unique(out, text.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Some(Value::Object(map)) => {
            for nested in ["name", "normalized_name", "symbol", "term"] {
                if let Some(Value::String(text)) = map.get(nested) {
                    push_unique(out, text.clone());
                }
            }
        }
        _ => {}
    }
}

pub(super) fn first_text(row: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(text) = nonempty(row, key) {
            return text;
        }
    }
    String::new()
}

pub(super) fn nonempty(row: &Value, key: &str) -> Option<String> {
    let text = str_field(row, key);
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

pub(super) fn str_field(row: &Value, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

pub(super) fn string_array(row: &Value, key: &str) -> Vec<String> {
    row.get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn number_field(row: &Value, key: &str) -> Option<f64> {
    row.get(key).and_then(|raw| {
        raw.as_f64()
            .or_else(|| raw.as_u64().map(|value| value as f64))
            .or_else(|| raw.as_i64().map(|value| value as f64))
            .or_else(|| raw.as_str().and_then(|text| text.parse::<f64>().ok()))
    })
}

pub(super) fn u64_field(row: &Value, key: &str) -> Option<u64> {
    row.get(key).and_then(|raw| {
        raw.as_u64()
            .or_else(|| raw.as_i64().and_then(|value| u64::try_from(value).ok()))
            .or_else(|| raw.as_str().and_then(|text| text.parse::<u64>().ok()))
    })
}

pub(super) fn bool_field(row: &Value, key: &str) -> Option<bool> {
    row.get(key).and_then(|raw| {
        raw.as_bool().or_else(|| {
            raw.as_str().and_then(|text| match lower(text).as_str() {
                "true" | "yes" | "1" => Some(true),
                "false" | "no" | "0" => Some(false),
                _ => None,
            })
        })
    })
}

pub(super) fn source_key(row: &Value) -> String {
    let drugs = string_or_array_key(row, &["drug_name", "drug", "intervention"]);
    let targets = string_or_array_key(row, &["target_name", "gene", "target"]);
    let diseases = string_or_array_key(row, &["disease_name", "disease", "condition"]);
    key_from_parts(&drugs, &targets, &diseases)
}

pub(super) fn candidate_key(candidate: &Candidate) -> String {
    key_from_parts(
        &candidate.drug_names,
        &candidate.target_names,
        &candidate.disease_names,
    )
}

pub(super) fn key_from_parts(drugs: &[String], targets: &[String], diseases: &[String]) -> String {
    let mut parts = Vec::new();
    parts.extend(
        drugs
            .iter()
            .map(|value| format!("drug:{}", norm_key(value))),
    );
    parts.extend(
        targets
            .iter()
            .map(|value| format!("target:{}", norm_key(value))),
    );
    parts.extend(
        diseases
            .iter()
            .map(|value| format!("disease:{}", norm_key(value))),
    );
    parts.retain(|value| !value.ends_with(':'));
    parts.sort();
    parts.dedup();
    parts.join("|")
}

pub(super) fn string_or_array_key(row: &Value, keys: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for key in keys {
        collect_named_entities(row, key, &mut out);
    }
    uniq(out)
}

pub(super) fn is_drug_type(value: &str) -> bool {
    matches!(value, "chemical" | "drug" | "intervention" | "compound")
}

pub(super) fn is_target_type(value: &str) -> bool {
    matches!(value, "gene" | "gene_protein" | "target" | "protein")
}

pub(super) fn is_generic_mechanism_class(value: &str) -> bool {
    contains_any(
        value,
        &[
            "hdac inhibitor",
            "cdk inhibitor",
            "hsp90 inhibitor",
            "mek inhibitor",
            "broad kinase",
            "generic mechanism",
            "mechanism class only",
        ],
    )
}

pub(super) fn should_use_typed_name(name: &str, explicit_values: &[String]) -> bool {
    explicit_values.is_empty() || !contains_any(&lower(name), &[" class", "class ", " class "])
}

pub(super) fn pearson(pairs: &[(f64, f64)]) -> Option<f64> {
    if pairs.len() < 2 {
        return None;
    }
    let n = pairs.len() as f64;
    let mean_x = pairs.iter().map(|(x, _)| x).sum::<f64>() / n;
    let mean_y = pairs.iter().map(|(_, y)| y).sum::<f64>() / n;
    let mut numerator = 0.0;
    let mut denom_x = 0.0;
    let mut denom_y = 0.0;
    for (x, y) in pairs {
        let dx = x - mean_x;
        let dy = y - mean_y;
        numerator += dx * dy;
        denom_x += dx * dx;
        denom_y += dy * dy;
    }
    let denom = (denom_x * denom_y).sqrt();
    if denom == 0.0 {
        None
    } else {
        Some((numerator / denom * 1_000_000.0).round() / 1_000_000.0)
    }
}

pub(super) fn parse_u64(raw: &str, flag: &str, min: u64) -> CliResult<u64> {
    let value = raw
        .parse::<u64>()
        .map_err(|error| CliError::usage(format!("parse {flag} {raw}: {error}")))?;
    if value < min {
        return Err(CliError::usage(format!("{flag} must be >= {min}")));
    }
    Ok(value)
}

pub(super) fn parse_unit(raw: &str) -> CliResult<f64> {
    let value = raw
        .parse::<f64>()
        .map_err(|error| CliError::usage(format!("parse unit threshold {raw}: {error}")))?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(CliError::usage("thresholds must be finite and in [0,1]"));
    }
    Ok(value)
}

pub(super) fn require(ok: bool, flag: &str) -> CliResult {
    if ok {
        Ok(())
    } else {
        Err(CliError::usage(format!(
            "biomedical-blindspot-audit requires {flag}"
        )))
    }
}

pub(super) fn require_path(path: &Path, flag: &str) -> CliResult {
    if path.as_os_str().is_empty() {
        Err(CliError::usage(format!(
            "biomedical-blindspot-audit requires {flag} <path>"
        )))
    } else {
        Ok(())
    }
}

pub(super) fn lower(value: &str) -> String {
    value.to_ascii_lowercase()
}

pub(super) fn norm_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(super) fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

pub(super) fn push_unique(out: &mut Vec<String>, value: String) {
    let text = value.trim();
    if !text.is_empty()
        && !out
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(text))
    {
        out.push(text.to_string());
    }
}

pub(super) fn uniq(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        push_unique(&mut out, value);
    }
    out
}
