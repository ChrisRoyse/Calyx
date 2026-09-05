//! Point-in-time historical forecast replay (#243).
//!
//! Historical Polymarket rows are useful only when each forecast input is fenced to a cutoff before
//! resolution. This module turns already-shaped, timestamped source artifacts into CalyxNative
//! forecast artifacts, reads those artifacts back, and only then joins finalized outcomes for
//! scoring. Terminal/closed rows and post-cutoff evidence are rejected as leakage.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use calyx_core::{Clock, FixedClock};
use calyx_ledger::{LedgerAppender, LedgerCfStore};
use serde::{Deserialize, Serialize};

use crate::calyx_native::{
    CalyxNativeForecast, CalyxNativeRequest, produce_calyx_native_forecast,
    read_calyx_native_forecast, write_calyx_native_forecast,
};
use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};
use crate::model::{MarketSnapshot, Resolution};
use crate::score::{
    ForecastScoreManifest, ForecastScoreRequest, ForecastSource, ResolvedOutcome,
    write_forecast_score_artifacts,
};

pub const HISTORICAL_REPLAY_SCHEMA_VERSION: &str = "poly.historical_point_in_time_replay.v1";
pub const ERR_HISTORICAL_REPLAY_TERMINAL_INPUT: &str =
    "CALYX_POLY_HISTORICAL_REPLAY_TERMINAL_INPUT";
pub const ERR_HISTORICAL_REPLAY_POST_CUTOFF_INPUT: &str =
    "CALYX_POLY_HISTORICAL_REPLAY_POST_CUTOFF_INPUT";
pub const ERR_HISTORICAL_REPLAY_MISSING_OUTCOME: &str =
    "CALYX_POLY_HISTORICAL_REPLAY_MISSING_OUTCOME";
pub const ERR_HISTORICAL_REPLAY_NO_CLEAN_WINNER: &str =
    "CALYX_POLY_HISTORICAL_REPLAY_NO_CLEAN_WINNER";
pub const ERR_HISTORICAL_REPLAY_DUPLICATE: &str = "CALYX_POLY_HISTORICAL_REPLAY_DUPLICATE";
pub const ERR_HISTORICAL_REPLAY_UNSUPPORTED_SHAPE: &str =
    "CALYX_POLY_HISTORICAL_REPLAY_UNSUPPORTED_SHAPE";
pub const ERR_HISTORICAL_REPLAY_LOOKAHEAD_OUTCOME: &str =
    "CALYX_POLY_HISTORICAL_REPLAY_LOOKAHEAD_OUTCOME";
pub const ERR_HISTORICAL_REPLAY_ARTIFACT: &str = "CALYX_POLY_HISTORICAL_REPLAY_ARTIFACT";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoricalReplaySourceRow {
    pub source_id: String,
    pub observed_ts: u64,
    pub block_number: Option<u64>,
    pub terminal: bool,
    pub includes_final_outcome: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoricalReplaySnapshot {
    pub snapshot_id: String,
    pub market_id: String,
    pub forecast_version: u32,
    pub cutoff_ts: u64,
    pub cutoff_block_number: Option<u64>,
    pub market_closed_at_cutoff: bool,
    pub binary_market: bool,
    pub snapshot: MarketSnapshot,
    pub source_rows: Vec<HistoricalReplaySourceRow>,
    pub forecast_request: CalyxNativeRequest,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoricalReplayOutcome {
    pub resolution: Resolution,
    pub clean_winner: bool,
    pub voided: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoricalReplayRequest {
    pub domain: String,
    pub scored_ts: u64,
    pub snapshots: Vec<HistoricalReplaySnapshot>,
    pub outcomes: Vec<HistoricalReplayOutcome>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoricalReplayAccepted {
    pub snapshot_id: String,
    pub condition_id: String,
    pub token_id: String,
    pub cutoff_ts: u64,
    pub resolved_ts: u64,
    pub forecast_path: String,
    pub forecast_hash: String,
    pub forecast_readback_equal: bool,
    pub outcome_joined_after_forecast_readback: bool,
    pub score_manifest: ForecastScoreManifest,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoricalReplayRejected {
    pub snapshot_id: String,
    pub code: String,
    pub message: String,
    pub artifact_written: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoricalReplayReport {
    pub schema_version: String,
    pub source_of_truth: String,
    pub domain: String,
    pub scored_ts: u64,
    pub input_snapshot_count: usize,
    pub accepted_count: usize,
    pub rejected_count: usize,
    pub accepted: Vec<HistoricalReplayAccepted>,
    pub rejected: Vec<HistoricalReplayRejected>,
}

pub fn run_historical_point_in_time_replay<S, C>(
    root: &Path,
    score_root: &Path,
    ledger: &mut LedgerAppender<S, C>,
    request: &HistoricalReplayRequest,
) -> Result<HistoricalReplayReport>
where
    S: LedgerCfStore,
    C: Clock,
{
    let outcomes = request
        .outcomes
        .iter()
        .map(|outcome| (outcome.resolution.condition_id.clone(), outcome.clone()))
        .collect::<HashMap<_, _>>();
    let mut seen_ids = HashSet::new();
    let mut seen_cutoffs = HashSet::new();
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();

    for snapshot in &request.snapshots {
        let identity = format!(
            "{}:{}:{}",
            snapshot.snapshot.condition_id, snapshot.snapshot.token_id, snapshot.cutoff_ts
        );
        if !seen_ids.insert(snapshot.snapshot_id.clone()) || !seen_cutoffs.insert(identity) {
            rejected.push(reject(
                snapshot,
                ERR_HISTORICAL_REPLAY_DUPLICATE,
                "duplicate snapshot id or condition/token/cutoff",
            ));
            continue;
        }
        match validate_snapshot(snapshot, &outcomes, &request.domain, request.scored_ts) {
            Ok(outcome) => {
                accepted.push(process_snapshot(
                    root, score_root, ledger, request, snapshot, outcome,
                )?);
            }
            Err(err) => rejected.push(HistoricalReplayRejected {
                snapshot_id: snapshot.snapshot_id.clone(),
                code: err.code().to_string(),
                message: err.message(),
                artifact_written: false,
            }),
        }
    }

    Ok(HistoricalReplayReport {
        schema_version: HISTORICAL_REPLAY_SCHEMA_VERSION.to_string(),
        source_of_truth: "cutoff-fenced snapshot artifacts, CalyxNative forecast artifacts, score artifacts, and score ledger rows".to_string(),
        domain: request.domain.clone(),
        scored_ts: request.scored_ts,
        input_snapshot_count: request.snapshots.len(),
        accepted_count: accepted.len(),
        rejected_count: rejected.len(),
        accepted,
        rejected,
    })
}

fn validate_snapshot<'a>(
    snapshot: &HistoricalReplaySnapshot,
    outcomes: &'a HashMap<String, HistoricalReplayOutcome>,
    request_domain: &str,
    scored_ts: u64,
) -> Result<&'a HistoricalReplayOutcome> {
    if !snapshot.binary_market || snapshot.snapshot.outcome_index > 1 {
        return fail(
            ERR_HISTORICAL_REPLAY_UNSUPPORTED_SHAPE,
            "only binary outcomes are supported",
        );
    }
    if snapshot.market_closed_at_cutoff
        || snapshot
            .source_rows
            .iter()
            .any(|row| row.terminal || row.includes_final_outcome)
    {
        return fail(
            ERR_HISTORICAL_REPLAY_TERMINAL_INPUT,
            "terminal closed-market or final-outcome evidence cannot be forecast input",
        );
    }
    if snapshot.snapshot.snapshot_ts > snapshot.cutoff_ts
        || snapshot
            .source_rows
            .iter()
            .any(|row| row.observed_ts > snapshot.cutoff_ts)
        || snapshot.source_rows.iter().any(|row| {
            match (snapshot.cutoff_block_number, row.block_number) {
                (Some(cutoff_block), Some(block_number)) => block_number > cutoff_block,
                (None, Some(_)) => true,
                _ => false,
            }
        })
    {
        return fail(
            ERR_HISTORICAL_REPLAY_POST_CUTOFF_INPUT,
            "snapshot, source row, or source block is after the forecast cutoff",
        );
    }
    let outcome = outcomes
        .get(&snapshot.snapshot.condition_id)
        .ok_or_else(|| {
            err(
                ERR_HISTORICAL_REPLAY_MISSING_OUTCOME,
                "missing finalized outcome",
            )
        })?;
    if !outcome.clean_winner || outcome.voided || outcome.resolution.disputed {
        return fail(
            ERR_HISTORICAL_REPLAY_NO_CLEAN_WINNER,
            "outcome is void, disputed, or not a single clean winner",
        );
    }
    if outcome.resolution.resolved_ts <= snapshot.cutoff_ts {
        return fail(
            ERR_HISTORICAL_REPLAY_LOOKAHEAD_OUTCOME,
            "resolution timestamp is not after forecast cutoff",
        );
    }
    if outcome.resolution.resolved_ts > scored_ts {
        return fail(
            ERR_HISTORICAL_REPLAY_LOOKAHEAD_OUTCOME,
            "resolution timestamp is after the score timestamp",
        );
    }
    if snapshot.forecast_request.condition_id != snapshot.snapshot.condition_id
        || snapshot.forecast_request.token_id != snapshot.snapshot.token_id
        || snapshot.forecast_request.domain != request_domain
    {
        return fail(
            ERR_HISTORICAL_REPLAY_ARTIFACT,
            "forecast request does not match the cutoff snapshot identity",
        );
    }
    Ok(outcome)
}

fn process_snapshot<S, C>(
    root: &Path,
    score_root: &Path,
    ledger: &mut LedgerAppender<S, C>,
    request: &HistoricalReplayRequest,
    snapshot: &HistoricalReplaySnapshot,
    outcome: &HistoricalReplayOutcome,
) -> Result<HistoricalReplayAccepted>
where
    S: LedgerCfStore,
    C: Clock,
{
    let case_dir = root.join(sanitize(&snapshot.snapshot_id));
    let snapshot_path = write_json(&case_dir, "cutoff-snapshot.json", snapshot)?;
    let snapshot_readback: serde_json::Value = read_json(&snapshot_path)?;
    let snapshot_value = serde_json::to_value(snapshot).map_err(|serde_err| {
        err(
            ERR_HISTORICAL_REPLAY_ARTIFACT,
            format!("serialize snapshot for readback compare: {serde_err}"),
        )
    })?;
    if snapshot_readback != snapshot_value {
        return fail(ERR_HISTORICAL_REPLAY_ARTIFACT, "snapshot readback mismatch");
    }

    let forecast = produce_calyx_native_forecast(
        &snapshot.forecast_request,
        &FixedClock::new(snapshot.cutoff_ts),
    )?;
    let forecast_path = write_calyx_native_forecast(&case_dir, &forecast)?;
    let readback = read_calyx_native_forecast(&forecast_path)?;
    if readback != forecast {
        return fail(ERR_HISTORICAL_REPLAY_ARTIFACT, "forecast readback mismatch");
    }
    let forecast_hash = hash_file(&forecast_path)?;
    let score_manifest = write_forecast_score_artifacts(
        score_root,
        ledger,
        &score_request(request, snapshot, outcome, &forecast, &forecast_hash),
    )?;

    Ok(HistoricalReplayAccepted {
        snapshot_id: snapshot.snapshot_id.clone(),
        condition_id: snapshot.snapshot.condition_id.clone(),
        token_id: snapshot.snapshot.token_id.clone(),
        cutoff_ts: snapshot.cutoff_ts,
        resolved_ts: outcome.resolution.resolved_ts,
        forecast_path: forecast_path.display().to_string(),
        forecast_hash,
        forecast_readback_equal: true,
        outcome_joined_after_forecast_readback: true,
        score_manifest,
    })
}

fn score_request(
    request: &HistoricalReplayRequest,
    snapshot: &HistoricalReplaySnapshot,
    outcome: &HistoricalReplayOutcome,
    forecast: &CalyxNativeForecast,
    forecast_hash: &str,
) -> ForecastScoreRequest {
    let actual_win = snapshot.snapshot.outcome_index == outcome.resolution.winning_outcome_index;
    ForecastScoreRequest {
        score_id: short_score_id(&snapshot.snapshot_id),
        forecast_id: snapshot.snapshot_id.clone(),
        forecast_version: snapshot.forecast_version,
        current_forecast_version: snapshot.forecast_version,
        market_id: snapshot.snapshot.condition_id.clone(),
        outcome_id: snapshot.snapshot.token_id.clone(),
        source: ForecastSource::CalyxNative,
        provider: None,
        probability: forecast.p_model,
        confidence: forecast.confidence,
        forecast_ts: snapshot.cutoff_ts,
        scored_ts: request.scored_ts,
        horizon_secs: outcome.resolution.resolved_ts - snapshot.cutoff_ts,
        sufficiency_state: if forecast.admissible {
            "admissible".to_string()
        } else {
            "non_admissible".to_string()
        },
        previous_probability: None,
        forecast_artifact_hash: forecast_hash.to_string(),
        outcome: ResolvedOutcome {
            outcome_id: snapshot.snapshot.token_id.clone(),
            resolved: true,
            actual_win,
            resolved_ts: outcome.resolution.resolved_ts,
            source: outcome.resolution.source.clone(),
            version: 1,
        },
        calibration_bin_count: 10,
    }
}

fn reject(
    snapshot: &HistoricalReplaySnapshot,
    code: &str,
    message: &str,
) -> HistoricalReplayRejected {
    HistoricalReplayRejected {
        snapshot_id: snapshot.snapshot_id.clone(),
        code: code.to_string(),
        message: message.to_string(),
        artifact_written: false,
    }
}

fn fail<T>(code: &str, message: impl Into<String>) -> Result<T> {
    Err(err(code, message))
}

fn err(code: &str, message: impl Into<String>) -> PolyError {
    PolyError::diagnostics(code, message.into())
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).map_err(|err| {
        PolyError::diagnostics(
            ERR_HISTORICAL_REPLAY_ARTIFACT,
            format!("read {} for hash: {err}", path.display()),
        )
    })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn short_score_id(snapshot_id: &str) -> String {
    format!(
        "hptr{}",
        &blake3::hash(snapshot_id.as_bytes()).to_hex().to_string()[..20]
    )
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
}
