use serde::{Deserialize, Serialize};

use crate::ChangeId;

pub const DEFAULT_SOAK_QUERIES: u64 = 1_000_000;
pub const DEFAULT_SOAK_SEED: u64 = 0xABCDEF;
pub const DEFAULT_SOAK_P99_TARGET_REDUCTION: f64 = 0.20;
pub const DEFAULT_SOAK_OSCILLATION_WINDOW: u64 = 10_000;
pub const DEFAULT_SOAK_SAMPLE_INTERVAL: u64 = 1_000;
pub const CALYX_ANNEAL_SOAK_INVALID_CONFIG: &str = "CALYX_ANNEAL_SOAK_INVALID_CONFIG";
pub const CALYX_ANNEAL_SOAK_INVALID_ROW: &str = "CALYX_ANNEAL_SOAK_INVALID_ROW";
pub const CALYX_ANNEAL_SOAK_TIME_BUDGET_EXHAUSTED: &str = "CALYX_ANNEAL_SOAK_TIME_BUDGET_EXHAUSTED";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoakMode {
    Seeded,
    LiveTraffic,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoakConfig {
    pub n_queries: u64,
    pub seed: u64,
    pub mode: SoakMode,
    pub p99_target_reduction: f64,
    pub min_recall: f64,
    pub oscillation_window: u64,
    pub sample_interval: u64,
    pub max_runtime_ms: Option<u64>,
}

impl Default for SoakConfig {
    fn default() -> Self {
        Self {
            n_queries: DEFAULT_SOAK_QUERIES,
            seed: DEFAULT_SOAK_SEED,
            mode: SoakMode::Seeded,
            p99_target_reduction: DEFAULT_SOAK_P99_TARGET_REDUCTION,
            min_recall: 0.0,
            oscillation_window: DEFAULT_SOAK_OSCILLATION_WINDOW,
            sample_interval: DEFAULT_SOAK_SAMPLE_INTERVAL,
            max_runtime_ms: Some(2 * 60 * 60 * 1_000),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeededSoakProfile {
    pub baseline_p99_ns: u64,
    pub final_p99_ns: u64,
    pub recall_baseline: f64,
    pub recall_final: f64,
    pub bits_per_anchor: f64,
}

impl Default for SeededSoakProfile {
    fn default() -> Self {
        Self {
            baseline_p99_ns: 100,
            final_p99_ns: 70,
            recall_baseline: 0.95,
            recall_final: 0.95,
            bits_per_anchor: 0.40,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricSample {
    pub p99_ns: u64,
    pub recall_10: f64,
    pub query_count: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SoakMetrics {
    pub samples: Vec<MetricSample>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoakReport {
    pub baseline_p99_ns: u64,
    pub final_p99_ns: u64,
    pub p99_reduction: f64,
    pub recall_baseline: f64,
    pub recall_final: f64,
    pub oscillation_detected: bool,
    pub promotions: Vec<ChangeId>,
    pub total_queries: u64,
    pub samples: Vec<MetricSample>,
    pub gate_passed: bool,
    pub ts: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "row_kind", rename_all = "snake_case")]
pub enum SoakRowKind {
    Report { report: SoakReport },
    Sample { sample: MetricSample },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoakStoredRow {
    pub run_id: [u8; 32],
    pub row: SoakRowKind,
}
