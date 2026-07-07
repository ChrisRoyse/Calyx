//! Local daily ops scheduler tick for association/kernel and calibration work (issue #12).
//!
//! This is a one-tick contract, not a daemon. It persists scheduler state, skips duplicate due
//! slots before running work, and delegates the real work to the existing domain graph, Ward
//! calibration, and calibration-refit modules.

use std::path::{Path, PathBuf};

use calyx_aster::vault::AsterVault;
use calyx_core::Clock;
use serde::{Deserialize, Serialize};

use crate::calibration_refit::{
    CalibrationRefitReport, CalibrationRefitRequest, CalibrationRefitRun, run_calibration_refit,
};
use crate::crypto_ingestor::reject_forbidden_drive;
use crate::diagnostics_store::{read_json, write_json};
use crate::domain_graph_build_job::{
    DomainGraphBuildReport, DomainGraphBuildRequest, DomainGraphBuildRun,
    run_domain_graph_build_job,
};
use crate::policy::{LocalOnlyPolicy, PolyAction};
use crate::ward_calibration::{
    WardCalibrationReport, WardCalibrationRequest, WardCalibrationRun, run_ward_calibration_report,
};
use crate::{PolyError, Result};

pub const DAILY_OPS_SCHEDULER_SCHEMA_VERSION: &str = "poly.daily_ops_scheduler.v1";
pub const DAILY_OPS_SCHEDULER_STATE_FILE: &str = "daily-ops-scheduler-state.json";
pub const DAILY_OPS_SCHEDULER_REPORT_FILE: &str = "daily-ops-scheduler-report.json";

pub const ERR_DAILY_OPS_SCHEDULER_INVALID_CONFIG: &str =
    "CALYX_POLY_DAILY_OPS_SCHEDULER_INVALID_CONFIG";
pub const ERR_DAILY_OPS_SCHEDULER_READBACK: &str = "CALYX_POLY_DAILY_OPS_SCHEDULER_READBACK";
pub const ERR_DAILY_OPS_SCHEDULER_FORBIDDEN: &str = "CALYX_POLY_DAILY_OPS_SCHEDULER_FORBIDDEN";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyOpsSchedulerConfig {
    pub job_id: String,
    pub cadence_secs: u64,
}

impl Default for DailyOpsSchedulerConfig {
    fn default() -> Self {
        Self {
            job_id: "daily-ops".to_string(),
            cadence_secs: 86_400,
        }
    }
}

pub struct DailyOpsSchedulerRequest<'a, C: Clock> {
    pub output_root: &'a Path,
    pub config: DailyOpsSchedulerConfig,
    pub now_ts: u64,
    pub policy: LocalOnlyPolicy,
    pub vault: &'a AsterVault<C>,
    pub clock: &'a dyn Clock,
    pub domain_graph: DomainGraphBuildRequest<'a>,
    pub ward: WardCalibrationRequest,
    pub ward_output_dir: &'a Path,
    pub calibration_refit: CalibrationRefitRequest<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DailyOpsSchedulerDecision {
    RanDailyJobs,
    SchedulerSkippedAlreadyRan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyOpsSchedulerState {
    pub schema_version: String,
    pub job_id: String,
    pub cadence_secs: u64,
    pub last_due_slot: Option<u64>,
    pub tick_count: u64,
    pub job_invocation_count: u64,
    pub last_decision: Option<DailyOpsSchedulerDecision>,
    pub domain_graph_report_path: Option<String>,
    pub ward_report_path: Option<String>,
    pub calibration_refit_report_path: Option<String>,
}

impl Default for DailyOpsSchedulerState {
    fn default() -> Self {
        Self {
            schema_version: DAILY_OPS_SCHEDULER_SCHEMA_VERSION.to_string(),
            job_id: String::new(),
            cadence_secs: 0,
            last_due_slot: None,
            tick_count: 0,
            job_invocation_count: 0,
            last_decision: None,
            domain_graph_report_path: None,
            ward_report_path: None,
            calibration_refit_report_path: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DailyOpsSchedulerReport {
    pub schema_version: String,
    pub source_of_truth: String,
    pub state_path: String,
    pub decision: DailyOpsSchedulerDecision,
    pub due_slot: u64,
    pub tick_count_after: u64,
    pub job_invocation_count_after: u64,
    pub domain_graph_report: Option<DomainGraphBuildReport>,
    pub ward_report: Option<WardCalibrationReport>,
    pub calibration_refit_report: Option<CalibrationRefitReport>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DailyOpsSchedulerRun {
    pub state_path: PathBuf,
    pub report_path: PathBuf,
    pub state: DailyOpsSchedulerState,
    pub report: DailyOpsSchedulerReport,
    pub domain_graph_run: Option<DomainGraphBuildRun>,
    pub ward_run: Option<WardCalibrationRun>,
    pub calibration_refit_run: Option<CalibrationRefitRun>,
}

struct DailyOpsJobRuns {
    domain_graph: Option<DomainGraphBuildRun>,
    ward: Option<WardCalibrationRun>,
    calibration_refit: Option<CalibrationRefitRun>,
}

pub fn run_daily_ops_scheduler_tick<C: Clock>(
    request: DailyOpsSchedulerRequest<'_, C>,
) -> Result<DailyOpsSchedulerRun> {
    reject_forbidden_drive(request.output_root)?;
    reject_forbidden_drive(request.domain_graph.output_dir)?;
    reject_forbidden_drive(request.ward_output_dir)?;
    reject_forbidden_drive(request.calibration_refit.out_dir)?;
    validate_config(&request.config)?;
    enforce_scheduler_policy(&request.policy)?;

    let state_path = request.output_root.join(DAILY_OPS_SCHEDULER_STATE_FILE);
    let mut state = read_state_or_default(&state_path)?;
    validate_state_config(&state, &request.config)?;
    state.schema_version = DAILY_OPS_SCHEDULER_SCHEMA_VERSION.to_string();
    state.job_id = request.config.job_id.clone();
    state.cadence_secs = request.config.cadence_secs;
    let due_slot = request.now_ts / request.config.cadence_secs;

    let (decision, runs) = if state.last_due_slot == Some(due_slot) {
        (
            DailyOpsSchedulerDecision::SchedulerSkippedAlreadyRan,
            DailyOpsJobRuns {
                domain_graph: None,
                ward: None,
                calibration_refit: None,
            },
        )
    } else {
        let graph =
            run_domain_graph_build_job(request.vault, &request.domain_graph, request.clock)?;
        let ward =
            run_ward_calibration_report(&request.ward, request.ward_output_dir, request.clock)?;
        let calibration = run_calibration_refit(&request.calibration_refit)?;
        state.job_invocation_count += 1;
        state.domain_graph_report_path = Some(graph.report_path.display().to_string());
        state.ward_report_path = Some(ward.report_path.display().to_string());
        state.calibration_refit_report_path = Some(calibration.report_path.display().to_string());
        state.last_due_slot = Some(due_slot);
        (
            DailyOpsSchedulerDecision::RanDailyJobs,
            DailyOpsJobRuns {
                domain_graph: Some(graph),
                ward: Some(ward),
                calibration_refit: Some(calibration),
            },
        )
    };

    state.tick_count += 1;
    state.last_decision = Some(decision);
    write_state_readback(request.output_root, &state)?;
    let report = persist_report(
        request.output_root,
        &state_path,
        decision,
        due_slot,
        &state,
        &runs,
    )?;

    Ok(DailyOpsSchedulerRun {
        state_path,
        report_path: request.output_root.join(DAILY_OPS_SCHEDULER_REPORT_FILE),
        state,
        report,
        domain_graph_run: runs.domain_graph,
        ward_run: runs.ward,
        calibration_refit_run: runs.calibration_refit,
    })
}

pub fn read_daily_ops_scheduler_state(path: &Path) -> Result<DailyOpsSchedulerState> {
    read_json(path)
}

fn validate_config(config: &DailyOpsSchedulerConfig) -> Result<()> {
    if config.job_id.trim().is_empty() || config.job_id.len() > 80 || config.cadence_secs == 0 {
        return Err(PolyError::diagnostics(
            ERR_DAILY_OPS_SCHEDULER_INVALID_CONFIG,
            "daily ops scheduler requires a non-empty <=80 char job_id and cadence_secs > 0",
        ));
    }
    Ok(())
}

fn validate_state_config(
    state: &DailyOpsSchedulerState,
    config: &DailyOpsSchedulerConfig,
) -> Result<()> {
    if state.cadence_secs != 0 && state.cadence_secs != config.cadence_secs {
        return Err(PolyError::diagnostics(
            ERR_DAILY_OPS_SCHEDULER_INVALID_CONFIG,
            format!(
                "existing daily scheduler cadence {} does not match requested {}",
                state.cadence_secs, config.cadence_secs
            ),
        ));
    }
    if !state.job_id.is_empty() && state.job_id != config.job_id {
        return Err(PolyError::diagnostics(
            ERR_DAILY_OPS_SCHEDULER_INVALID_CONFIG,
            format!(
                "existing daily scheduler job_id {} does not match requested {}",
                state.job_id, config.job_id
            ),
        ));
    }
    Ok(())
}

fn enforce_scheduler_policy(policy: &LocalOnlyPolicy) -> Result<()> {
    for action in [
        PolyAction::RunScheduler,
        PolyAction::UpdateAssociations,
        PolyAction::AdmitForecast,
        PolyAction::ScoreForecast,
    ] {
        let decision = policy.enforce(action);
        if !decision.allowed {
            return Err(PolyError::policy(
                ERR_DAILY_OPS_SCHEDULER_FORBIDDEN,
                format!(
                    "daily scheduler action {} was refused: {}",
                    action.as_str(),
                    decision.reason
                ),
            ));
        }
    }
    Ok(())
}

fn read_state_or_default(path: &Path) -> Result<DailyOpsSchedulerState> {
    if path.exists() {
        read_json(path)
    } else {
        Ok(DailyOpsSchedulerState::default())
    }
}

fn write_state_readback(dir: &Path, state: &DailyOpsSchedulerState) -> Result<PathBuf> {
    let path = write_json(dir, DAILY_OPS_SCHEDULER_STATE_FILE, state)?;
    let readback: DailyOpsSchedulerState = read_json(&path)?;
    if readback != *state {
        return Err(PolyError::diagnostics(
            ERR_DAILY_OPS_SCHEDULER_READBACK,
            format!(
                "daily ops scheduler state {} did not read back as written",
                path.display()
            ),
        ));
    }
    Ok(path)
}

fn persist_report(
    dir: &Path,
    state_path: &Path,
    decision: DailyOpsSchedulerDecision,
    due_slot: u64,
    state: &DailyOpsSchedulerState,
    runs: &DailyOpsJobRuns,
) -> Result<DailyOpsSchedulerReport> {
    let report = DailyOpsSchedulerReport {
        schema_version: DAILY_OPS_SCHEDULER_SCHEMA_VERSION.to_string(),
        source_of_truth:
            "daily scheduler state/report JSON plus delegated domain graph, Ward, and calibration-refit reports"
                .to_string(),
        state_path: state_path.display().to_string(),
        decision,
        due_slot,
        tick_count_after: state.tick_count,
        job_invocation_count_after: state.job_invocation_count,
        domain_graph_report: runs.domain_graph.as_ref().map(|run| run.report.clone()),
        ward_report: runs.ward.as_ref().map(|run| run.report.clone()),
        calibration_refit_report: runs
            .calibration_refit
            .as_ref()
            .map(|run| run.report.clone()),
    };
    let path = write_json(dir, DAILY_OPS_SCHEDULER_REPORT_FILE, &report)?;
    let readback: DailyOpsSchedulerReport = read_json(&path)?;
    if readback != report {
        return Err(PolyError::diagnostics(
            ERR_DAILY_OPS_SCHEDULER_READBACK,
            format!(
                "daily ops scheduler report {} did not read back as written",
                path.display()
            ),
        ));
    }
    Ok(report)
}
