use std::collections::HashMap;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use calyx_core::{Clock, LensId, Result};
use calyx_ledger::LedgerCfStore;
use serde::{Deserialize, Serialize};

use crate::{
    AnnealFaultLedgerDetails, AnnealLedger, BudgetHandle, ChangeId, ComponentHealth, ComponentKind,
    DegradeRegistry, HealthStorage, LogicalTime,
};

mod support;

use support::{backoff_ticks, budget_exhausted, component_details, sha256, write_fault_event};

pub const CALYX_ANNEAL_FAULT_INVALID_EVENT: &str = "CALYX_ANNEAL_FAULT_INVALID_EVENT";
const DEFAULT_SIGNAL_DECAY_BITS: f64 = 0.05;
const DEFAULT_PROBE_FAILURE_THRESHOLD: u32 = 1;

pub trait FaultDetector<S>: Send + Sync
where
    S: HealthStorage,
{
    fn check(&self, registry: &DegradeRegistry<S>) -> Vec<FaultEvent>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultKind {
    Corruption,
    EndpointFailing,
    TauDrifted,
    SignalDecayed,
    StaleIndex,
    MetricsUnavailable,
    ProbeError,
}

impl FaultKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Corruption => "corruption",
            Self::EndpointFailing => "endpoint_failing",
            Self::TauDrifted => "tau_drifted",
            Self::SignalDecayed => "signal_decayed",
            Self::StaleIndex => "stale_index",
            Self::MetricsUnavailable => "metrics_unavailable",
            Self::ProbeError => "probe_error",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultEvent {
    pub component: ComponentKind,
    pub fault_kind: FaultKind,
    pub recommendation: String,
    pub observed_at: LogicalTime,
}

impl FaultEvent {
    pub fn new(
        component: ComponentKind,
        fault_kind: FaultKind,
        recommendation: impl Into<String>,
        observed_at: LogicalTime,
    ) -> Self {
        Self {
            component,
            fault_kind,
            recommendation: recommendation.into(),
            observed_at,
        }
    }

    pub fn health_transition(&self) -> ComponentHealth {
        let reason = format!("{}: {}", self.fault_kind.as_str(), self.recommendation);
        match self.fault_kind {
            FaultKind::EndpointFailing | FaultKind::ProbeError => {
                ComponentHealth::failing(self.observed_at, reason)
            }
            FaultKind::SignalDecayed => ComponentHealth::parked(self.observed_at, reason),
            FaultKind::Corruption
            | FaultKind::TauDrifted
            | FaultKind::StaleIndex
            | FaultKind::MetricsUnavailable => ComponentHealth::degraded(self.observed_at, reason),
        }
    }

    pub fn ledger_details(&self) -> AnnealFaultLedgerDetails {
        let mut details = component_details(&self.component);
        details.fault_kind = self.fault_kind.as_str().to_string();
        details.recommendation = self.recommendation.clone();
        details
    }

    fn change_id(&self) -> ChangeId {
        let mut bytes = self.component.storage_key();
        bytes.extend_from_slice(self.fault_kind.as_str().as_bytes());
        bytes.extend_from_slice(&self.observed_at.to_be_bytes());
        let digest = blake3::hash(&bytes);
        let mut raw = [0_u8; 8];
        raw.copy_from_slice(&digest.as_bytes()[..8]);
        ChangeId(u64::from_be_bytes(raw).max(1))
    }
}

pub struct FaultMonitor<S>
where
    S: HealthStorage,
{
    detectors: Vec<Box<dyn FaultDetector<S>>>,
    budget: BudgetHandle,
    pub tick_interval_ms: u64,
}

impl<S> FaultMonitor<S>
where
    S: HealthStorage,
{
    pub fn new(
        detectors: Vec<Box<dyn FaultDetector<S>>>,
        budget: BudgetHandle,
        tick_interval_ms: u64,
    ) -> Self {
        Self {
            detectors,
            budget,
            tick_interval_ms,
        }
    }

    pub fn run_once<L, C>(
        &mut self,
        registry: &mut DegradeRegistry<S>,
        ledger: &mut AnnealLedger<L, C>,
    ) -> Result<Vec<FaultEvent>>
    where
        L: LedgerCfStore,
        C: Clock,
    {
        let mut events = Vec::new();
        for detector in &self.detectors {
            if !self.budget.try_consume() {
                return Err(budget_exhausted());
            }
            for event in detector.check(registry) {
                registry.set_health(event.component.clone(), event.health_transition(), ledger)?;
                write_fault_event(ledger, &event)?;
                events.push(event);
            }
        }
        Ok(events)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChecksumEntry {
    pub path: PathBuf,
    pub sha256: [u8; 32],
}

impl ChecksumEntry {
    pub fn new(path: impl Into<PathBuf>, sha256: [u8; 32]) -> Self {
        Self {
            path: path.into(),
            sha256,
        }
    }
}

pub struct ChecksumDetector {
    pub components: Vec<(ComponentKind, ChecksumEntry)>,
    clock: Arc<dyn Clock>,
}

impl ChecksumDetector {
    pub fn new(components: Vec<(ComponentKind, ChecksumEntry)>, clock: Arc<dyn Clock>) -> Self {
        Self { components, clock }
    }
}

impl<S> FaultDetector<S> for ChecksumDetector
where
    S: HealthStorage,
{
    fn check(&self, _registry: &DegradeRegistry<S>) -> Vec<FaultEvent> {
        self.components
            .iter()
            .filter_map(|(component, entry)| match fs::read(&entry.path) {
                Ok(bytes) if sha256(&bytes) == entry.sha256 => None,
                Ok(_) | Err(_) => Some(FaultEvent::new(
                    component.clone(),
                    FaultKind::Corruption,
                    "rebuild derived artifact from base slots",
                    self.clock.now(),
                )),
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EndpointUrl(String);

impl EndpointUrl {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub trait HttpProbe: Send + Sync {
    fn probe(&self, endpoint: &EndpointUrl) -> Result<ProbeStatus>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeStatus {
    pub ok: bool,
}

pub struct LensProbeDetector {
    pub endpoints: Vec<(LensId, EndpointUrl)>,
    pub http_client: Arc<dyn HttpProbe>,
    pub failure_threshold: u32,
    clock: Arc<dyn Clock>,
    state: Mutex<HashMap<LensId, EndpointProbeState>>,
}

#[derive(Clone, Copy, Debug, Default)]
struct EndpointProbeState {
    consecutive_failures: u32,
    cooldown_ticks: u32,
}

impl LensProbeDetector {
    pub fn new(
        endpoints: Vec<(LensId, EndpointUrl)>,
        http_client: Arc<dyn HttpProbe>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            endpoints,
            http_client,
            failure_threshold: DEFAULT_PROBE_FAILURE_THRESHOLD,
            clock,
            state: Mutex::new(HashMap::new()),
        }
    }
}

impl<S> FaultDetector<S> for LensProbeDetector
where
    S: HealthStorage,
{
    fn check(&self, _registry: &DegradeRegistry<S>) -> Vec<FaultEvent> {
        let mut events = Vec::new();
        let Ok(mut state) = self.state.lock() else {
            return events;
        };
        for (lens_id, endpoint) in &self.endpoints {
            let entry = state.entry(*lens_id).or_default();
            if entry.cooldown_ticks > 0 {
                entry.cooldown_ticks -= 1;
                continue;
            }
            let result = catch_unwind(AssertUnwindSafe(|| self.http_client.probe(endpoint)));
            match result {
                Ok(Ok(status)) if status.ok => {
                    *entry = EndpointProbeState::default();
                }
                Ok(_) => push_probe_event(
                    &mut events,
                    *lens_id,
                    entry,
                    FaultKind::EndpointFailing,
                    self.clock.now(),
                    self.failure_threshold.max(1),
                ),
                Err(_) => push_probe_event(
                    &mut events,
                    *lens_id,
                    entry,
                    FaultKind::ProbeError,
                    self.clock.now(),
                    self.failure_threshold.max(1),
                ),
            }
        }
        events
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TauDriftSample {
    pub component: ComponentKind,
    pub tau: f64,
    pub far: f64,
    pub drift_tolerance: f64,
}

pub trait WardMetrics: Send + Sync {
    fn tau_drift_samples(&self) -> Result<Vec<TauDriftSample>>;
}

pub struct TauDriftDetector {
    pub ward_metrics: Arc<dyn WardMetrics>,
    pub error_component: ComponentKind,
    clock: Arc<dyn Clock>,
}

impl TauDriftDetector {
    pub fn new(
        ward_metrics: Arc<dyn WardMetrics>,
        error_component: ComponentKind,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            ward_metrics,
            error_component,
            clock,
        }
    }
}

impl<S> FaultDetector<S> for TauDriftDetector
where
    S: HealthStorage,
{
    fn check(&self, _registry: &DegradeRegistry<S>) -> Vec<FaultEvent> {
        let Ok(samples) = self.ward_metrics.tau_drift_samples() else {
            return vec![FaultEvent::new(
                self.error_component.clone(),
                FaultKind::MetricsUnavailable,
                "ward metrics unavailable",
                self.clock.now(),
            )];
        };
        samples
            .into_iter()
            .filter(|sample| sample.far > sample.tau + sample.drift_tolerance)
            .map(|sample| {
                FaultEvent::new(
                    sample.component,
                    FaultKind::TauDrifted,
                    "recalibrate guard tau",
                    self.clock.now(),
                )
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SignalSample {
    pub lens_id: LensId,
    pub bits_per_anchor: f64,
}

pub trait AssayMetrics: Send + Sync {
    fn signal_samples(&self) -> Result<Vec<SignalSample>>;
}

pub struct SignalDecayDetector {
    pub assay: Arc<dyn AssayMetrics>,
    pub threshold_bits: f64,
    pub error_component: ComponentKind,
    clock: Arc<dyn Clock>,
}

impl SignalDecayDetector {
    pub fn new(
        assay: Arc<dyn AssayMetrics>,
        error_component: ComponentKind,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            assay,
            threshold_bits: DEFAULT_SIGNAL_DECAY_BITS,
            error_component,
            clock,
        }
    }
}

impl<S> FaultDetector<S> for SignalDecayDetector
where
    S: HealthStorage,
{
    fn check(&self, _registry: &DegradeRegistry<S>) -> Vec<FaultEvent> {
        let Ok(samples) = self.assay.signal_samples() else {
            return vec![FaultEvent::new(
                self.error_component.clone(),
                FaultKind::MetricsUnavailable,
                "assay metrics unavailable",
                self.clock.now(),
            )];
        };
        samples
            .into_iter()
            .filter(|sample| sample.bits_per_anchor < self.threshold_bits)
            .map(|sample| {
                FaultEvent::new(
                    ComponentKind::lens_endpoint(sample.lens_id),
                    FaultKind::SignalDecayed,
                    "park lens endpoint below signal floor",
                    self.clock.now(),
                )
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaleEntry {
    pub component: ComponentKind,
    pub last_rebuild_at: LogicalTime,
}

pub struct StaleDetector {
    pub entries: Vec<StaleEntry>,
    pub rebuild_lag_bound_secs: u64,
    clock: Arc<dyn Clock>,
}

impl StaleDetector {
    pub fn new(
        entries: Vec<StaleEntry>,
        rebuild_lag_bound_secs: u64,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            entries,
            rebuild_lag_bound_secs,
            clock,
        }
    }
}

impl<S> FaultDetector<S> for StaleDetector
where
    S: HealthStorage,
{
    fn check(&self, _registry: &DegradeRegistry<S>) -> Vec<FaultEvent> {
        let now = self.clock.now();
        self.entries
            .iter()
            .filter(|entry| now.saturating_sub(entry.last_rebuild_at) > self.rebuild_lag_bound_secs)
            .map(|entry| {
                FaultEvent::new(
                    entry.component.clone(),
                    FaultKind::StaleIndex,
                    "rebuild stale derived artifact",
                    now,
                )
            })
            .collect()
    }
}

fn push_probe_event(
    events: &mut Vec<FaultEvent>,
    lens_id: LensId,
    state: &mut EndpointProbeState,
    kind: FaultKind,
    observed_at: LogicalTime,
    threshold: u32,
) {
    state.consecutive_failures = state.consecutive_failures.saturating_add(1);
    state.cooldown_ticks = backoff_ticks(state.consecutive_failures);
    if state.consecutive_failures >= threshold {
        events.push(FaultEvent::new(
            ComponentKind::lens_endpoint(lens_id),
            kind,
            "restore lens endpoint",
            observed_at,
        ));
    }
}
