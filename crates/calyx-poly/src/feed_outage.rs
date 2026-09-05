//! Explicit feed-outage state for local read-only ingestion (#37).
//!
//! A feed observation is first persisted as source-of-truth JSON, then read back and normalized into
//! per-field present/absent slot state. Degraded feed state refuses forecast admission before the
//! normal quality gate runs.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::admission::{AdmissionDecision, AdmissionInputs, AdmissionParams, evaluate_admission};
use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};

pub const FEED_OUTAGE_SCHEMA_VERSION: &str = "poly.feed_outage.v1";
pub const FEED_OBSERVATION_ARTIFACT_KIND: &str = "poly_feed_observation";
pub const FEED_STATE_REPORT_ARTIFACT_KIND: &str = "poly_feed_state_report";

pub const FEED_OUTAGE_PASSED: &str = "CALYX_POLY_FEED_STATE_PRESENT";
pub const FEED_OUTAGE_DEGRADED: &str = "CALYX_POLY_FEED_STATE_DEGRADED";
pub const REFUSE_DEGRADED_FEED: &str = "CALYX_POLY_ADMISSION_DEGRADED_FEED";
const ERR_INVALID_OBSERVATION: &str = "CALYX_POLY_FEED_OBSERVATION_INVALID";
const ERR_READBACK_MISMATCH: &str = "CALYX_POLY_FEED_READBACK_MISMATCH";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedObservationStatus {
    Healthy,
    EmptyResponse,
    Timeout,
    MalformedPayload,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedSlotStatus {
    Present,
    Absent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedObservation {
    pub schema_version: String,
    pub artifact_kind: String,
    pub source_id: String,
    pub source_kind: String,
    pub source_url: String,
    pub captured_ts: u64,
    pub status: FeedObservationStatus,
    pub required_fields: Vec<String>,
    pub payload_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedSlotState {
    pub field: String,
    pub status: FeedSlotStatus,
    pub absent_reason: Option<String>,
    pub value_sha256: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedStateReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub observation_path: String,
    pub source_id: String,
    pub source_kind: String,
    pub source_url: String,
    pub captured_ts: u64,
    pub observed_status: FeedObservationStatus,
    pub payload_sha256: String,
    pub raw_observation_sha256: String,
    pub slot_states: Vec<FeedSlotState>,
    pub degraded: bool,
    pub absent_slot_count: usize,
    pub status_code: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedOutageRun {
    pub observation_path: PathBuf,
    pub report_path: PathBuf,
    pub observation: FeedObservation,
    pub report: FeedStateReport,
}

pub fn run_feed_outage_readback(
    observation: &FeedObservation,
    output_root: &Path,
) -> Result<FeedOutageRun> {
    validate_observation(observation)?;
    let observation_path = write_json(
        output_root,
        &format!(
            "{}-feed-observation.json",
            safe_file_id(&observation.source_id)
        ),
        observation,
    )?;
    let readback: FeedObservation = read_json(&observation_path)?;
    if readback != *observation {
        return Err(readback_mismatch(format!(
            "feed observation {} did not read back as written",
            observation_path.display()
        )));
    }
    let raw_observation_sha256 = sha256_file(&observation_path)?;
    let report = evaluate_feed_observation(&readback, &observation_path, &raw_observation_sha256);
    let report_path = write_json(
        output_root,
        &format!(
            "{}-feed-state-report.json",
            safe_file_id(&observation.source_id)
        ),
        &report,
    )?;
    let report_readback: FeedStateReport = read_json(&report_path)?;
    if report_readback != report {
        return Err(readback_mismatch(format!(
            "feed state report {} did not read back as written",
            report_path.display()
        )));
    }
    Ok(FeedOutageRun {
        observation_path,
        report_path,
        observation: readback,
        report: report_readback,
    })
}

pub fn feed_guarded_admission(
    report: &FeedStateReport,
    params: &AdmissionParams,
    inputs: &AdmissionInputs,
) -> AdmissionDecision {
    if report.degraded || report.absent_slot_count > 0 {
        return AdmissionDecision {
            admitted: false,
            code: REFUSE_DEGRADED_FEED.to_string(),
            reason: format!(
                "feed {} is degraded: {} absent required source slot(s)",
                report.source_id, report.absent_slot_count
            ),
        };
    }
    evaluate_admission(params, inputs)
}

fn evaluate_feed_observation(
    observation: &FeedObservation,
    observation_path: &Path,
    raw_observation_sha256: &str,
) -> FeedStateReport {
    let payload_sha256 = sha256_bytes(observation.payload_text.as_bytes());
    let (slot_states, reason) = match observation.status {
        FeedObservationStatus::Healthy => healthy_slot_states(observation),
        FeedObservationStatus::EmptyResponse => absent_slot_states(
            observation,
            "empty_response",
            "source returned an empty response",
        ),
        FeedObservationStatus::Timeout => absent_slot_states(
            observation,
            "timeout",
            "source timed out before payload read",
        ),
        FeedObservationStatus::MalformedPayload => absent_slot_states(
            observation,
            "malformed_payload",
            "source returned malformed payload bytes",
        ),
    };
    let absent_slot_count = slot_states
        .iter()
        .filter(|slot| slot.status == FeedSlotStatus::Absent)
        .count();
    let degraded = absent_slot_count > 0;
    FeedStateReport {
        schema_version: FEED_OUTAGE_SCHEMA_VERSION.to_string(),
        artifact_kind: FEED_STATE_REPORT_ARTIFACT_KIND.to_string(),
        observation_path: observation_path.display().to_string(),
        source_id: observation.source_id.clone(),
        source_kind: observation.source_kind.clone(),
        source_url: observation.source_url.clone(),
        captured_ts: observation.captured_ts,
        observed_status: observation.status,
        payload_sha256,
        raw_observation_sha256: raw_observation_sha256.to_string(),
        slot_states,
        degraded,
        absent_slot_count,
        status_code: if degraded {
            FEED_OUTAGE_DEGRADED
        } else {
            FEED_OUTAGE_PASSED
        }
        .to_string(),
        reason,
    }
}

fn healthy_slot_states(observation: &FeedObservation) -> (Vec<FeedSlotState>, String) {
    let Ok(Value::Object(payload)) = serde_json::from_str::<Value>(&observation.payload_text)
    else {
        return absent_slot_states(
            observation,
            "malformed_payload",
            "healthy transport status carried malformed payload bytes",
        );
    };
    let mut missing = 0usize;
    let mut slots = Vec::with_capacity(observation.required_fields.len());
    for field in &observation.required_fields {
        match payload.get(field).filter(|value| !value.is_null()) {
            Some(value) => slots.push(FeedSlotState {
                field: field.clone(),
                status: FeedSlotStatus::Present,
                absent_reason: None,
                value_sha256: Some(sha256_bytes(value.to_string().as_bytes())),
            }),
            None => {
                missing += 1;
                slots.push(FeedSlotState {
                    field: field.clone(),
                    status: FeedSlotStatus::Absent,
                    absent_reason: Some("missing_required_field".to_string()),
                    value_sha256: None,
                });
            }
        }
    }
    let reason = if missing == 0 {
        "all required source slots are present".to_string()
    } else {
        format!("{missing} required source slot(s) absent")
    };
    (slots, reason)
}

fn absent_slot_states(
    observation: &FeedObservation,
    reason_code: &str,
    reason: &str,
) -> (Vec<FeedSlotState>, String) {
    (
        observation
            .required_fields
            .iter()
            .map(|field| FeedSlotState {
                field: field.clone(),
                status: FeedSlotStatus::Absent,
                absent_reason: Some(reason_code.to_string()),
                value_sha256: None,
            })
            .collect(),
        reason.to_string(),
    )
}

fn validate_observation(observation: &FeedObservation) -> Result<()> {
    if observation.schema_version != FEED_OUTAGE_SCHEMA_VERSION
        || observation.artifact_kind != FEED_OBSERVATION_ARTIFACT_KIND
    {
        return invalid_observation("unexpected feed observation schema or artifact kind");
    }
    if observation.source_id.trim().is_empty()
        || observation.source_kind.trim().is_empty()
        || observation.source_url.trim().is_empty()
    {
        return invalid_observation("source_id, source_kind, and source_url are required");
    }
    if observation.required_fields.is_empty() {
        return invalid_observation("at least one required source field is required");
    }
    if observation
        .required_fields
        .iter()
        .any(|field| field.trim().is_empty())
    {
        return invalid_observation("required source field names must be non-empty");
    }
    Ok(())
}

fn safe_file_id(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn sha256_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).map_err(|err| {
        PolyError::raw_source(
            ERR_READBACK_MISMATCH,
            format!("read feed artifact {} for hash: {err}", path.display()),
        )
    })?;
    Ok(sha256_bytes(&bytes))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn invalid_observation(message: impl Into<String>) -> Result<()> {
    Err(PolyError::raw_source(
        ERR_INVALID_OBSERVATION,
        message.into(),
    ))
}

fn readback_mismatch(message: impl Into<String>) -> PolyError {
    PolyError::raw_source(ERR_READBACK_MISMATCH, message.into())
}
