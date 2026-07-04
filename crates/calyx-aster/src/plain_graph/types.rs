use calyx_core::{CxId, Result, Seq};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::key::graph_corrupt;

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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlainGraphCsr {
    pub collection: String,
    pub source_snapshot: Seq,
    pub nodes: Vec<CxId>,
    pub offsets: Vec<usize>,
    pub edges: Vec<PlainGraphCsrEdge>,
    pub association_edge_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlainGraphCsrEdge {
    pub dst: CxId,
    pub edge_type: String,
    pub weight: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphEdgeCommit {
    pub seq: Seq,
    pub edge_key: Vec<u8>,
    pub reverse_key: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
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

pub fn plain_graph_edge_raw_weight(value: &[u8]) -> Result<f32> {
    if value.is_empty() {
        return Err(graph_corrupt(
            "graph edge value is empty; scoring edges require a positive JSON weight",
        ));
    }
    let json: Value = serde_json::from_slice(value).map_err(|error| {
        graph_corrupt(format!(
            "graph edge value must be a JSON number or object with numeric weight: {error}"
        ))
    })?;
    let raw = match json {
        Value::Number(number) => number.as_f64(),
        Value::Object(object) => object.get("weight").and_then(Value::as_f64),
        _ => None,
    }
    .ok_or_else(|| {
        graph_corrupt("graph edge value must carry a positive numeric evidence weight")
    })?;
    let raw = raw as f32;
    if raw.is_finite() && raw > 0.0 {
        Ok(raw)
    } else {
        Err(graph_corrupt(format!(
            "graph edge evidence weight must be finite and > 0; got {raw}"
        )))
    }
}

pub fn plain_graph_normalized_edge_weight(raw: f32, max_raw: f32) -> Result<f32> {
    if !raw.is_finite() || raw <= 0.0 || !max_raw.is_finite() || max_raw <= 0.0 {
        return Err(graph_corrupt(format!(
            "graph edge weight normalization requires positive finite raw/max weights; raw={raw} max={max_raw}"
        )));
    }
    validate_plain_graph_csr_weight(raw / max_raw)
}

pub(super) fn validate_plain_graph_csr_weight(weight: f32) -> Result<f32> {
    if weight.is_finite() && weight > 0.0 && weight <= 1.0 {
        Ok(weight)
    } else {
        Err(graph_corrupt(format!(
            "CSR edge weight must be finite and in (0,1]; got {weight}"
        )))
    }
}
