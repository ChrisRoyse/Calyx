use crate::raw_source_readme::inventory_readme;
use crate::raw_sources::{
    RawEndpointSample, RawFileState, RawJoinMap, RawSourceCoverage, RawSourceFailure,
    RawSourceInventory, RawSourceSamplingRequest,
};
use crate::{PolyError, Result};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn normalize_request(
    mut request: RawSourceSamplingRequest,
) -> Result<RawSourceSamplingRequest> {
    if request.output_root.is_relative() {
        let current_dir = env::current_dir().map_err(|err| {
            PolyError::raw_source(
                "POLY_RAW_SOURCE_CURRENT_DIR_FAILED",
                format!("read current directory: {err}"),
            )
        })?;
        request.output_root = current_dir.join(&request.output_root);
    }
    fs::create_dir_all(&request.output_root).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_OUTPUT_ROOT_CREATE_FAILED",
            format!(
                "create output root {}: {err}",
                request.output_root.display()
            ),
        )
    })?;
    request.output_root =
        display_safe_path(fs::canonicalize(&request.output_root).map_err(|err| {
            PolyError::raw_source(
                "POLY_RAW_SOURCE_OUTPUT_ROOT_CANONICALIZE_FAILED",
                format!(
                    "canonicalize output root {}: {err}",
                    request.output_root.display()
                ),
            )
        })?);
    Ok(request)
}

pub(crate) fn validate_request(request: &RawSourceSamplingRequest) -> Result<()> {
    if request.output_root.as_os_str().is_empty() {
        return Err(PolyError::raw_source(
            "POLY_RAW_SOURCE_OUTPUT_ROOT_EMPTY",
            "raw source output root must not be empty",
        ));
    }
    if request.timeout_secs == 0 {
        return Err(PolyError::raw_source(
            "POLY_RAW_SOURCE_TIMEOUT_INVALID",
            "raw source timeout must be greater than zero",
        ));
    }
    if request.max_body_bytes == 0 {
        return Err(PolyError::raw_source(
            "POLY_RAW_SOURCE_BODY_LIMIT_INVALID",
            "raw source max body bytes must be greater than zero",
        ));
    }
    Ok(())
}

pub(crate) fn inventory_failure(
    samples: &[RawEndpointSample],
    join: &RawJoinMap,
    readback_sha_mismatches: usize,
) -> Option<RawSourceFailure> {
    if readback_sha_mismatches > 0 {
        return Some(failure(
            "POLY_RAW_SOURCE_READBACK_SHA_MISMATCH",
            format!("{readback_sha_mismatches} sample body files failed SHA256 readback"),
            None,
        ));
    }
    for sample in samples {
        if accepted_sports_quiet_window(sample) {
            continue;
        }
        if !sample.expectation_met {
            return Some(failure(
                sample
                    .error_code
                    .as_deref()
                    .unwrap_or("POLY_RAW_SOURCE_SAMPLE_EXPECTATION_FAILED"),
                format!(
                    "sample {} failed expectation; expected_success={} status={:?} url={}",
                    sample.name, sample.expected_success, sample.status_code, sample.url
                ),
                Some(sample.name.clone()),
            ));
        }
    }
    let missing = required_missing_join_fields(join);
    if !missing.is_empty() {
        return Some(failure(
            "POLY_RAW_SOURCE_JOIN_IDENTIFIER_MISSING",
            format!("missing required join identifiers: {}", missing.join(", ")),
            None,
        ));
    }
    None
}

fn accepted_sports_quiet_window(sample: &RawEndpointSample) -> bool {
    sample.source == "websocket-sports"
        && sample.endpoint == "sports"
        && !sample.edge_case
        && !sample.expectation_met
        && sample.status_code == Some(101)
        && sample.error_code.as_deref() == Some("POLY_RAW_SOURCE_PUBLIC_WS_TIMEOUT")
        && sample.websocket_json_frame_count == Some(0)
}

pub(crate) fn coverage(samples: &[RawEndpointSample]) -> RawSourceCoverage {
    let sampled_sources = samples
        .iter()
        .map(|sample| sample.source.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut unsampled_sources = Vec::new();
    if !samples.iter().any(|sample| {
        sample.source == "historical-dump" && !sample.edge_case && sample.expectation_met
    }) {
        unsampled_sources.push("historical resolved-market dump files".to_string());
    }
    let has_onchain_order_filled = samples.iter().any(|sample| {
        ((sample.source == "polygon-rpc" && sample.endpoint == "v2-order-filled-logs")
            || (sample.source == "goldsky-subgraph"
                && sample.endpoint == "orderbook-order-filled-events"))
            && !sample.edge_case
            && sample.expectation_met
    });
    let has_onchain_redemptions = samples.iter().any(|sample| {
        sample.source == "goldsky-subgraph"
            && sample.endpoint == "activity-redemptions"
            && !sample.edge_case
            && sample.expectation_met
    });
    if !(has_onchain_order_filled && has_onchain_redemptions) {
        unsampled_sources.insert(
            0,
            "on-chain/subgraph OrderFilled and redemption events".to_string(),
        );
    }
    if !samples.iter().any(|sample| {
        sample.source == "websocket-market" && !sample.edge_case && sample.expectation_met
    }) {
        unsampled_sources.insert(0, "public WebSocket market channel".to_string());
    }
    if !samples
        .iter()
        .any(|sample| sample.source == "clob-post" && !sample.edge_case && sample.expectation_met)
    {
        unsampled_sources.push("POST batch CLOB market-data endpoints".to_string());
    }
    if !samples.iter().any(|sample| {
        sample.source == "websocket-sports" && !sample.edge_case && sample.expectation_met
    }) {
        unsampled_sources.push("public sports WebSocket channel".to_string());
    }
    if !samples.iter().any(|sample| {
        sample.source == "websocket-rtds" && !sample.edge_case && sample.expectation_met
    }) {
        unsampled_sources.push("public RTDS WebSocket channel".to_string());
    }
    if !samples.iter().any(|sample| {
        sample.source == "websocket-rtds"
            && sample.endpoint == "equity_prices"
            && !sample.edge_case
            && sample.expectation_met
    }) {
        unsampled_sources.push("RTDS equity_prices stream".to_string());
    }

    RawSourceCoverage {
        sample_count: samples.len(),
        required_success_count: samples
            .iter()
            .filter(|sample| sample.expected_success && sample.expectation_met)
            .count(),
        required_failure_count: samples
            .iter()
            .filter(|sample| sample.expected_success && !sample.expectation_met)
            .count(),
        edge_case_count: samples.iter().filter(|sample| sample.edge_case).count(),
        total_body_bytes: samples.iter().map(|sample| sample.body_bytes).sum(),
        readback_sha_mismatches: samples
            .iter()
            .filter(|sample| sample.body_sha256 != sample.after.body_sha256)
            .count(),
        sampled_sources,
        unsampled_sources,
    }
}

pub(crate) fn write_inventory_artifacts(root: &Path, inventory: &RawSourceInventory) -> Result<()> {
    fs::create_dir_all(root).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_ARTIFACT_DIR_CREATE_FAILED",
            format!("create artifact root {}: {err}", root.display()),
        )
    })?;
    write_json(&root.join("source-inventory.json"), inventory)?;
    write_json(&root.join("coverage-report.json"), &inventory.coverage)?;
    write_json(
        &root.join("docs-index-coverage.json"),
        &inventory.docs_index_coverage,
    )?;
    write_json(&root.join("join-map.json"), &inventory.join_map)?;
    let schema_dir = root.join("schema-observations");
    fs::create_dir_all(&schema_dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_SCHEMA_DIR_CREATE_FAILED",
            format!(
                "create schema observations dir {}: {err}",
                schema_dir.display()
            ),
        )
    })?;
    write_json(
        &schema_dir.join("http-public-observations.json"),
        &inventory
            .schema_observations
            .iter()
            .filter(|observation| !observation.source.starts_with("websocket-"))
            .collect::<Vec<_>>(),
    )?;
    let websocket_observations = inventory
        .schema_observations
        .iter()
        .filter(|observation| observation.source.starts_with("websocket-"))
        .collect::<Vec<_>>();
    if !websocket_observations.is_empty() {
        write_json(
            &schema_dir.join("websocket-public-observations.json"),
            &websocket_observations,
        )?;
        let market_websocket_observations = websocket_observations
            .iter()
            .copied()
            .filter(|observation| observation.source == "websocket-market")
            .collect::<Vec<_>>();
        if !market_websocket_observations.is_empty() {
            write_json(
                &schema_dir.join("websocket-market-observations.json"),
                &market_websocket_observations,
            )?;
        }
    }
    let sports_observations = inventory
        .schema_observations
        .iter()
        .filter(|observation| observation.source == "websocket-sports")
        .collect::<Vec<_>>();
    if !sports_observations.is_empty() {
        write_json(
            &schema_dir.join("websocket-sports-observations.json"),
            &sports_observations,
        )?;
    }
    let rtds_observations = inventory
        .schema_observations
        .iter()
        .filter(|observation| observation.source == "websocket-rtds")
        .collect::<Vec<_>>();
    if !rtds_observations.is_empty() {
        write_json(
            &schema_dir.join("websocket-rtds-observations.json"),
            &rtds_observations,
        )?;
    }
    let historical_observations = inventory
        .schema_observations
        .iter()
        .filter(|observation| observation.source == "historical-dump")
        .collect::<Vec<_>>();
    if !historical_observations.is_empty() {
        write_json(
            &schema_dir.join("historical-dump-observations.json"),
            &historical_observations,
        )?;
    }
    fs::write(root.join("README.md"), inventory_readme(inventory)).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_README_WRITE_FAILED",
            format!("write raw source README under {}: {err}", root.display()),
        )
    })?;
    Ok(())
}

pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            PolyError::raw_source(
                "POLY_RAW_SOURCE_JSON_DIR_CREATE_FAILED",
                format!("create JSON parent {}: {err}", parent.display()),
            )
        })?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_JSON_ENCODE_FAILED",
            format!("encode JSON {}: {err}", path.display()),
        )
    })?;
    fs::write(path, bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_JSON_WRITE_FAILED",
            format!("write JSON {}: {err}", path.display()),
        )
    })
}

pub(crate) fn file_state(body_path: &Path, metadata_path: &Path) -> Result<RawFileState> {
    let body_exists = body_path.exists();
    let body_bytes = if body_exists {
        body_path
            .metadata()
            .map_err(|err| {
                PolyError::raw_source(
                    "POLY_RAW_SOURCE_BODY_METADATA_FAILED",
                    format!("read body metadata {}: {err}", body_path.display()),
                )
            })?
            .len()
    } else {
        0
    };
    let body_sha256 = if body_exists {
        let bytes = fs::read(body_path).map_err(|err| {
            PolyError::raw_source(
                "POLY_RAW_SOURCE_BODY_READBACK_FAILED",
                format!("read body for SHA256 {}: {err}", body_path.display()),
            )
        })?;
        Some(sha256_hex(&bytes))
    } else {
        None
    };
    Ok(RawFileState {
        body_exists,
        metadata_exists: metadata_path.exists(),
        body_bytes,
        body_sha256,
    })
}

pub(crate) fn observe_json(value: &Value) -> (Option<usize>, Vec<String>, Vec<String>) {
    let (records, object) = match value {
        Value::Array(items) => (Some(items.len()), items.first().and_then(Value::as_object)),
        Value::Object(map) => {
            let records = map
                .values()
                .filter_map(Value::as_array)
                .map(Vec::len)
                .sum::<usize>();
            (Some(records.max(1)), Some(map))
        }
        Value::Null => (Some(0), None),
        _ => (Some(1), None),
    };
    let mut fields = Vec::new();
    let mut json_string_fields = Vec::new();
    if let Some(map) = object {
        fields = map.keys().cloned().collect();
        for (key, value) in map {
            if value
                .as_str()
                .is_some_and(|text| serde_json::from_str::<Value>(text).is_ok())
            {
                json_string_fields.push(key.clone());
            }
        }
    }
    (records, fields, json_string_fields)
}

pub(crate) fn sample_error_code(
    expected_success: bool,
    http_success: bool,
    json_parse_ok: bool,
    transport_error: &Option<String>,
) -> Option<String> {
    if transport_error.is_some() {
        return Some("POLY_RAW_SOURCE_HTTP_TRANSPORT_FAILED".to_string());
    }
    if expected_success && !http_success {
        return Some("POLY_RAW_SOURCE_HTTP_FAILED".to_string());
    }
    if expected_success && !json_parse_ok {
        return Some("POLY_RAW_SOURCE_JSON_PARSE_FAILED".to_string());
    }
    if !expected_success && !http_success {
        return Some("POLY_RAW_SOURCE_EXPECTED_HTTP_FAILURE".to_string());
    }
    if !expected_success && http_success {
        return Some("POLY_RAW_SOURCE_UNEXPECTED_HTTP_SUCCESS".to_string());
    }
    None
}

pub(crate) fn first_array_object(value: Option<&Value>) -> Option<&Value> {
    value
        .and_then(Value::as_array)
        .and_then(|items| items.first())
}

pub(crate) fn string_field(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(|value| match value {
        Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    })
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub(crate) fn sanitize_segment(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) fn now_unix_ms() -> Result<u128> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|err| {
            PolyError::raw_source(
                "POLY_RAW_SOURCE_CLOCK_INVALID",
                format!("system clock is before UNIX_EPOCH: {err}"),
            )
        })
}

fn required_missing_join_fields(join: &RawJoinMap) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if join.condition_id.is_none() {
        missing.push("condition_id");
    }
    if join.token_id.is_none() {
        missing.push("token_id");
    }
    if join.event_id.is_none() {
        missing.push("event_id");
    }
    if join.trade_user_address.is_none() {
        missing.push("trade_user_address");
    }
    missing
}

fn failure(
    code: impl Into<String>,
    message: impl Into<String>,
    sample_name: Option<String>,
) -> RawSourceFailure {
    RawSourceFailure {
        code: code.into(),
        message: message.into(),
        sample_name,
    }
}

#[cfg(windows)]
pub(crate) fn display_safe_path(path: PathBuf) -> PathBuf {
    let text = path.display().to_string();
    if let Some(stripped) = text.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        path
    }
}

#[cfg(not(windows))]
pub(crate) fn display_safe_path(path: PathBuf) -> PathBuf {
    path
}
