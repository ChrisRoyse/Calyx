use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use crate::Result;
use crate::raw_source_support::write_json;
use crate::raw_sources::{RawEndpointSample, RawRuntimeSemanticsObservation, RawSourceFailure};

pub(crate) fn clob_post_runtime_semantics(
    samples: &[RawEndpointSample],
    parsed_bodies: &BTreeMap<String, Value>,
) -> Vec<RawRuntimeSemanticsObservation> {
    samples
        .iter()
        .filter_map(|sample| {
            let value = parsed_bodies.get(&sample.name);
            match sample.name.as_str() {
                "edge_clob_post_prices_missing_side_runtime_semantics" => Some(observation(
                    sample,
                    value,
                    "POST /prices array item omits side",
                    "HTTP 200 token map defaults the missing side to SELL",
                    sample.status_code == Some(200) && object_map_has_nested_key(value, "SELL"),
                    "POLY_RAW_SOURCE_CLOB_POST_MISSING_SIDE_SEMANTICS_CHANGED",
                    "Normalized price ingestion must not require side to be present in raw POST /prices responses; missing side is a live SELL-default semantics.",
                )),
                "edge_clob_post_prices_invalid_side_runtime_semantics" => Some(observation(
                    sample,
                    value,
                    "POST /prices array item uses invalid side HOLD",
                    "HTTP 200 empty JSON object",
                    sample.status_code == Some(200) && is_empty_object(value),
                    "POLY_RAW_SOURCE_CLOB_POST_INVALID_SIDE_SEMANTICS_CHANGED",
                    "Invalid side is an empty-result semantics, not necessarily an HTTP failure.",
                )),
                "edge_clob_post_prices_invalid_token_runtime_semantics" => Some(observation(
                    sample,
                    value,
                    "POST /prices array item uses an invalid token id",
                    "HTTP 200 empty JSON object",
                    sample.status_code == Some(200) && is_empty_object(value),
                    "POLY_RAW_SOURCE_CLOB_POST_INVALID_TOKEN_SEMANTICS_CHANGED",
                    "Invalid token is an empty-result semantics for this endpoint, not necessarily an HTTP failure.",
                )),
                "edge_clob_post_batch_history_21_duplicate_markets_runtime_semantics" => {
                    Some(observation(
                        sample,
                        value,
                        "POST /batch-prices-history sends 21 duplicate market entries",
                        "HTTP 200 response with history field",
                        sample.status_code == Some(200) && object_has_key(value, "history"),
                        "POLY_RAW_SOURCE_CLOB_POST_DUPLICATE_LIMIT_SEMANTICS_CHANGED",
                        "The max-20 boundary must be tested with unique markets; duplicates do not prove the documented limit.",
                    ))
                }
                "edge_clob_post_batch_history_21_unique_markets_runtime_semantics" => {
                    Some(observation(
                        sample,
                        value,
                        "POST /batch-prices-history sends 21 unique Gamma-derived market tokens",
                        "HTTP 400 error stating markets exceeds maximum of 20",
                        sample.status_code == Some(400)
                            && error_contains(value, "exceeds maximum of 20"),
                        "POLY_RAW_SOURCE_CLOB_POST_UNIQUE_LIMIT_SEMANTICS_CHANGED",
                        "The batch-history limit is enforced for unique market entries and must be modeled before batching requests.",
                    ))
                }
                _ => None,
            }
        })
        .collect()
}

pub(crate) fn runtime_semantics_failure(
    observations: &[RawRuntimeSemanticsObservation],
) -> Option<RawSourceFailure> {
    observations.iter().find(|item| !item.semantics_match).map(|item| {
        RawSourceFailure {
            code: item.failure_code.clone().unwrap_or_else(|| {
                "POLY_RAW_SOURCE_RUNTIME_SEMANTICS_MISMATCH".to_string()
            }),
            message: format!(
                "{} did not match expected runtime semantics: {}; actual_status={:?}; actual_body_shape={}",
                item.sample_name,
                item.expected_runtime_semantics,
                item.actual_status_code,
                item.actual_body_shape
            ),
            sample_name: Some(item.sample_name.clone()),
        }
    })
}

pub(crate) fn write_runtime_semantics_artifacts(
    root: &Path,
    observations: &[RawRuntimeSemanticsObservation],
) -> Result<()> {
    if observations.is_empty() {
        return Ok(());
    }
    write_json(
        &root
            .join("schema-observations")
            .join("clob-post-runtime-semantics.json"),
        &observations,
    )
}

fn observation(
    sample: &RawEndpointSample,
    value: Option<&Value>,
    request_case: impl Into<String>,
    expected_runtime_semantics: impl Into<String>,
    semantics_match: bool,
    failure_code: impl Into<String>,
    schema_implication: impl Into<String>,
) -> RawRuntimeSemanticsObservation {
    RawRuntimeSemanticsObservation {
        sample_name: sample.name.clone(),
        source: sample.source.clone(),
        endpoint: sample.endpoint.clone(),
        method: sample.method.clone(),
        docs_url: sample.docs_url.clone(),
        request_case: request_case.into(),
        expected_runtime_semantics: expected_runtime_semantics.into(),
        actual_status_code: sample.status_code,
        actual_body_shape: body_shape(value, sample),
        actual_body_fields: body_fields(value),
        semantics_match,
        failure_code: (!semantics_match).then(|| failure_code.into()),
        schema_implication: schema_implication.into(),
        request_body_path: sample.request_body_path.clone(),
        request_body_sha256: sample.request_body_sha256.clone(),
        body_path: sample.body_path.clone(),
        metadata_path: sample.metadata_path.clone(),
        body_bytes: sample.body_bytes,
        body_sha256: sample.body_sha256.clone(),
        before: sample.before.clone(),
        after: sample.after.clone(),
    }
}

fn body_shape(value: Option<&Value>, sample: &RawEndpointSample) -> String {
    match value {
        Some(Value::Object(map)) if map.is_empty() => "empty_object".to_string(),
        Some(Value::Object(map)) if map.values().all(Value::is_object) => {
            "object_map_to_objects".to_string()
        }
        Some(Value::Object(_)) => "object".to_string(),
        Some(Value::Array(items)) => format!("array_len_{}", items.len()),
        Some(Value::Null) => "null".to_string(),
        Some(Value::Bool(_)) => "bool".to_string(),
        Some(Value::Number(_)) => "number".to_string(),
        Some(Value::String(_)) => "string".to_string(),
        None if sample.body_exists => "non_json_or_unparsed".to_string(),
        None => "absent".to_string(),
    }
}

fn body_fields(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_object)
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default()
}

fn object_map_has_nested_key(value: Option<&Value>, key: &str) -> bool {
    value
        .and_then(Value::as_object)
        .is_some_and(|map| map.values().any(|item| object_has_key(Some(item), key)))
}

fn object_has_key(value: Option<&Value>, key: &str) -> bool {
    value
        .and_then(Value::as_object)
        .is_some_and(|map| map.contains_key(key))
}

fn is_empty_object(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_object)
        .is_some_and(serde_json::Map::is_empty)
}

fn error_contains(value: Option<&Value>, expected: &str) -> bool {
    value
        .and_then(Value::as_object)
        .and_then(|map| map.get("error"))
        .and_then(Value::as_str)
        .is_some_and(|message| message.contains(expected))
}
