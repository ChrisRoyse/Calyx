//! Local scheduler tick for the crypto capture harness (issue #12).
//!
//! This is a one-tick contract, not a daemon. A caller can run it from Task Scheduler or another
//! local supervisor. The scheduler persists its own state, skips duplicate due slots before source
//! fetch, and delegates actual capture to the #238 harness.

use std::path::{Path, PathBuf};

use calyx_core::{VaultId, VaultStore};
use serde::{Deserialize, Serialize};

use crate::crypto_capture_harness::{
    CryptoCaptureHarnessConfig, CryptoCaptureHarnessReport, CryptoCaptureHarnessRequest,
    CryptoCaptureHarnessRun, CryptoCaptureRunner, run_crypto_capture_harness_once,
};
use crate::crypto_ingestor::reject_forbidden_drive;
use crate::diagnostics_store::{read_json, write_json};
use crate::pending_forecast_register::{PendingForecastLedgerStore, PendingForecastRegister};
use crate::policy::{LocalOnlyPolicy, PolyAction};
use crate::{PolyError, Result};

pub const CRYPTO_CAPTURE_SCHEDULER_SCHEMA_VERSION: &str = "poly.crypto_capture_scheduler.v1";
pub const CRYPTO_CAPTURE_SCHEDULER_STATE_FILE: &str = "crypto-capture-scheduler-state.json";
pub const CRYPTO_CAPTURE_SCHEDULER_REPORT_FILE: &str = "crypto-capture-scheduler-report.json";

pub const ERR_CRYPTO_CAPTURE_SCHEDULER_INVALID_CONFIG: &str =
    "CALYX_POLY_CRYPTO_CAPTURE_SCHEDULER_INVALID_CONFIG";
pub const ERR_CRYPTO_CAPTURE_SCHEDULER_READBACK: &str =
    "CALYX_POLY_CRYPTO_CAPTURE_SCHEDULER_READBACK";
pub const ERR_CRYPTO_CAPTURE_SCHEDULER_FORBIDDEN: &str =
    "CALYX_POLY_CRYPTO_CAPTURE_SCHEDULER_FORBIDDEN";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CryptoCaptureSchedulerConfig {
    pub job_id: String,
    pub cadence_secs: u64,
    pub harness_config: CryptoCaptureHarnessConfig,
}

impl Default for CryptoCaptureSchedulerConfig {
    fn default() -> Self {
        Self {
            job_id: "crypto-capture".to_string(),
            cadence_secs: 60,
            harness_config: CryptoCaptureHarnessConfig::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct CryptoCaptureSchedulerRequest<'a> {
    pub vault_id: VaultId,
    pub vault_salt: &'a [u8],
    pub output_root: &'a Path,
    pub config: CryptoCaptureSchedulerConfig,
    pub now_ts: u64,
    pub policy: LocalOnlyPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CryptoCaptureSchedulerDecision {
    Captured,
    HarnessSkippedDuplicateInterval,
    SchedulerSkippedAlreadyRan,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CryptoCaptureSchedulerState {
    pub schema_version: String,
    pub job_id: String,
    pub cadence_secs: u64,
    pub last_due_slot: Option<u64>,
    pub tick_count: u64,
    pub capture_invocation_count: u64,
    pub last_decision: Option<CryptoCaptureSchedulerDecision>,
    pub harness_state_path: Option<String>,
    pub harness_report_path: Option<String>,
}

impl Default for CryptoCaptureSchedulerState {
    fn default() -> Self {
        Self {
            schema_version: CRYPTO_CAPTURE_SCHEDULER_SCHEMA_VERSION.to_string(),
            job_id: String::new(),
            cadence_secs: 0,
            last_due_slot: None,
            tick_count: 0,
            capture_invocation_count: 0,
            last_decision: None,
            harness_state_path: None,
            harness_report_path: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CryptoCaptureSchedulerReport {
    pub schema_version: String,
    pub source_of_truth: String,
    pub state_path: String,
    pub decision: CryptoCaptureSchedulerDecision,
    pub due_slot: u64,
    pub tick_count_after: u64,
    pub capture_invocation_count_after: u64,
    pub harness_report: Option<CryptoCaptureHarnessReport>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CryptoCaptureSchedulerRun {
    pub state_path: PathBuf,
    pub report_path: PathBuf,
    pub state: CryptoCaptureSchedulerState,
    pub report: CryptoCaptureSchedulerReport,
    pub harness_run: Option<CryptoCaptureHarnessRun>,
}

pub fn run_crypto_capture_scheduler_tick<S, R>(
    store: &S,
    register: &mut PendingForecastRegister,
    request: CryptoCaptureSchedulerRequest<'_>,
    runner: &mut R,
) -> Result<CryptoCaptureSchedulerRun>
where
    S: VaultStore + PendingForecastLedgerStore,
    R: CryptoCaptureRunner<S>,
{
    let output_root = request.output_root;
    reject_forbidden_drive(output_root)?;
    validate_config(&request.config)?;
    enforce_scheduler_policy(&request.policy)?;
    let state_path = output_root.join(CRYPTO_CAPTURE_SCHEDULER_STATE_FILE);
    let mut state = read_state_or_default(&state_path)?;
    validate_state_config(&state, &request.config)?;
    state.schema_version = CRYPTO_CAPTURE_SCHEDULER_SCHEMA_VERSION.to_string();
    state.job_id = request.config.job_id.clone();
    state.cadence_secs = request.config.cadence_secs;
    let due_slot = request.now_ts / request.config.cadence_secs;

    let (decision, harness_run) = if state.last_due_slot == Some(due_slot) {
        (
            CryptoCaptureSchedulerDecision::SchedulerSkippedAlreadyRan,
            None,
        )
    } else {
        let harness_root = output_root.join("crypto-capture-harness");
        let run = run_crypto_capture_harness_once(
            store,
            register,
            CryptoCaptureHarnessRequest {
                vault_id: request.vault_id,
                vault_salt: request.vault_salt,
                output_root: &harness_root,
                config: request.config.harness_config.clone(),
                now_ts: request.now_ts,
            },
            runner,
        )?;
        let decision = match run.report.decision {
            crate::crypto_capture_harness::CryptoCaptureDecisionKind::Captured => {
                CryptoCaptureSchedulerDecision::Captured
            }
            crate::crypto_capture_harness::CryptoCaptureDecisionKind::SkippedDuplicateInterval => {
                CryptoCaptureSchedulerDecision::HarnessSkippedDuplicateInterval
            }
        };
        state.capture_invocation_count += 1;
        state.harness_state_path = Some(run.state_path.display().to_string());
        state.harness_report_path = Some(run.report_path.display().to_string());
        state.last_due_slot = Some(due_slot);
        (decision, Some(run))
    };

    state.tick_count += 1;
    state.last_decision = Some(decision);
    write_state_readback(output_root, &state)?;
    let report = persist_report(
        output_root,
        &state_path,
        decision,
        due_slot,
        &state,
        &harness_run,
    )?;
    Ok(CryptoCaptureSchedulerRun {
        state_path,
        report_path: output_root.join(CRYPTO_CAPTURE_SCHEDULER_REPORT_FILE),
        state,
        report,
        harness_run,
    })
}

pub fn read_crypto_capture_scheduler_state(path: &Path) -> Result<CryptoCaptureSchedulerState> {
    read_json(path)
}

fn validate_config(config: &CryptoCaptureSchedulerConfig) -> Result<()> {
    if config.job_id.trim().is_empty() || config.job_id.len() > 80 || config.cadence_secs == 0 {
        return Err(PolyError::diagnostics(
            ERR_CRYPTO_CAPTURE_SCHEDULER_INVALID_CONFIG,
            "crypto capture scheduler requires a non-empty <=80 char job_id and cadence_secs > 0",
        ));
    }
    Ok(())
}

fn validate_state_config(
    state: &CryptoCaptureSchedulerState,
    config: &CryptoCaptureSchedulerConfig,
) -> Result<()> {
    if state.cadence_secs != 0 && state.cadence_secs != config.cadence_secs {
        return Err(PolyError::diagnostics(
            ERR_CRYPTO_CAPTURE_SCHEDULER_INVALID_CONFIG,
            format!(
                "existing scheduler cadence {} does not match requested {}",
                state.cadence_secs, config.cadence_secs
            ),
        ));
    }
    if !state.job_id.is_empty() && state.job_id != config.job_id {
        return Err(PolyError::diagnostics(
            ERR_CRYPTO_CAPTURE_SCHEDULER_INVALID_CONFIG,
            format!(
                "existing scheduler job_id {} does not match requested {}",
                state.job_id, config.job_id
            ),
        ));
    }
    Ok(())
}

fn enforce_scheduler_policy(policy: &LocalOnlyPolicy) -> Result<()> {
    for action in [
        PolyAction::RunScheduler,
        PolyAction::ReadPublicData,
        PolyAction::IngestSnapshot,
    ] {
        let decision = policy.enforce(action);
        if !decision.allowed {
            return Err(PolyError::policy(
                ERR_CRYPTO_CAPTURE_SCHEDULER_FORBIDDEN,
                format!(
                    "scheduler action {} was refused: {}",
                    action.as_str(),
                    decision.reason
                ),
            ));
        }
    }
    Ok(())
}

fn read_state_or_default(path: &Path) -> Result<CryptoCaptureSchedulerState> {
    if path.exists() {
        read_json(path)
    } else {
        Ok(CryptoCaptureSchedulerState::default())
    }
}

fn write_state_readback(dir: &Path, state: &CryptoCaptureSchedulerState) -> Result<PathBuf> {
    let path = write_json(dir, CRYPTO_CAPTURE_SCHEDULER_STATE_FILE, state)?;
    let readback: CryptoCaptureSchedulerState = read_json(&path)?;
    if readback != *state {
        return Err(PolyError::diagnostics(
            ERR_CRYPTO_CAPTURE_SCHEDULER_READBACK,
            format!(
                "crypto capture scheduler state {} did not read back as written",
                path.display()
            ),
        ));
    }
    Ok(path)
}

fn persist_report(
    dir: &Path,
    state_path: &Path,
    decision: CryptoCaptureSchedulerDecision,
    due_slot: u64,
    state: &CryptoCaptureSchedulerState,
    harness_run: &Option<CryptoCaptureHarnessRun>,
) -> Result<CryptoCaptureSchedulerReport> {
    let report = CryptoCaptureSchedulerReport {
        schema_version: CRYPTO_CAPTURE_SCHEDULER_SCHEMA_VERSION.to_string(),
        source_of_truth: "crypto capture scheduler state JSON plus delegated #238 harness state"
            .to_string(),
        state_path: state_path.display().to_string(),
        decision,
        due_slot,
        tick_count_after: state.tick_count,
        capture_invocation_count_after: state.capture_invocation_count,
        harness_report: harness_run.as_ref().map(|run| run.report.clone()),
    };
    let path = write_json(dir, CRYPTO_CAPTURE_SCHEDULER_REPORT_FILE, &report)?;
    let readback: CryptoCaptureSchedulerReport = read_json(&path)?;
    if readback != report {
        return Err(PolyError::diagnostics(
            ERR_CRYPTO_CAPTURE_SCHEDULER_READBACK,
            format!(
                "crypto capture scheduler report {} did not read back as written",
                path.display()
            ),
        ));
    }
    Ok(report)
}
