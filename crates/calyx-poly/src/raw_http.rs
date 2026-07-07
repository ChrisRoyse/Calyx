use std::fs;

use serde_json::Value;

use crate::rate_limit_governor::{
    RateLimitEndpoint, RateLimitedHttpOutcome, execute_rate_limited_request, parse_retry_after_ms,
};
use crate::raw_source_probes::Probe;
use crate::raw_source_support::{
    file_state, observe_json, sample_error_code, sanitize_segment, sha256_hex, write_json,
};
use crate::raw_sources::{RawEndpointSample, RawFileState, RawSourceSamplingRequest};
use crate::{PolyError, Result};

pub(crate) fn capture_http_probe(
    request: &RawSourceSamplingRequest,
    agent: &ureq::Agent,
    probe: &Probe,
) -> Result<(RawEndpointSample, serde_json::Result<Value>)> {
    let sample_dir = request
        .output_root
        .join("raw")
        .join(&probe.source)
        .join(sanitize_segment(&probe.name));
    let body_path = sample_dir.join(if probe.expect_json {
        "body.json"
    } else {
        "body.bin"
    });
    let metadata_path = sample_dir.join("metadata.json");
    let request_path = sample_dir.join("request.json");
    let before = file_state(&body_path, &metadata_path)?;
    let max_body_bytes = u64::try_from(request.max_body_bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_BODY_LIMIT_CONVERT_FAILED",
            format!(
                "convert max body bytes {} to u64: {err}",
                request.max_body_bytes
            ),
        )
    })?;

    fs::create_dir_all(&sample_dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_SAMPLE_DIR_CREATE_FAILED",
            format!("create sample directory {}: {err}", sample_dir.display()),
        )
    })?;
    let request_body_bytes = persist_request_body(probe, &request_path)?;
    verify_request_body_readback(probe, &request_path, request_body_bytes.as_deref())?;
    let (status_code, bytes, transport_error) =
        execute_http_probe(agent, probe, request_body_bytes.as_deref(), max_body_bytes)?;
    persist_response_body(&body_path, &bytes)?;

    let parsed = serde_json::from_slice::<Value>(&bytes);
    let json_parse_ok = parsed.is_ok();
    let http_success = status_code.is_some_and(|code| (200..300).contains(&code));
    let expected_body_ok = if probe.expect_json {
        json_parse_ok
    } else {
        !bytes.is_empty()
    };
    let expectation_met = if probe.expected_success {
        http_success && expected_body_ok && transport_error.is_none()
    } else {
        !http_success || transport_error.is_some()
    };
    let body_sha256 = (!bytes.is_empty()).then(|| sha256_hex(&bytes));
    let (record_count, top_level_fields, _) =
        parsed
            .as_ref()
            .map(observe_json)
            .unwrap_or((None, Vec::new(), Vec::new()));
    let mut sample = RawEndpointSample {
        name: probe.name.clone(),
        source: probe.source.clone(),
        transport: "http".to_string(),
        endpoint: probe.endpoint.clone(),
        method: probe.method.clone(),
        url: probe.url.clone(),
        docs_url: probe.docs_url.clone(),
        request_body_exists: request_body_bytes.is_some(),
        request_body_bytes: request_body_bytes.as_ref().map_or(0, Vec::len) as u64,
        request_body_sha256: request_body_bytes.as_ref().map(|bytes| sha256_hex(bytes)),
        request_body_path: request_body_bytes
            .as_ref()
            .map(|_| request_path.display().to_string()),
        expected_success: probe.expected_success,
        edge_case: probe.edge_case,
        status_code,
        http_success,
        expectation_met,
        error_code: http_error_code(probe, http_success, json_parse_ok, &transport_error, &bytes),
        error_message: transport_error,
        body_exists: !bytes.is_empty(),
        body_bytes: bytes.len() as u64,
        body_sha256,
        json_parse_ok,
        record_count,
        top_level_fields,
        websocket_frame_count: None,
        websocket_json_frame_count: None,
        websocket_event_types: Vec::new(),
        websocket_pong_received: None,
        websocket_outbound_messages: Vec::new(),
        websocket_frames: Vec::new(),
        before,
        after: RawFileState {
            body_exists: false,
            metadata_exists: false,
            body_bytes: 0,
            body_sha256: None,
        },
        body_path: body_path.display().to_string(),
        metadata_path: metadata_path.display().to_string(),
    };
    write_json(&metadata_path, &sample)?;
    let after = file_state(&body_path, &metadata_path)?;
    if sample.body_sha256 != after.body_sha256 {
        sample.error_code = Some("POLY_RAW_SOURCE_READBACK_SHA_MISMATCH".to_string());
        sample.expectation_met = false;
    }
    sample.after = after;
    write_json(&metadata_path, &sample)?;
    Ok((sample, parsed))
}

fn http_error_code(
    probe: &Probe,
    http_success: bool,
    json_parse_ok: bool,
    transport_error: &Option<String>,
    bytes: &[u8],
) -> Option<String> {
    if probe.expect_json {
        return sample_error_code(
            probe.expected_success,
            http_success,
            json_parse_ok,
            transport_error,
        );
    }
    if transport_error.is_some() {
        return Some("POLY_RAW_SOURCE_HTTP_TRANSPORT_FAILED".to_string());
    }
    if probe.expected_success && !http_success {
        return Some("POLY_RAW_SOURCE_HTTP_FAILED".to_string());
    }
    if probe.expected_success && bytes.is_empty() {
        return Some("POLY_RAW_SOURCE_BODY_EMPTY".to_string());
    }
    if !probe.expected_success && !http_success {
        return Some("POLY_RAW_SOURCE_EXPECTED_HTTP_FAILURE".to_string());
    }
    if !probe.expected_success && http_success {
        return Some("POLY_RAW_SOURCE_UNEXPECTED_HTTP_SUCCESS".to_string());
    }
    None
}

fn execute_http_probe(
    agent: &ureq::Agent,
    probe: &Probe,
    request_body_bytes: Option<&[u8]>,
    max_body_bytes: u64,
) -> Result<(Option<u16>, Vec<u8>, Option<String>)> {
    let endpoint = RateLimitEndpoint::new(&probe.source, &probe.endpoint, &probe.method);
    execute_rate_limited_request(&endpoint, || {
        let result = match probe.method.as_str() {
            "GET" => agent
                .get(&probe.url)
                .header("Accept", "application/json")
                .call(),
            "POST" => {
                let body = request_body_bytes.ok_or_else(|| {
                    PolyError::raw_source(
                        "POLY_RAW_SOURCE_POST_BODY_MISSING",
                        format!("POST probe {} has no request body", probe.name),
                    )
                })?;
                agent
                    .post(&probe.url)
                    .header("Accept", "application/json")
                    .header("Content-Type", "application/json")
                    .send(body)
            }
            other => {
                return Err(PolyError::raw_source(
                    "POLY_RAW_SOURCE_HTTP_METHOD_UNSUPPORTED",
                    format!("unsupported method {other} for probe {}", probe.name),
                ));
            }
        };
        let mut status_code = None;
        let mut retry_after_ms = None;
        let mut bytes = Vec::new();
        let mut transport_error = None;
        match result {
            Ok(mut response) => {
                status_code = Some(response.status().as_u16());
                retry_after_ms = parse_retry_after_ms(
                    response
                        .headers()
                        .get("retry-after")
                        .and_then(|value| value.to_str().ok()),
                );
                bytes = response
                    .body_mut()
                    .with_config()
                    .limit(max_body_bytes)
                    .read_to_vec()
                    .map_err(|err| {
                        PolyError::raw_source(
                            "POLY_RAW_SOURCE_BODY_READ_FAILED",
                            format!("read body for {}: {err}", probe.name),
                        )
                    })?;
            }
            Err(err) => transport_error = Some(err.to_string()),
        }
        Ok(RateLimitedHttpOutcome {
            status_code,
            retry_after_ms,
            value: (status_code, bytes, transport_error),
        })
    })
}

fn persist_request_body(probe: &Probe, request_path: &std::path::Path) -> Result<Option<Vec<u8>>> {
    let Some(body) = &probe.request_body else {
        remove_stale(request_path, "request")?;
        return Ok(None);
    };
    let bytes = serde_json::to_vec(body).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_REQUEST_BODY_ENCODE_FAILED",
            format!("encode request body for {}: {err}", probe.name),
        )
    })?;
    fs::write(request_path, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_REQUEST_BODY_WRITE_FAILED",
            format!("write request body {}: {err}", request_path.display()),
        )
    })?;
    Ok(Some(bytes))
}

fn verify_request_body_readback(
    probe: &Probe,
    request_path: &std::path::Path,
    expected: Option<&[u8]>,
) -> Result<()> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let readback = fs::read(request_path).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_REQUEST_BODY_READBACK_FAILED",
            format!(
                "read request body after write for {} at {}: {err}",
                probe.name,
                request_path.display()
            ),
        )
    })?;
    if readback != expected {
        return Err(PolyError::raw_source(
            "POLY_RAW_SOURCE_REQUEST_BODY_READBACK_MISMATCH",
            format!(
                "request body readback mismatch for {} at {}; expected_sha256={} actual_sha256={}",
                probe.name,
                request_path.display(),
                sha256_hex(expected),
                sha256_hex(&readback)
            ),
        ));
    }
    Ok(())
}

fn persist_response_body(body_path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    if !bytes.is_empty() {
        fs::write(body_path, bytes).map_err(|err| {
            PolyError::raw_source(
                "POLY_RAW_SOURCE_BODY_WRITE_FAILED",
                format!("write body {}: {err}", body_path.display()),
            )
        })?;
    } else {
        remove_stale(body_path, "body")?;
    }
    Ok(())
}

fn remove_stale(path: &std::path::Path, label: &str) -> Result<()> {
    if path.exists() {
        fs::remove_file(path).map_err(|err| {
            PolyError::raw_source(
                "POLY_RAW_SOURCE_STALE_FILE_REMOVE_FAILED",
                format!("remove stale {label} file {}: {err}", path.display()),
            )
        })?;
    }
    Ok(())
}
