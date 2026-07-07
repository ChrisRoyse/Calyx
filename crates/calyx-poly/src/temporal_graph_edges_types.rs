use calyx_assay::{
    AutocorrelationReport, CrossCorrelationReport, InterEventHazardReport, TEResult, Timestamp,
    TransferEntropyConfig,
};
use calyx_core::{CxId, Seq};
use serde::{Deserialize, Serialize};

pub const TEMPORAL_GRAPH_SCHEMA_VERSION: &str = "poly.temporal_graph_edges.v1";
pub const EDGE_TEMPORAL_LEAD_LAG: &str = "association.temporal_lead_lag";
pub const EDGE_TEMPORAL_TRANSFER_ENTROPY: &str = "association.temporal_transfer_entropy";
pub const EDGE_TEMPORAL_PERIODICITY: &str = "association.temporal_periodicity";
pub const EDGE_TEMPORAL_HAZARD: &str = "association.temporal_hazard";

pub const ERR_TEMPORAL_GRAPH_INVALID_INPUT: &str = "CALYX_POLY_TEMPORAL_GRAPH_INVALID_INPUT";
pub const ERR_TEMPORAL_GRAPH_INSUFFICIENT: &str = "CALYX_POLY_TEMPORAL_GRAPH_INSUFFICIENT";
pub const ERR_TEMPORAL_GRAPH_LOW_SIGNAL: &str = "CALYX_POLY_TEMPORAL_GRAPH_LOW_SIGNAL";
pub const ERR_TEMPORAL_GRAPH_NO_DIRECTION: &str = "CALYX_POLY_TEMPORAL_GRAPH_NO_DIRECTION";
pub const ERR_TEMPORAL_GRAPH_EMPTY: &str = "CALYX_POLY_TEMPORAL_GRAPH_EMPTY";
pub const ERR_TEMPORAL_GRAPH_READBACK_MISMATCH: &str =
    "CALYX_POLY_TEMPORAL_GRAPH_READBACK_MISMATCH";

pub(crate) const DEFAULT_OVERDUE_ALPHA: f64 = 0.05;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalPoint {
    pub ts: Timestamp,
    pub value: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemporalTransferEntropyConfig {
    pub window_size: usize,
    pub k: usize,
    pub bootstrap_resamples: usize,
    pub bootstrap_seed: u64,
}

impl From<TemporalTransferEntropyConfig> for TransferEntropyConfig {
    fn from(config: TemporalTransferEntropyConfig) -> Self {
        Self {
            window_size: config.window_size,
            k: config.k,
            bootstrap_resamples: config.bootstrap_resamples,
            bootstrap_seed: config.bootstrap_seed,
        }
    }
}

impl Default for TemporalTransferEntropyConfig {
    fn default() -> Self {
        let assay = TransferEntropyConfig::default();
        Self {
            window_size: assay.window_size,
            k: assay.k,
            bootstrap_resamples: assay.bootstrap_resamples,
            bootstrap_seed: assay.bootstrap_seed,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalGraphConfig {
    pub max_lag: usize,
    pub candidate_lags: Vec<usize>,
    pub overdue_alpha: f64,
    pub te_config: TemporalTransferEntropyConfig,
}

impl Default for TemporalGraphConfig {
    fn default() -> Self {
        Self {
            max_lag: 2,
            candidate_lags: vec![1, 2],
            overdue_alpha: DEFAULT_OVERDUE_ALPHA,
            te_config: TemporalTransferEntropyConfig::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalGraphRequest {
    pub domain: String,
    pub market_id: String,
    pub market_cx_id: CxId,
    pub driver_name: String,
    pub response_name: String,
    pub recurrence_name: String,
    pub driver_series: Vec<TemporalPoint>,
    pub response_series: Vec<TemporalPoint>,
    pub recurrence_event_times: Vec<f64>,
    pub now: f64,
    pub config: TemporalGraphConfig,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemporalGraphNode {
    pub cx_id: CxId,
    pub node_kind: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "assay", rename_all = "snake_case")]
pub enum TemporalGraphEvidence {
    LeadLag {
        report: CrossCorrelationReport,
    },
    TransferEntropy {
        selected: TEResult,
        sweep: Vec<TEResult>,
    },
    Periodicity {
        report: AutocorrelationReport,
    },
    Hazard {
        report: InterEventHazardReport,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalGraphEdge {
    pub src: CxId,
    pub dst: CxId,
    pub edge_type: String,
    pub relation_key: String,
    pub weight: f32,
    pub evidence: TemporalGraphEvidence,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalGraphEdgeSet {
    pub schema_version: String,
    pub domain: String,
    pub market_id: String,
    pub paired_sample_count: usize,
    pub recurrence_event_count: usize,
    pub node_count: usize,
    pub edge_count: usize,
    pub nodes: Vec<TemporalGraphNode>,
    pub edges: Vec<TemporalGraphEdge>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalGraphEdgeValue {
    pub schema_version: String,
    pub edge_type: String,
    pub relation_key: String,
    pub weight: f32,
    pub evidence: TemporalGraphEvidence,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalGraphReadback {
    pub src: CxId,
    pub dst: CxId,
    pub edge_type: String,
    pub value: TemporalGraphEdgeValue,
    pub value_blake3: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalGraphRun {
    pub schema_version: String,
    pub collection: String,
    pub domain: String,
    pub snapshot_seq: Seq,
    pub graph_cf_row_count: usize,
    pub computed: TemporalGraphEdgeSet,
    pub readback_edges: Vec<TemporalGraphReadback>,
}
