use std::path::PathBuf;

use calyx_core::{CxId, Seq};
use serde::{Deserialize, Serialize};

use crate::domain::Domain;
use crate::kernel_recall_admission::ComputedKernelRecall;
use crate::pair_gain_gate::PairGainMaterializationRecord;

pub const DOMAIN_GRAPH_BUILD_SCHEMA_VERSION: &str = "poly.domain_graph_build_job.v1";
pub const EDGE_LOOM_AGREEMENT: &str = "association.loom_agreement";

pub const ERR_DOMAIN_GRAPH_EMPTY: &str = "CALYX_POLY_DOMAIN_GRAPH_EMPTY";
pub const ERR_DOMAIN_GRAPH_INVALID_INPUT: &str = "CALYX_POLY_DOMAIN_GRAPH_INVALID_INPUT";
pub const ERR_DOMAIN_GRAPH_READBACK_MISMATCH: &str = "CALYX_POLY_DOMAIN_GRAPH_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainGraphEdgeInput {
    pub src: CxId,
    pub dst: CxId,
    pub edge_type: String,
    pub relation_key: String,
    pub source: String,
    pub weight: f32,
    pub include_in_kernel: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainGraphEdgeValue {
    pub schema_version: String,
    pub edge_type: String,
    pub relation_key: String,
    pub source: String,
    pub weight: f32,
    pub include_in_kernel: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainGraphReadbackEdge {
    pub src: CxId,
    pub dst: CxId,
    pub edge_type: String,
    pub value: DomainGraphEdgeValue,
    pub value_blake3: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainGraphPairGainSummary {
    pub path: PathBuf,
    pub record: PairGainMaterializationRecord,
    pub interaction_eager_count: usize,
    pub interaction_lazy_count: usize,
    pub provisional_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainGraphBuildReport {
    pub schema_version: String,
    pub domain: Domain,
    pub collection: String,
    pub panel_version: u32,
    pub source_cx_ids: Vec<CxId>,
    pub graph_node_count: usize,
    pub graph_edge_count: usize,
    pub loom_edge_count: usize,
    pub supplied_edge_count: usize,
    pub kernel_edge_count: usize,
    pub disconnected_component_count: usize,
    pub graph_cf_row_count: usize,
    pub csr_node_count: usize,
    pub csr_edge_count: usize,
    pub xterm_count: usize,
    pub pair_gain: DomainGraphPairGainSummary,
    pub computed_kernel_recall: ComputedKernelRecall,
    pub readback_edges: Vec<DomainGraphReadbackEdge>,
    pub source_of_truth: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DomainGraphBuildRun {
    pub report_path: PathBuf,
    pub report: DomainGraphBuildReport,
    pub graph_snapshot_seq: Seq,
}
