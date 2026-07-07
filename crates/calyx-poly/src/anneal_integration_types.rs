use calyx_anneal::{
    IndexConfig, MatPlanConfig, MetricSnapshot, ReplayQuery, ShadowVerdict, TripwireMetric,
    TripwireThresholdEntry,
};
use serde::{Deserialize, Serialize};

pub const ANNEAL_INTEGRATION_SCHEMA_VERSION: &str = "poly.anneal_integration.v1";
pub const ANNEAL_INTEGRATION_ARTIFACT_KIND: &str = "poly_anneal_integration_report";
pub const ANNEAL_INTEGRATION_REPORT_FILE: &str = "anneal_integration_report.json";
pub const ANNEAL_INTEGRATION_LEDGER_FILE: &str = "anneal_integration_ledger.jsonl";

pub const ERR_ANNEAL_INTEGRATION_INVALID_REQUEST: &str = "POLY_ANNEAL_INTEGRATION_INVALID_REQUEST";
pub const ERR_ANNEAL_INTEGRATION_MISSING_ARTIFACT: &str =
    "POLY_ANNEAL_INTEGRATION_MISSING_ARTIFACT";
pub const ERR_ANNEAL_INTEGRATION_LEDGER_IO: &str = "POLY_ANNEAL_INTEGRATION_LEDGER_IO";
pub const ERR_ANNEAL_INTEGRATION_LEDGER_DECODE: &str = "POLY_ANNEAL_INTEGRATION_LEDGER_DECODE";
pub const ERR_ANNEAL_INTEGRATION_READBACK_MISMATCH: &str =
    "POLY_ANNEAL_INTEGRATION_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnnealIntegrationArtifactRef {
    pub path: String,
    pub blake3: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnnealIntegrationParamSet {
    pub version: String,
    pub index: IndexConfig,
    pub fusion: MatPlanConfig,
    pub tau: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnnealIntegrationMetricProfile {
    pub recall_at_k: f64,
    pub guard_far: f64,
    pub guard_frr: f64,
    pub search_p99_ms: f64,
    pub ingest_p95_ms: f64,
}

impl AnnealIntegrationMetricProfile {
    pub fn metric_values(self) -> [(TripwireMetric, f64); 5] {
        [
            (TripwireMetric::RecallAtK, self.recall_at_k),
            (TripwireMetric::GuardFAR, self.guard_far),
            (TripwireMetric::GuardFRR, self.guard_frr),
            (TripwireMetric::SearchP99, self.search_p99_ms),
            (TripwireMetric::IngestP95, self.ingest_p95_ms),
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnnealIntegrationMetricRow {
    pub query_id: u64,
    pub metrics: AnnealIntegrationMetricProfile,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnnealIntegrationTripwireBounds {
    pub recall_at_k_min: f64,
    pub guard_far_max: f64,
    pub guard_frr_max: f64,
    pub search_p99_max_ms: f64,
    pub ingest_p95_max_ms: f64,
    pub hysteresis: f64,
}

impl AnnealIntegrationTripwireBounds {
    pub fn metric_bounds(self) -> [(TripwireMetric, f64); 5] {
        [
            (TripwireMetric::RecallAtK, self.recall_at_k_min),
            (TripwireMetric::GuardFAR, self.guard_far_max),
            (TripwireMetric::GuardFRR, self.guard_frr_max),
            (TripwireMetric::SearchP99, self.search_p99_max_ms),
            (TripwireMetric::IngestP95, self.ingest_p95_max_ms),
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnnealIntegrationRequest {
    pub domain: String,
    pub scope_id: String,
    pub generated_at_ts: u64,
    pub replay_seed: u64,
    pub replay_artifact: AnnealIntegrationArtifactRef,
    pub rollback_artifact: AnnealIntegrationArtifactRef,
    pub tripwire_vault: String,
    pub ledger_dir: String,
    pub current: AnnealIntegrationParamSet,
    pub candidate: AnnealIntegrationParamSet,
    pub tripwire_bounds: AnnealIntegrationTripwireBounds,
    pub replay_queries: Vec<ReplayQuery>,
    pub incumbent_metrics: Vec<AnnealIntegrationMetricRow>,
    pub candidate_metrics: Vec<AnnealIntegrationMetricRow>,
    pub budget_ticks: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnealIntegrationStatus {
    Promoted,
    Reverted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnnealIntegrationLedgerEntry {
    pub schema_version: String,
    pub sequence: u64,
    pub domain: String,
    pub scope_id: String,
    pub previous_version: String,
    pub candidate_version: String,
    pub active_version: String,
    pub status: AnnealIntegrationStatus,
    pub changed_parameters: Vec<String>,
    pub replay_hash: String,
    pub rollback_hash: String,
    pub report_hash: String,
    pub generated_at_ts: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnnealIntegrationReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub domain: String,
    pub scope_id: String,
    pub status: AnnealIntegrationStatus,
    pub reason: String,
    pub replay_query_count: usize,
    pub budget_ticks: usize,
    pub previous: AnnealIntegrationParamSet,
    pub candidate: AnnealIntegrationParamSet,
    pub active_after: AnnealIntegrationParamSet,
    pub changed_parameters: Vec<String>,
    pub shadow_verdict: ShadowVerdict,
    pub metrics: MetricSnapshot,
    pub replay_artifact: AnnealIntegrationArtifactRef,
    pub rollback_artifact: AnnealIntegrationArtifactRef,
    pub tripwire_config_path: String,
    pub tripwire_thresholds: Vec<TripwireThresholdEntry>,
    pub ledger_entry: AnnealIntegrationLedgerEntry,
    pub report_hash: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnnealIntegrationRun {
    pub report_path: std::path::PathBuf,
    pub ledger_path: std::path::PathBuf,
    pub report: AnnealIntegrationReport,
    pub ledger_entries: Vec<AnnealIntegrationLedgerEntry>,
}
