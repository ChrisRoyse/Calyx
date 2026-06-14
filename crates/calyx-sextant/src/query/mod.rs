//! Query surfaces for Stage 4 search and PH55 cross-model planning.

pub mod ask;
pub mod executor;
pub mod planner;
mod search;

use calyx_aster::collection::{Collection, IsolationLevel, SecondaryIndexSpec};
use calyx_aster::layers::{RecordKey, Row};
use calyx_core::LedgerRef;
use calyx_core::{CxId, LensId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use ask::{AskResult, ask};
pub use executor::execute;
pub use planner::{DEFAULT_COST_CAP_MS, plan};
pub use search::*;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UniversalQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relational: Option<RelationalFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<DocFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kv: Option<KvLookup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeseries: Option<TsRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_hop: Option<GraphHop>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector: Option<VectorQuery>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate: Option<AggSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<AskSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_cap_ms: Option<u32>,
    #[serde(default)]
    pub explain: bool,
    #[serde(default = "default_isolation")]
    pub isolation: IsolationLevel,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RelationalFilter {
    pub collection: Collection,
    #[serde(default)]
    pub predicates: Vec<FieldPredicate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_rows: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldPredicate {
    pub field: String,
    pub op: FieldOp,
    pub value: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldOp {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    Contains,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocFilter {
    pub collection: Collection,
    pub path: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_docs: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocPathFilter {
    pub path: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KvLookup {
    pub ns: String,
    pub key: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TsRange {
    pub series: String,
    pub start: i64,
    pub end: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_points: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphHop {
    pub from_cx_ids: Vec<CxId>,
    pub hop_kind: String,
    pub max_hops: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VectorQuery {
    pub lens_ids: Vec<LensId>,
    pub query_vec: Vec<f32>,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggSpec {
    pub op: AggOp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggOp {
    Count,
    Sum,
    Min,
    Max,
    Avg,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskSpec {
    pub question: String,
    #[serde(default)]
    pub context_cx_ids: Vec<CxId>,
    #[serde(default = "default_ask_top_k")]
    pub top_k: usize,
    #[serde(default)]
    pub oracle: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanStep {
    RelationalScan {
        collection: Collection,
        filter: Vec<FieldPredicate>,
        index: Option<SecondaryIndexSpec>,
    },
    DocScan {
        collection: Collection,
        path_filter: DocPathFilter,
    },
    KvGet {
        ns: String,
        key: Vec<u8>,
    },
    TsRangeScan {
        series: String,
        start: i64,
        end: i64,
    },
    GraphHop {
        from_cx_ids: Vec<CxId>,
        hop_kind: String,
    },
    VectorFusion {
        lens_ids: Vec<LensId>,
        query_vec: Vec<f32>,
        limit: usize,
    },
    Aggregate {
        spec: AggSpec,
    },
    Ask {
        question: String,
        context_cx_ids: Vec<CxId>,
        top_k: usize,
        oracle: bool,
    },
}

impl PlanStep {
    pub fn kind(&self) -> PlanStepKind {
        match self {
            Self::RelationalScan { .. } => PlanStepKind::RelationalScan,
            Self::DocScan { .. } => PlanStepKind::DocScan,
            Self::KvGet { .. } => PlanStepKind::KvGet,
            Self::TsRangeScan { .. } => PlanStepKind::TsRangeScan,
            Self::GraphHop { .. } => PlanStepKind::GraphHop,
            Self::VectorFusion { .. } => PlanStepKind::VectorFusion,
            Self::Aggregate { .. } => PlanStepKind::Aggregate,
            Self::Ask { .. } => PlanStepKind::Ask,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepKind {
    RelationalScan,
    DocScan,
    KvGet,
    TsRangeScan,
    GraphHop,
    VectorFusion,
    Aggregate,
    Ask,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossModelPlan {
    pub steps: Vec<PlanStep>,
    pub estimated_cost_ms: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explain: Option<ExplainOutput>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExplainOutput {
    pub steps: Vec<ExplainStep>,
    pub total_cost_ms: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExplainStep {
    pub ordinal: usize,
    pub kind: PlanStepKind,
    pub estimated_cost_ms: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chosen_index: Option<SecondaryIndexSpec>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueryResult {
    pub rows: Vec<ProvenancedRow>,
    pub total_scanned: u64,
    pub elapsed_ms: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explain: Option<ExplainOutput>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProvenancedRow {
    pub key: RecordKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Row>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ledger_ref: Option<LedgerRef>,
}

const fn default_isolation() -> IsolationLevel {
    IsolationLevel::ReadCommitted
}

pub const DEFAULT_ASK_TOP_K: usize = 10;

const fn default_ask_top_k() -> usize {
    DEFAULT_ASK_TOP_K
}

impl Default for UniversalQuery {
    fn default() -> Self {
        Self {
            relational: None,
            document: None,
            kv: None,
            timeseries: None,
            graph_hop: None,
            vector: None,
            aggregate: None,
            ask: None,
            cost_cap_ms: None,
            explain: false,
            isolation: default_isolation(),
        }
    }
}
