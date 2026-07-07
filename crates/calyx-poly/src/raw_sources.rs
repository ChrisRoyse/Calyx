//! Raw Polymarket source inventory and sample-corpus capture.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::raw_clob_post_probes::clob_post_runtime_semantics_probes;
pub use crate::raw_docs_coverage::{
    RawDocsCoverageRow, RawDocsIndexCoverage, RawDocsIndexSnapshot,
};
use crate::raw_docs_coverage::{build_docs_index_coverage, docs_coverage_failure};
use crate::raw_historical::capture_historical_samples;
use crate::raw_http::capture_http_probe;
use crate::raw_onchain::capture_onchain_samples;
use crate::raw_public_websocket::capture_public_websocket;
use crate::raw_public_websocket_probes::public_websocket_probes;
use crate::raw_runtime_semantics::{
    clob_post_runtime_semantics, runtime_semantics_failure, write_runtime_semantics_artifacts,
};
use crate::raw_source_probes::{Probe, docs, dynamic_probes, edge_probes, initial_probes};
use crate::raw_source_support::{
    coverage, first_array_object, inventory_failure, normalize_request, now_unix_ms, observe_json,
    string_field, validate_request, write_inventory_artifacts,
};
use crate::raw_websocket::{capture_market_websocket, market_websocket_probes};
use crate::{PolyError, Result};

pub const RAW_SOURCE_INVENTORY_SCHEMA_VERSION: &str = "poly.raw_source_inventory.v2";
pub const RAW_SOURCE_SAMPLE_PASSED: &str = "POLY_RAW_SOURCE_SAMPLE_PASSED";

const DEFAULT_MAX_BODY_BYTES: usize = 50 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawSourceSamplingRequest {
    pub output_root: PathBuf,
    pub timeout_secs: u64,
    pub max_body_bytes: usize,
}

impl RawSourceSamplingRequest {
    pub fn target_default() -> Self {
        Self {
            output_root: PathBuf::from("target/fsv/polymarket_raw_source_inventory"),
            timeout_secs: 30,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        }
    }

    pub fn normalized(self) -> Result<Self> {
        normalize_request(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawFileState {
    pub body_exists: bool,
    pub metadata_exists: bool,
    pub body_bytes: u64,
    pub body_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawEndpointSample {
    pub name: String,
    pub source: String,
    pub transport: String,
    pub endpoint: String,
    pub method: String,
    pub url: String,
    pub docs_url: String,
    pub request_body_exists: bool,
    pub request_body_bytes: u64,
    pub request_body_sha256: Option<String>,
    pub request_body_path: Option<String>,
    pub expected_success: bool,
    pub edge_case: bool,
    pub status_code: Option<u16>,
    pub http_success: bool,
    pub expectation_met: bool,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub body_exists: bool,
    pub body_bytes: u64,
    pub body_sha256: Option<String>,
    pub json_parse_ok: bool,
    pub record_count: Option<usize>,
    pub top_level_fields: Vec<String>,
    pub websocket_frame_count: Option<usize>,
    pub websocket_json_frame_count: Option<usize>,
    pub websocket_event_types: Vec<String>,
    pub websocket_pong_received: Option<bool>,
    pub websocket_outbound_messages: Vec<String>,
    pub websocket_frames: Vec<RawWebSocketFrameState>,
    pub before: RawFileState,
    pub after: RawFileState,
    pub body_path: String,
    pub metadata_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawWebSocketFrameState {
    pub direction: String,
    pub opcode: String,
    pub received_at_unix_ms: u128,
    pub body_bytes: u64,
    pub body_sha256: Option<String>,
    pub json_parse_ok: bool,
    pub event_type: Option<String>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawJoinMap {
    pub market_id: Option<String>,
    pub market_slug: Option<String>,
    pub event_id: Option<String>,
    pub condition_id: Option<String>,
    pub token_id: Option<String>,
    pub opposite_token_id: Option<String>,
    pub trade_user_address: Option<String>,
    pub trade_token_id: Option<String>,
    pub trade_condition_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawSourceCoverage {
    pub sample_count: usize,
    pub required_success_count: usize,
    pub required_failure_count: usize,
    pub edge_case_count: usize,
    pub total_body_bytes: u64,
    pub readback_sha_mismatches: usize,
    pub sampled_sources: Vec<String>,
    pub unsampled_sources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawSchemaObservation {
    pub sample_name: String,
    pub source: String,
    pub body_shape: String,
    pub result_semantics: String,
    pub top_level_fields: Vec<String>,
    pub record_count: Option<usize>,
    pub json_string_fields: Vec<String>,
    pub websocket_event_types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawRuntimeSemanticsObservation {
    pub sample_name: String,
    pub source: String,
    pub endpoint: String,
    pub method: String,
    pub docs_url: String,
    pub request_case: String,
    pub expected_runtime_semantics: String,
    pub actual_status_code: Option<u16>,
    pub actual_body_shape: String,
    pub actual_body_fields: Vec<String>,
    pub semantics_match: bool,
    pub failure_code: Option<String>,
    pub schema_implication: String,
    pub request_body_path: Option<String>,
    pub request_body_sha256: Option<String>,
    pub body_path: String,
    pub metadata_path: String,
    pub body_bytes: u64,
    pub body_sha256: Option<String>,
    pub before: RawFileState,
    pub after: RawFileState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawSourceFailure {
    pub code: String,
    pub message: String,
    pub sample_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawSourceInventory {
    pub schema_version: String,
    pub captured_at_unix_ms: u128,
    pub source_of_truth: String,
    pub docs: Vec<String>,
    pub samples: Vec<RawEndpointSample>,
    pub join_map: RawJoinMap,
    pub coverage: RawSourceCoverage,
    pub docs_index_coverage: RawDocsIndexCoverage,
    pub schema_observations: Vec<RawSchemaObservation>,
    pub runtime_semantics: Vec<RawRuntimeSemanticsObservation>,
    pub passed: bool,
    pub status_code: String,
    pub failure: Option<RawSourceFailure>,
}

struct RawSampler {
    request: RawSourceSamplingRequest,
    agent: ureq::Agent,
    parsed_bodies: BTreeMap<String, Value>,
}

pub fn run_polymarket_raw_source_sampling(
    request: RawSourceSamplingRequest,
) -> Result<RawSourceInventory> {
    let request = request.normalized()?;
    validate_request(&request)?;
    let mut sampler = RawSampler::new(request.clone());
    let mut samples = Vec::new();

    for probe in initial_probes() {
        samples.push(sampler.capture(&probe)?);
    }
    let join_map = sampler.derive_join_map();
    for probe in dynamic_probes(&join_map) {
        samples.push(sampler.capture(&probe)?);
    }
    for probe in edge_probes(&join_map) {
        samples.push(sampler.capture(&probe)?);
    }
    if let Some(token) = &join_map.token_id {
        let unique_tokens = sampler.derive_unique_clob_tokens(21)?;
        for probe in clob_post_runtime_semantics_probes(token, &unique_tokens) {
            samples.push(sampler.capture(&probe)?);
        }
    }
    for probe in market_websocket_probes(&join_map) {
        samples.push(capture_market_websocket(&request, &probe)?);
    }
    for probe in public_websocket_probes() {
        samples.push(capture_public_websocket(&request, &probe)?);
    }
    samples.extend(capture_onchain_samples(&request, &sampler.agent)?);
    samples.extend(capture_historical_samples(&request, &sampler.agent)?);

    let mut schema_observations = sampler.schema_observations(&samples);
    schema_observations.extend(
        samples
            .iter()
            .filter(|sample| sample.source == "historical-dump")
            .map(|sample| schema_observation(sample, None, Vec::new())),
    );
    let runtime_semantics = clob_post_runtime_semantics(&samples, &sampler.parsed_bodies);
    let coverage = coverage(&samples);
    let docs_index_coverage =
        build_docs_index_coverage(&request, &sampler.agent, &samples, &coverage)?;
    let failure = runtime_semantics_failure(&runtime_semantics)
        .or_else(|| inventory_failure(&samples, &join_map, coverage.readback_sha_mismatches))
        .or_else(|| docs_coverage_failure(&docs_index_coverage));
    let passed = failure.is_none();
    let inventory = RawSourceInventory {
        schema_version: RAW_SOURCE_INVENTORY_SCHEMA_VERSION.to_string(),
        captured_at_unix_ms: now_unix_ms()?,
        source_of_truth:
            "live public/read-only Polymarket HTTP/WebSocket responses plus physical raw corpus files"
                .to_string(),
        docs: docs(),
        samples,
        join_map,
        coverage,
        docs_index_coverage,
        schema_observations,
        runtime_semantics,
        passed,
        status_code: if passed {
            RAW_SOURCE_SAMPLE_PASSED.to_string()
        } else {
            failure
                .as_ref()
                .map(|failure| failure.code.clone())
                .unwrap_or_else(|| "POLY_RAW_SOURCE_SAMPLE_FAILED".to_string())
        },
        failure,
    };
    write_inventory_artifacts(&request.output_root, &inventory)?;
    write_runtime_semantics_artifacts(&request.output_root, &inventory.runtime_semantics)?;
    Ok(inventory)
}

pub fn read_raw_source_inventory(path: &Path) -> Result<RawSourceInventory> {
    let bytes = fs::read(path).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_INVENTORY_READ_FAILED",
            format!("read inventory {}: {err}", path.display()),
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_INVENTORY_DECODE_FAILED",
            format!("decode inventory {}: {err}", path.display()),
        )
    })
}

pub fn require_raw_source_sampling_passed(inventory: &RawSourceInventory) -> Result<()> {
    if inventory.passed {
        Ok(())
    } else {
        let message = inventory
            .failure
            .as_ref()
            .map(|failure| failure.message.clone())
            .unwrap_or_else(|| "raw source sampling did not pass".to_string());
        Err(PolyError::raw_source(
            inventory.status_code.clone(),
            message,
        ))
    }
}

impl RawSampler {
    fn new(request: RawSourceSamplingRequest) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(request.timeout_secs)))
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            request,
            agent,
            parsed_bodies: BTreeMap::new(),
        }
    }

    fn capture(&mut self, probe: &Probe) -> Result<RawEndpointSample> {
        let (sample, parsed) = capture_http_probe(&self.request, &self.agent, probe)?;
        if let Ok(value) = &parsed {
            self.parsed_bodies.insert(probe.name.clone(), value.clone());
        }
        Ok(sample)
    }

    fn derive_join_map(&self) -> RawJoinMap {
        let mut join = RawJoinMap::default();
        if let Some(first_market) =
            first_array_object(self.parsed_bodies.get("gamma_markets_active"))
        {
            join.market_id = string_field(first_market, "id");
            join.market_slug = string_field(first_market, "slug");
            join.condition_id = string_field(first_market, "conditionId");
            if let Some(tokens) = string_field(first_market, "clobTokenIds")
                .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
            {
                join.token_id = tokens.first().cloned();
                join.opposite_token_id = tokens.get(1).cloned();
            }
            join.event_id = first_market
                .get("events")
                .and_then(Value::as_array)
                .and_then(|events| events.first())
                .and_then(|event| string_field(event, "id"));
        }
        if let Some(first_trade) = first_array_object(self.parsed_bodies.get("data_trades")) {
            join.trade_user_address = string_field(first_trade, "proxyWallet");
            join.trade_token_id = string_field(first_trade, "asset");
            join.trade_condition_id = string_field(first_trade, "conditionId");
        }
        join
    }

    fn derive_unique_clob_tokens(&self, required: usize) -> Result<Vec<String>> {
        let Some(Value::Array(markets)) = self.parsed_bodies.get("gamma_markets_active") else {
            return Err(PolyError::raw_source(
                "POLY_RAW_SOURCE_GAMMA_ACTIVE_MARKETS_MISSING",
                "gamma_markets_active must be sampled before deriving unique CLOB tokens",
            ));
        };
        let mut tokens = Vec::new();
        for market in markets {
            let Some(raw_tokens) = string_field(market, "clobTokenIds") else {
                continue;
            };
            let parsed_tokens =
                serde_json::from_str::<Vec<String>>(&raw_tokens).map_err(|err| {
                    PolyError::raw_source(
                        "POLY_RAW_SOURCE_CLOB_TOKEN_IDS_DECODE_FAILED",
                        format!("decode Gamma clobTokenIds field {raw_tokens}: {err}"),
                    )
                })?;
            for token in parsed_tokens {
                if !token.trim().is_empty() && !tokens.contains(&token) {
                    tokens.push(token);
                    if tokens.len() == required {
                        return Ok(tokens);
                    }
                }
            }
        }
        Err(PolyError::raw_source(
            "POLY_RAW_SOURCE_UNIQUE_CLOB_TOKENS_INSUFFICIENT",
            format!(
                "need {required} unique Gamma-derived CLOB tokens for runtime semantics; got {}",
                tokens.len()
            ),
        ))
    }

    fn schema_observations(&self, samples: &[RawEndpointSample]) -> Vec<RawSchemaObservation> {
        let mut observations = samples
            .iter()
            .filter_map(|sample| {
                self.parsed_bodies.get(&sample.name).map(|value| {
                    let (_, _, json_string_fields) = observe_json(value);
                    schema_observation(sample, Some(value), json_string_fields)
                })
            })
            .collect::<Vec<_>>();
        observations.extend(
            samples
                .iter()
                .filter(|sample| sample.transport == "websocket")
                .map(|sample| schema_observation(sample, None, Vec::new())),
        );
        observations
    }
}

fn schema_observation(
    sample: &RawEndpointSample,
    value: Option<&Value>,
    json_string_fields: Vec<String>,
) -> RawSchemaObservation {
    let body_shape = schema_body_shape(sample, value);
    RawSchemaObservation {
        sample_name: sample.name.clone(),
        source: sample.source.clone(),
        result_semantics: result_semantics(sample, &body_shape),
        body_shape,
        top_level_fields: sample.top_level_fields.clone(),
        record_count: sample.record_count,
        json_string_fields,
        websocket_event_types: sample.websocket_event_types.clone(),
    }
}

fn schema_body_shape(sample: &RawEndpointSample, value: Option<&Value>) -> String {
    match value {
        Some(Value::Null) => "json_null",
        Some(Value::Array(items)) if items.is_empty() => "empty_array",
        Some(Value::Array(_)) => "non_empty_array",
        Some(Value::Object(map)) if map.is_empty() => "empty_object",
        Some(Value::Object(_)) => "object",
        Some(Value::String(_)) => "string",
        Some(Value::Number(_)) => "number",
        Some(Value::Bool(_)) => "boolean",
        None if sample.transport == "websocket" => {
            if sample.websocket_json_frame_count.unwrap_or(0) == 0 {
                "websocket_control_frames_only"
            } else {
                "websocket_json_event_frames"
            }
        }
        None if !sample.body_exists => "missing_body",
        None if !sample.json_parse_ok => "non_json_body",
        None => "json_not_reparsed",
    }
    .to_string()
}

fn result_semantics(sample: &RawEndpointSample, body_shape: &str) -> String {
    if url_param_is(&sample.url, "limit", "0") {
        match (body_shape, sample.http_success) {
            ("json_null", true) => "limit_zero_json_null".to_string(),
            ("empty_array", true) => "limit_zero_empty_array".to_string(),
            ("non_empty_array", true) => "limit_zero_defaulted_non_empty_page".to_string(),
            ("object", false) => "limit_zero_error_object".to_string(),
            ("empty_object", false) => "limit_zero_empty_error_object".to_string(),
            (shape, true) => format!("limit_zero_success_{shape}"),
            (shape, false) => format!("limit_zero_error_{shape}"),
        }
    } else if !sample.http_success && sample.status_code.is_some() {
        format!("http_error_{body_shape}")
    } else {
        body_shape.to_string()
    }
}

fn url_param_is(url: &str, name: &str, expected: &str) -> bool {
    url.split('?').nth(1).is_some_and(|query| {
        query.split('&').any(|pair| {
            pair.split_once('=')
                .is_some_and(|(key, value)| key == name && value == expected)
        })
    })
}
