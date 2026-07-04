use serde_json::Value;

use crate::error::{CliError, CliResult};

pub(super) fn candidate_id(issue: &str, idx: usize, row: &Value) -> String {
    str_field(row, "hypothesis_id")
        .or_else(|| str_field(row, "candidate_id"))
        .unwrap_or_else(|| format!("{issue}:row:{idx:06}"))
}

pub(super) fn display_names(row: &Value) -> Vec<String> {
    if let Some(names) = row.get("normalized_names").and_then(Value::as_array) {
        let values = names
            .iter()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if !values.is_empty() {
            return values;
        }
    }
    let mut values = Vec::new();
    for key in [
        "drug_name",
        "target_name",
        "disease_name",
        "gene",
        "variant",
        "therapies",
        "cancer_type",
    ] {
        collect_field_strings(row.get(key), &mut values);
    }
    if values.is_empty() {
        values.push(candidate_id("unknown", 0, row));
    }
    values
}

pub(super) fn source_class(row: &Value) -> String {
    str_field(row, "source_class")
        .or_else(|| str_field(row, "candidate_type"))
        .or_else(|| str_field(row, "novelty_class"))
        .unwrap_or_else(|| "unknown".to_string())
}

pub(super) fn falsification_summary(row: &Value) -> CliResult<String> {
    match row.get("falsification_status") {
        Some(Value::String(text)) => Ok(text.clone()),
        Some(value) => serde_json::to_string(value)
            .map_err(|err| CliError::runtime(format!("serialize falsification status: {err}"))),
        None => Ok(String::new()),
    }
}

pub(super) fn original_rank(row: &Value) -> Option<usize> {
    row.get("rank")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

pub(super) fn score_float(row: &Value) -> f64 {
    ["rank_score", "typed_score", "score"]
        .iter()
        .find_map(|key| row.get(*key).and_then(Value::as_f64))
        .unwrap_or(0.0)
}

pub(super) fn endpoint_type(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_object)
        .and_then(|obj| obj.get("type"))
        .and_then(Value::as_str)
        .map(|value| value.to_ascii_lowercase())
}

pub(super) fn max_named_score(row: &Value, ids: &[&str]) -> f64 {
    let mut best = 0.0;
    visit_objects(row, &mut |object| {
        let Some(id) = object.get("id").and_then(Value::as_str) else {
            return;
        };
        if ids.iter().any(|target| id.eq_ignore_ascii_case(target))
            && let Some(score) = object.get("score").and_then(Value::as_f64)
            && score > best
        {
            best = score;
        }
    });
    best
}

pub(super) fn has_dgidb_clinical_source(row: &Value) -> bool {
    let mut found = false;
    visit_objects(row, &mut |object| {
        let Some(values) = object.get("source_dbs").and_then(Value::as_array) else {
            return;
        };
        found |= values.iter().filter_map(Value::as_str).any(|source| {
            ["tdgclinicaltrial", "ttd", "pharmgkb"]
                .iter()
                .any(|needle| source.eq_ignore_ascii_case(needle))
        });
    });
    found
}

pub(super) fn all_text(value: &Value) -> String {
    let mut out = String::new();
    collect_text(value, &mut out);
    out
}

pub(super) fn str_field(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
}

fn collect_field_strings(value: Option<&Value>, out: &mut Vec<String>) {
    match value {
        Some(Value::String(text)) if !text.is_empty() => out.push(text.clone()),
        Some(Value::Array(values)) => {
            for value in values {
                if let Some(text) = value.as_str()
                    && !text.is_empty()
                {
                    out.push(text.to_string());
                }
            }
        }
        _ => {}
    }
}

fn visit_objects<F: FnMut(&serde_json::Map<String, Value>)>(value: &Value, f: &mut F) {
    match value {
        Value::Object(object) => {
            f(object);
            for value in object.values() {
                visit_objects(value, f);
            }
        }
        Value::Array(values) => {
            for value in values {
                visit_objects(value, f);
            }
        }
        _ => {}
    }
}

fn collect_text(value: &Value, out: &mut String) {
    match value {
        Value::String(text) => {
            out.push(' ');
            out.push_str(text);
        }
        Value::Object(object) => {
            for (key, value) in object {
                out.push(' ');
                out.push_str(key);
                collect_text(value, out);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_text(value, out);
            }
        }
        other if !other.is_null() => {
            out.push(' ');
            out.push_str(&other.to_string());
        }
        _ => {}
    }
}
