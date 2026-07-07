use calyx_core::{CxId, Seq};
use serde::{Deserialize, Serialize};

pub const STRUCTURAL_GRAPH_SCHEMA_VERSION: &str = "poly.structural_edges.v1";
pub const EDGE_YES_NO_COMPLEMENT: &str = "structural.yes_no_complement";
pub const EDGE_NEGRISK_SIBLING: &str = "structural.negrisk_sibling";
pub const EDGE_EVENT_SIBLING: &str = "structural.event_sibling";
pub const EDGE_NESTED_DATE_CONTAINS: &str = "structural.nested_date_contains";

pub const ERR_STRUCTURAL_GRAPH_INVALID_INPUT: &str = "POLY_STRUCTURAL_GRAPH_INVALID_INPUT";
pub const ERR_STRUCTURAL_GRAPH_EMPTY: &str = "POLY_STRUCTURAL_GRAPH_EMPTY";
pub const ERR_STRUCTURAL_GRAPH_READBACK_MISMATCH: &str = "POLY_STRUCTURAL_GRAPH_READBACK_MISMATCH";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuralDateRange {
    pub start_ts: u64,
    pub end_ts: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StructuralMarketInput {
    pub cx_id: CxId,
    pub condition_id: String,
    pub token_id: String,
    pub outcome_index: u32,
    pub event_id: Option<String>,
    pub neg_risk: bool,
    pub expected_neg_risk_outcomes: Option<usize>,
    pub price: Option<f64>,
    pub date_range: Option<StructuralDateRange>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuralEdgeKind {
    YesNoComplement,
    NegRiskSibling,
    EventSibling,
    NestedDateContains,
}

impl StructuralEdgeKind {
    pub fn edge_type(&self) -> &'static str {
        match self {
            Self::YesNoComplement => EDGE_YES_NO_COMPLEMENT,
            Self::NegRiskSibling => EDGE_NEGRISK_SIBLING,
            Self::EventSibling => EDGE_EVENT_SIBLING,
            Self::NestedDateContains => EDGE_NESTED_DATE_CONTAINS,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StructuralEdge {
    pub src: CxId,
    pub dst: CxId,
    pub kind: StructuralEdgeKind,
    pub edge_type: String,
    pub relation_key: String,
    pub residual: Option<f64>,
    pub weight: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuralAbsence {
    pub code: String,
    pub relation: String,
    pub relation_key: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StructuralEdgeSet {
    pub schema_version: String,
    pub input_count: usize,
    pub edge_count: usize,
    pub absent: Vec<StructuralAbsence>,
    pub edges: Vec<StructuralEdge>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StructuralGraphEdgeValue {
    pub schema_version: String,
    pub kind: StructuralEdgeKind,
    pub edge_type: String,
    pub relation_key: String,
    pub residual: Option<f64>,
    pub weight: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StructuralGraphReadback {
    pub src: CxId,
    pub dst: CxId,
    pub edge_type: String,
    pub value: StructuralGraphEdgeValue,
    pub value_blake3: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StructuralGraphRun {
    pub schema_version: String,
    pub collection: String,
    pub snapshot_seq: Seq,
    pub graph_cf_row_count: usize,
    pub computed: StructuralEdgeSet,
    pub readback_edges: Vec<StructuralGraphReadback>,
}
