//! Local service and scheduler state scaffold (issue #124).
//!
//! This is not a live daemon launcher. It is the durable local contract for the services Poly is
//! allowed to run: ingestion, association updates, forecast generation, forecast admission, outcome
//! scoring, and scheduling. The scaffold writes per-service state files and a scheduler state file,
//! then reads every artifact back. Any forbidden trading/executor action fails closed before a
//! success-looking state file is written.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::diagnostics_store::{read_json, write_json};
use crate::{LocalOnlyPolicy, PolyAction, PolyError, Result};

pub const SERVICE_SCAFFOLD_SCHEMA_VERSION: &str = "poly.service_scaffold.v1";
pub const SERVICE_SCAFFOLD_ARTIFACT_KIND: &str = "poly_service_scaffold_manifest";
pub const SERVICE_SCAFFOLD_MANIFEST_FILE: &str = "service_scaffold_manifest.json";
pub const SERVICE_SCHEDULER_STATE_FILE: &str = "service_scheduler_state.json";

pub const ERR_SERVICE_SCAFFOLD_MISSING_CONFIG: &str = "CALYX_POLY_SERVICE_MISSING_CONFIG";
pub const ERR_SERVICE_SCAFFOLD_MALFORMED: &str = "CALYX_POLY_SERVICE_MALFORMED_CONFIG";
pub const ERR_SERVICE_SCAFFOLD_FORBIDDEN: &str = "CALYX_POLY_SERVICE_FORBIDDEN_ACTION";
pub const ERR_SERVICE_SCAFFOLD_READBACK_MISMATCH: &str = "CALYX_POLY_SERVICE_READBACK_MISMATCH";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LocalServiceKind {
    Ingestor,
    Association,
    Forecaster,
    Admission,
    Scorer,
    Scheduler,
}

impl LocalServiceKind {
    pub const REQUIRED: [Self; 6] = [
        Self::Ingestor,
        Self::Association,
        Self::Forecaster,
        Self::Admission,
        Self::Scorer,
        Self::Scheduler,
    ];

    pub const fn slug(self) -> &'static str {
        match self {
            Self::Ingestor => "ingestor",
            Self::Association => "association",
            Self::Forecaster => "forecaster",
            Self::Admission => "admission",
            Self::Scorer => "scorer",
            Self::Scheduler => "scheduler",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalServiceConfig {
    pub kind: LocalServiceKind,
    pub enabled: bool,
    pub cadence_seconds: u64,
    pub actions: Vec<PolyAction>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalServiceState {
    pub schema_version: String,
    pub service: LocalServiceKind,
    pub service_name: String,
    pub enabled: bool,
    pub cadence_seconds: u64,
    pub actions: Vec<String>,
    pub policy_decisions: Vec<String>,
    pub status: String,
    pub state_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerJobState {
    pub service: LocalServiceKind,
    pub cadence_seconds: u64,
    pub depends_on: Vec<LocalServiceKind>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerState {
    pub schema_version: String,
    pub service: LocalServiceKind,
    pub jobs: Vec<SchedulerJobState>,
    pub no_executor_service: bool,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceScaffoldManifest {
    pub schema_version: String,
    pub artifact_kind: String,
    pub emitted_at_millis: u64,
    pub service_count: usize,
    pub service_paths: Vec<String>,
    pub scheduler_path: String,
    pub no_executor_service: bool,
    pub services: Vec<LocalServiceState>,
    pub scheduler: SchedulerState,
}

pub struct ServiceScaffoldRequest<'a> {
    pub out_dir: &'a Path,
    pub emitted_at_millis: u64,
    pub policy: LocalOnlyPolicy,
    pub services: Vec<LocalServiceConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceScaffoldRun {
    pub manifest_path: PathBuf,
    pub service_paths: Vec<PathBuf>,
    pub scheduler_path: PathBuf,
    pub manifest: ServiceScaffoldManifest,
}

pub fn default_local_service_configs() -> Vec<LocalServiceConfig> {
    vec![
        service(
            LocalServiceKind::Ingestor,
            60,
            vec![PolyAction::ReadPublicData, PolyAction::IngestSnapshot],
        ),
        service(
            LocalServiceKind::Association,
            300,
            vec![PolyAction::UpdateAssociations],
        ),
        service(
            LocalServiceKind::Forecaster,
            300,
            vec![PolyAction::WriteForecastArtifact],
        ),
        service(
            LocalServiceKind::Admission,
            300,
            vec![PolyAction::AdmitForecast],
        ),
        service(
            LocalServiceKind::Scorer,
            900,
            vec![PolyAction::ScoreForecast],
        ),
        service(
            LocalServiceKind::Scheduler,
            60,
            vec![PolyAction::RunScheduler],
        ),
    ]
}

pub fn run_service_scaffold(request: &ServiceScaffoldRequest<'_>) -> Result<ServiceScaffoldRun> {
    validate_request(request)?;
    let services_dir = request.out_dir.join("services");
    let mut service_paths = Vec::new();
    let mut states = Vec::new();
    for config in &request.services {
        let state = service_state(request, config, &services_dir)?;
        let path = write_json(
            &services_dir,
            &format!("{}_state.json", config.kind.slug()),
            &state,
        )?;
        let readback = read_json::<LocalServiceState>(&path)?;
        if readback != state {
            return Err(readback_mismatch(&path));
        }
        service_paths.push(path);
        states.push(readback);
    }

    let scheduler = scheduler_state(&request.services);
    let scheduler_path = write_json(request.out_dir, SERVICE_SCHEDULER_STATE_FILE, &scheduler)?;
    let scheduler_readback = read_json::<SchedulerState>(&scheduler_path)?;
    if scheduler_readback != scheduler {
        return Err(readback_mismatch(&scheduler_path));
    }

    let manifest = ServiceScaffoldManifest {
        schema_version: SERVICE_SCAFFOLD_SCHEMA_VERSION.to_string(),
        artifact_kind: SERVICE_SCAFFOLD_ARTIFACT_KIND.to_string(),
        emitted_at_millis: request.emitted_at_millis,
        service_count: states.len(),
        service_paths: service_paths
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        scheduler_path: scheduler_path.display().to_string(),
        no_executor_service: true,
        services: states,
        scheduler: scheduler_readback,
    };
    let manifest_path = write_json(request.out_dir, SERVICE_SCAFFOLD_MANIFEST_FILE, &manifest)?;
    let manifest_readback = read_service_scaffold_manifest(&manifest_path)?;
    if manifest_readback != manifest {
        return Err(readback_mismatch(&manifest_path));
    }
    Ok(ServiceScaffoldRun {
        manifest_path,
        service_paths,
        scheduler_path,
        manifest: manifest_readback,
    })
}

pub fn read_service_scaffold_manifest(path: &Path) -> Result<ServiceScaffoldManifest> {
    read_json(path)
}

fn validate_request(request: &ServiceScaffoldRequest<'_>) -> Result<()> {
    if request.services.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_SERVICE_SCAFFOLD_MISSING_CONFIG,
            "service scaffold requires the complete local service config",
        ));
    }
    let mut seen = HashSet::new();
    for config in &request.services {
        if !seen.insert(config.kind) {
            return Err(PolyError::diagnostics(
                ERR_SERVICE_SCAFFOLD_MALFORMED,
                format!("duplicate service config for {}", config.kind.slug()),
            ));
        }
        if config.cadence_seconds == 0 || config.actions.is_empty() {
            return Err(PolyError::diagnostics(
                ERR_SERVICE_SCAFFOLD_MALFORMED,
                format!(
                    "service {} requires cadence_seconds > 0 and at least one action",
                    config.kind.slug()
                ),
            ));
        }
        for action in &config.actions {
            let decision = request.policy.enforce(*action);
            if !decision.allowed {
                return Err(PolyError::diagnostics(
                    ERR_SERVICE_SCAFFOLD_FORBIDDEN,
                    format!(
                        "service {} requested forbidden action {}: {}",
                        config.kind.slug(),
                        action.as_str(),
                        decision.reason
                    ),
                ));
            }
        }
    }
    for required in LocalServiceKind::REQUIRED {
        if !seen.contains(&required) {
            return Err(PolyError::diagnostics(
                ERR_SERVICE_SCAFFOLD_MISSING_CONFIG,
                format!("missing required local service {}", required.slug()),
            ));
        }
    }
    Ok(())
}

fn service_state(
    request: &ServiceScaffoldRequest<'_>,
    config: &LocalServiceConfig,
    services_dir: &Path,
) -> Result<LocalServiceState> {
    let state_path = services_dir.join(format!("{}_state.json", config.kind.slug()));
    let mut policy_decisions = Vec::new();
    for action in &config.actions {
        let decision = request.policy.enforce(*action);
        policy_decisions.push(decision.code);
    }
    Ok(LocalServiceState {
        schema_version: SERVICE_SCAFFOLD_SCHEMA_VERSION.to_string(),
        service: config.kind,
        service_name: format!("calyx-poly-{}", config.kind.slug()),
        enabled: config.enabled,
        cadence_seconds: config.cadence_seconds,
        actions: config
            .actions
            .iter()
            .map(|action| action.as_str().to_string())
            .collect(),
        policy_decisions,
        status: if config.enabled {
            "configured".to_string()
        } else {
            "disabled".to_string()
        },
        state_path: state_path.display().to_string(),
    })
}

fn scheduler_state(configs: &[LocalServiceConfig]) -> SchedulerState {
    let jobs = configs
        .iter()
        .filter(|config| config.kind != LocalServiceKind::Scheduler)
        .map(|config| SchedulerJobState {
            service: config.kind,
            cadence_seconds: config.cadence_seconds,
            depends_on: dependencies(config.kind),
        })
        .collect();
    SchedulerState {
        schema_version: SERVICE_SCAFFOLD_SCHEMA_VERSION.to_string(),
        service: LocalServiceKind::Scheduler,
        jobs,
        no_executor_service: true,
        status: "configured".to_string(),
    }
}

fn dependencies(kind: LocalServiceKind) -> Vec<LocalServiceKind> {
    match kind {
        LocalServiceKind::Ingestor => Vec::new(),
        LocalServiceKind::Association => vec![LocalServiceKind::Ingestor],
        LocalServiceKind::Forecaster => vec![LocalServiceKind::Association],
        LocalServiceKind::Admission => vec![LocalServiceKind::Forecaster],
        LocalServiceKind::Scorer => vec![LocalServiceKind::Admission],
        LocalServiceKind::Scheduler => Vec::new(),
    }
}

fn service(
    kind: LocalServiceKind,
    cadence_seconds: u64,
    actions: Vec<PolyAction>,
) -> LocalServiceConfig {
    LocalServiceConfig {
        kind,
        enabled: true,
        cadence_seconds,
        actions,
    }
}

fn readback_mismatch(path: &Path) -> PolyError {
    PolyError::diagnostics(
        ERR_SERVICE_SCAFFOLD_READBACK_MISMATCH,
        format!(
            "service scaffold artifact changed during readback from {}",
            path.display()
        ),
    )
}
