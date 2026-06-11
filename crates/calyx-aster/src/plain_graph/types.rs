use calyx_core::{CxId, Seq};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlainGraphDirection {
    Out,
    In,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraverseOptions<'a> {
    pub edge_type: Option<&'a str>,
    pub direction: PlainGraphDirection,
    pub max_hops: usize,
    pub cost_cap: usize,
}

impl Default for TraverseOptions<'_> {
    fn default() -> Self {
        Self {
            edge_type: None,
            direction: PlainGraphDirection::Out,
            max_hops: 3,
            cost_cap: 10_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlainGraphEdge {
    pub src: CxId,
    pub dst: CxId,
    pub edge_type: String,
    pub value: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlainGraphCsr {
    pub collection: String,
    pub source_snapshot: Seq,
    pub nodes: Vec<CxId>,
    pub offsets: Vec<usize>,
    pub edges: Vec<PlainGraphCsrEdge>,
    pub association_edge_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlainGraphCsrEdge {
    pub dst: CxId,
    pub edge_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphEdgeCommit {
    pub seq: Seq,
    pub edge_key: Vec<u8>,
    pub reverse_key: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsrCommit {
    pub seq: Seq,
    pub key: Vec<u8>,
    pub projection: PlainGraphCsr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DecodedEdge {
    pub src: CxId,
    pub dst: CxId,
    pub edge_type: String,
}
