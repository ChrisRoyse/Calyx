//! UMA finality intake for Polymarket resolution records.
//!
//! This module converts read-only UMA/CTF observations into Poly [`Resolution`] records only when
//! the source state is final, non-disputed, and has a single winning payout slot.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{PolyError, Result};
use crate::model::{OracleRiskEvidence, Resolution};

pub const UMA_ONCHAIN_SOURCE: &str = "uma-onchain";
pub const UMA_RESOLUTION_WATCHER_SCHEMA_VERSION: &str = "poly.uma_resolution_watcher.v1";
pub const UMA_RESOLUTION_WATCHER_REPORT_FILE: &str = "uma_resolution_watcher_report.json";
pub const ERR_UMA_RESOLUTION_INVALID: &str = "CALYX_POLY_UMA_RESOLUTION_INVALID";
pub const ERR_UMA_RESOLUTION_LOG_INVALID: &str = "CALYX_POLY_UMA_RESOLUTION_LOG_INVALID";
pub const ERR_UMA_RESOLUTION_NOT_FINAL: &str = "CALYX_POLY_UMA_RESOLUTION_NOT_FINAL";
pub const ERR_UMA_RESOLUTION_READBACK_MISMATCH: &str =
    "CALYX_POLY_UMA_RESOLUTION_READBACK_MISMATCH";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UmaFinalityState {
    Proposed,
    InLiveness,
    Disputed,
    Finalized,
    VoidedInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UmaResolutionObservation {
    pub condition_id: String,
    pub question_id: Option<String>,
    pub oracle: String,
    pub outcome_labels: Vec<String>,
    pub payout_numerators: Vec<u64>,
    pub proposed_ts: Option<u64>,
    pub expiration_ts: Option<u64>,
    pub observed_ts: u64,
    pub resolved_ts: Option<u64>,
    pub active_dispute: bool,
    pub voided_invalid: bool,
    pub source_tx_hash: Option<String>,
    pub source_block_number: Option<u64>,
    pub source_log_index: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UmaResolutionDecision {
    pub condition_id: String,
    pub finality_state: UmaFinalityState,
    pub liveness_seconds_remaining: f64,
    pub disputed: bool,
    pub voided: bool,
    pub groundable: bool,
    pub reason: String,
    pub oracle_risk: OracleRiskEvidence,
    pub resolution: Option<Resolution>,
    pub source_tx_hash: Option<String>,
    pub source_block_number: Option<u64>,
    pub source_log_index: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UmaResolutionWatcherRequest {
    pub observations: Vec<UmaResolutionObservation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UmaResolutionWatcherReport {
    pub schema_version: String,
    pub observed_count: usize,
    pub finalized_resolution_count: usize,
    pub held_count: usize,
    pub voided_count: usize,
    pub decisions: Vec<UmaResolutionDecision>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UmaResolutionWatcherRun {
    pub report_path: PathBuf,
    pub report: UmaResolutionWatcherReport,
}

pub fn evaluate_uma_resolution(
    observation: &UmaResolutionObservation,
) -> Result<UmaResolutionDecision> {
    validate_observation(observation)?;
    let liveness = liveness_remaining(observation);
    let state = finality_state(observation, liveness);
    let (resolution, voided, reason) = match state {
        UmaFinalityState::Finalized => finalized_resolution(observation)?,
        UmaFinalityState::VoidedInvalid => (
            None,
            true,
            "UMA/CTF payout vector is void or non-single-winner; hold scoring closed".to_string(),
        ),
        UmaFinalityState::Disputed => (
            None,
            false,
            "UMA dispute is active; no grounding or scoring until final".to_string(),
        ),
        UmaFinalityState::InLiveness => (
            None,
            false,
            "UMA optimistic-oracle liveness window remains open".to_string(),
        ),
        UmaFinalityState::Proposed => (
            None,
            false,
            "UMA proposal is observed but no final CTF payout exists yet".to_string(),
        ),
    };
    let groundable = resolution.is_some();
    Ok(UmaResolutionDecision {
        condition_id: observation.condition_id.clone(),
        finality_state: state,
        liveness_seconds_remaining: liveness as f64,
        disputed: observation.active_dispute,
        voided,
        groundable,
        reason,
        oracle_risk: oracle_risk_for(state, observation.active_dispute, liveness),
        resolution,
        source_tx_hash: observation.source_tx_hash.clone(),
        source_block_number: observation.source_block_number,
        source_log_index: observation.source_log_index,
    })
}

pub fn require_groundable_uma_resolution(decision: &UmaResolutionDecision) -> Result<&Resolution> {
    decision.resolution.as_ref().ok_or_else(|| {
        uma_error(
            ERR_UMA_RESOLUTION_NOT_FINAL,
            format!(
                "condition {} is {:?}: {}",
                decision.condition_id, decision.finality_state, decision.reason
            ),
        )
    })
}

pub fn compute_uma_resolution_watcher_report(
    request: &UmaResolutionWatcherRequest,
) -> Result<UmaResolutionWatcherReport> {
    if request.observations.is_empty() {
        return Err(uma_error(
            ERR_UMA_RESOLUTION_INVALID,
            "UMA resolution watcher requires at least one observation",
        ));
    }
    let decisions = request
        .observations
        .iter()
        .map(evaluate_uma_resolution)
        .collect::<Result<Vec<_>>>()?;
    Ok(UmaResolutionWatcherReport {
        schema_version: UMA_RESOLUTION_WATCHER_SCHEMA_VERSION.to_string(),
        observed_count: decisions.len(),
        finalized_resolution_count: decisions.iter().filter(|d| d.groundable).count(),
        held_count: decisions
            .iter()
            .filter(|d| !d.groundable && !d.voided)
            .count(),
        voided_count: decisions.iter().filter(|d| d.voided).count(),
        decisions,
    })
}

pub fn run_uma_resolution_watcher(
    output_root: &Path,
    request: &UmaResolutionWatcherRequest,
) -> Result<UmaResolutionWatcherRun> {
    let report = compute_uma_resolution_watcher_report(request)?;
    fs::create_dir_all(output_root).map_err(|err| {
        uma_error(
            ERR_UMA_RESOLUTION_INVALID,
            format!(
                "create UMA watcher output root {}: {err}",
                output_root.display()
            ),
        )
    })?;
    let report_path = output_root.join(UMA_RESOLUTION_WATCHER_REPORT_FILE);
    write_uma_resolution_watcher_report(&report_path, &report)?;
    let readback = read_uma_resolution_watcher_report(&report_path)?;
    if readback != report {
        return Err(uma_error(
            ERR_UMA_RESOLUTION_READBACK_MISMATCH,
            format!(
                "UMA watcher report readback mismatch at {}",
                report_path.display()
            ),
        ));
    }
    Ok(UmaResolutionWatcherRun {
        report_path,
        report,
    })
}

pub fn write_uma_resolution_watcher_report(
    path: &Path,
    report: &UmaResolutionWatcherReport,
) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(report).map_err(|err| {
        uma_error(
            ERR_UMA_RESOLUTION_INVALID,
            format!("encode UMA watcher report {}: {err}", path.display()),
        )
    })?;
    fs::write(path, bytes).map_err(|err| {
        uma_error(
            ERR_UMA_RESOLUTION_INVALID,
            format!("write UMA watcher report {}: {err}", path.display()),
        )
    })
}

pub fn read_uma_resolution_watcher_report(path: &Path) -> Result<UmaResolutionWatcherReport> {
    let bytes = fs::read(path).map_err(|err| {
        uma_error(
            ERR_UMA_RESOLUTION_INVALID,
            format!("read UMA watcher report {}: {err}", path.display()),
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|err| {
        uma_error(
            ERR_UMA_RESOLUTION_INVALID,
            format!("decode UMA watcher report {}: {err}", path.display()),
        )
    })
}

pub fn parse_condition_resolution_log_value(
    log: &Value,
    outcome_labels: Vec<String>,
    resolved_ts: u64,
) -> Result<UmaResolutionObservation> {
    let topics = log
        .get("topics")
        .and_then(Value::as_array)
        .ok_or_else(|| log_error("ConditionResolution log missing topics array"))?;
    if topics.len() < 4 {
        return Err(log_error(
            "ConditionResolution log requires four indexed topics",
        ));
    }
    let topic = |index: usize| -> Result<String> {
        topics[index]
            .as_str()
            .filter(|text| is_hex_word(text))
            .map(str::to_string)
            .ok_or_else(|| log_error(format!("topic {index} is not a 32-byte hex word")))
    };
    let condition_id = topic(1)?;
    let oracle = address_from_topic(&topic(2)?)?;
    let question_id = Some(topic(3)?);
    let data = log
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| log_error("ConditionResolution log missing data field"))?;
    let (_outcome_slot_count, payout_numerators) = decode_condition_resolution_data(data)?;
    Ok(UmaResolutionObservation {
        condition_id,
        question_id,
        oracle,
        outcome_labels,
        payout_numerators,
        proposed_ts: None,
        expiration_ts: Some(resolved_ts),
        observed_ts: resolved_ts,
        resolved_ts: Some(resolved_ts),
        active_dispute: false,
        voided_invalid: false,
        source_tx_hash: optional_string(log, "transactionHash"),
        source_block_number: optional_hex_u64(log, "blockNumber")?,
        source_log_index: optional_hex_u64(log, "logIndex")?,
    })
}

pub fn decode_condition_resolution_data(data: &str) -> Result<(u64, Vec<u64>)> {
    let hex = data.trim().strip_prefix("0x").unwrap_or(data.trim());
    if hex.len() < 192 || !hex.len().is_multiple_of(64) {
        return Err(log_error("ConditionResolution data must be ABI words"));
    }
    let outcome_slot_count = parse_word_u64(&hex[0..64], "outcomeSlotCount")?;
    let offset = parse_word_u64(&hex[64..128], "payoutNumerators offset")?;
    if offset % 32 != 0 || offset < 64 {
        return Err(log_error(
            "payoutNumerators offset is not a valid ABI word offset",
        ));
    }
    let len_start = usize::try_from(offset)
        .ok()
        .and_then(|offset| offset.checked_mul(2))
        .ok_or_else(|| log_error("payoutNumerators offset overflows"))?;
    if len_start + 64 > hex.len() {
        return Err(log_error("payoutNumerators length word is outside data"));
    }
    let payout_len = parse_word_u64(&hex[len_start..len_start + 64], "payoutNumerators length")?;
    if payout_len != outcome_slot_count {
        return Err(log_error(format!(
            "outcomeSlotCount {outcome_slot_count} != payout length {payout_len}"
        )));
    }
    let mut payouts = Vec::new();
    let mut cursor = len_start + 64;
    for index in 0..payout_len {
        if cursor + 64 > hex.len() {
            return Err(log_error(format!(
                "payoutNumerators[{index}] missing ABI word"
            )));
        }
        payouts.push(parse_word_u64(
            &hex[cursor..cursor + 64],
            "payoutNumerators value",
        )?);
        cursor += 64;
    }
    Ok((outcome_slot_count, payouts))
}

fn validate_observation(observation: &UmaResolutionObservation) -> Result<()> {
    if observation.condition_id.trim().is_empty() || observation.oracle.trim().is_empty() {
        return Err(uma_error(
            ERR_UMA_RESOLUTION_INVALID,
            "UMA observation requires non-empty condition_id and oracle",
        ));
    }
    if let (Some(proposed), Some(expiration)) = (observation.proposed_ts, observation.expiration_ts)
        && expiration < proposed
    {
        return Err(uma_error(
            ERR_UMA_RESOLUTION_INVALID,
            "UMA expiration_ts cannot precede proposed_ts",
        ));
    }
    if !observation.payout_numerators.is_empty()
        && observation.payout_numerators.len() != observation.outcome_labels.len()
    {
        return Err(uma_error(
            ERR_UMA_RESOLUTION_INVALID,
            "payout_numerators must have one label per outcome",
        ));
    }
    Ok(())
}

fn finality_state(observation: &UmaResolutionObservation, liveness: u64) -> UmaFinalityState {
    if observation.voided_invalid || non_single_winner(&observation.payout_numerators) {
        return UmaFinalityState::VoidedInvalid;
    }
    if observation.active_dispute {
        return UmaFinalityState::Disputed;
    }
    if !observation.payout_numerators.is_empty() {
        return UmaFinalityState::Finalized;
    }
    if liveness > 0 {
        return UmaFinalityState::InLiveness;
    }
    UmaFinalityState::Proposed
}

fn finalized_resolution(
    observation: &UmaResolutionObservation,
) -> Result<(Option<Resolution>, bool, String)> {
    let winner = observation
        .payout_numerators
        .iter()
        .position(|value| *value > 0)
        .ok_or_else(|| uma_error(ERR_UMA_RESOLUTION_INVALID, "missing winning payout"))?;
    let resolved_ts = observation.resolved_ts.ok_or_else(|| {
        uma_error(
            ERR_UMA_RESOLUTION_INVALID,
            "final UMA observation requires resolved_ts",
        )
    })?;
    let winning_label = observation
        .outcome_labels
        .get(winner)
        .cloned()
        .ok_or_else(|| uma_error(ERR_UMA_RESOLUTION_INVALID, "winner has no outcome label"))?;
    Ok((
        Some(Resolution {
            condition_id: observation.condition_id.clone(),
            winning_outcome_index: winner as u32,
            winning_label,
            resolved_ts,
            source: UMA_ONCHAIN_SOURCE.to_string(),
            disputed: false,
        }),
        false,
        "UMA/CTF payout vector is finalized, non-disputed, and single-winner".to_string(),
    ))
}

fn non_single_winner(payouts: &[u64]) -> bool {
    !payouts.is_empty() && payouts.iter().filter(|value| **value > 0).count() != 1
}

fn liveness_remaining(observation: &UmaResolutionObservation) -> u64 {
    observation
        .expiration_ts
        .map(|expiration| expiration.saturating_sub(observation.observed_ts))
        .unwrap_or_default()
}

fn oracle_risk_for(
    state: UmaFinalityState,
    active_dispute: bool,
    liveness: u64,
) -> OracleRiskEvidence {
    OracleRiskEvidence {
        oracle: "uma".to_string(),
        dispute_risk: match state {
            UmaFinalityState::Disputed | UmaFinalityState::VoidedInvalid => 1.0,
            UmaFinalityState::InLiveness => 0.5,
            UmaFinalityState::Proposed => 0.2,
            UmaFinalityState::Finalized => 0.0,
        },
        active_dispute,
        liveness_seconds_remaining: liveness as f64,
    }
}

fn optional_string(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(str::to_string)
}

fn optional_hex_u64(value: &Value, field: &str) -> Result<Option<u64>> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(|text| parse_hex_u64(text, field))
        .transpose()
}

fn address_from_topic(topic: &str) -> Result<String> {
    let hex = topic.trim().strip_prefix("0x").unwrap_or(topic.trim());
    if hex.len() != 64 {
        return Err(log_error("indexed address topic is not 32 bytes"));
    }
    Ok(format!("0x{}", &hex[24..64]))
}

fn parse_word_u64(word: &str, name: &str) -> Result<u64> {
    if word.len() != 64 || !word.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(log_error(format!("{name} is not a 32-byte ABI word")));
    }
    if word[..48].bytes().any(|b| b != b'0') {
        return Err(log_error(format!("{name} exceeds u64")));
    }
    u64::from_str_radix(&word[48..64], 16).map_err(|err| log_error(format!("{name}: {err}")))
}

fn parse_hex_u64(text: &str, name: &str) -> Result<u64> {
    u64::from_str_radix(text.trim_start_matches("0x"), 16)
        .map_err(|err| log_error(format!("{name} is not hex u64: {err}")))
}

fn is_hex_word(text: &str) -> bool {
    text.strip_prefix("0x")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn log_error(message: impl Into<String>) -> PolyError {
    uma_error(ERR_UMA_RESOLUTION_LOG_INVALID, message)
}

fn uma_error(code: impl Into<String>, message: impl Into<String>) -> PolyError {
    PolyError::grounding(code, message)
}
