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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlainGraphEdgeWeightStats {
    pub explicit_weight_edges: usize,
    pub legacy_unit_weight_edges: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DecodedEdge {
    pub src: CxId,
    pub dst: CxId,
    pub edge_type: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EdgeWeightPolicy {
    Strict,
    LegacyUnit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ParsedEdgeWeight {
    pub raw: f32,
    pub source: EdgeWeightSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EdgeWeightSource {
    Explicit,
    LegacyUnit,
}

pub fn plain_graph_edge_raw_weight(value: &[u8]) -> Result<f32> {
    plain_graph_edge_raw_weight_with_policy(value, EdgeWeightPolicy::Strict)
        .map(|parsed| parsed.raw)
}

pub(super) fn plain_graph_edge_raw_weight_with_policy(
    value: &[u8],
    policy: EdgeWeightPolicy,
) -> Result<ParsedEdgeWeight> {
    let value = trim_ascii_whitespace(value);
    if value.is_empty() {
        return Err(graph_corrupt(
            "graph edge value is empty; scoring edges require a positive JSON weight",
        ));
    }
    if policy == EdgeWeightPolicy::LegacyUnit {
        return legacy_unit_edge_raw_weight(value);
    }
    let json: Value = match serde_json::from_slice(value) {
        Ok(json) => json,
        Err(error) => {
            return Err(graph_corrupt(format!(
                "graph edge value must be a JSON number or object with numeric weight: {error}"
            )));
        }
    };
    parsed_json_edge_raw_weight(json, policy)
}

fn legacy_unit_edge_raw_weight(value: &[u8]) -> Result<ParsedEdgeWeight> {
    if looks_like_json_number(value) {
        let raw = std::str::from_utf8(value)
            .ok()
            .and_then(|text| text.parse::<f64>().ok())
            .ok_or_else(|| {
                graph_corrupt(
                    "graph edge value must be a JSON number or object with numeric weight",
                )
            })?;
        return validate_parsed_edge_weight(raw as f32, EdgeWeightSource::Explicit);
    }
    if value.first() == Some(&b'{') && contains_weight_key(value) {
        let json: Value = serde_json::from_slice(value).map_err(|error| {
            graph_corrupt(format!(
                "graph edge value must be a JSON number or object with numeric weight: {error}"
            ))
        })?;
        return parsed_json_edge_raw_weight(json, EdgeWeightPolicy::LegacyUnit);
    }
    Ok(ParsedEdgeWeight {
        raw: 1.0,
        source: EdgeWeightSource::LegacyUnit,
    })
}

fn parsed_json_edge_raw_weight(json: Value, policy: EdgeWeightPolicy) -> Result<ParsedEdgeWeight> {
    let raw = match json {
        Value::Number(number) => number.as_f64(),
        Value::Object(object) => {
            if let Some(weight) = object.get("weight") {
                weight.as_f64().or(Some(f64::NAN))
            } else if policy == EdgeWeightPolicy::LegacyUnit {
                return Ok(ParsedEdgeWeight {
                    raw: 1.0,
                    source: EdgeWeightSource::LegacyUnit,
                });
            } else {
                None
            }
        }
        _ if policy == EdgeWeightPolicy::LegacyUnit => {
            return Ok(ParsedEdgeWeight {
                raw: 1.0,
                source: EdgeWeightSource::LegacyUnit,
            });
        }
        _ => None,
    }
    .ok_or_else(|| {
        graph_corrupt("graph edge value must carry a positive numeric evidence weight")
    })?;
    validate_parsed_edge_weight(raw as f32, EdgeWeightSource::Explicit)
}

fn validate_parsed_edge_weight(raw: f32, source: EdgeWeightSource) -> Result<ParsedEdgeWeight> {
    if raw.is_finite() && raw > 0.0 {
        Ok(ParsedEdgeWeight { raw, source })
    } else {
        Err(graph_corrupt(format!(
            "graph edge evidence weight must be finite and > 0; got {raw}"
        )))
    }
}

fn trim_ascii_whitespace(mut value: &[u8]) -> &[u8] {
    while let Some((first, rest)) = value.split_first() {
        if !first.is_ascii_whitespace() {
            break;
        }
        value = rest;
    }
    while let Some((last, rest)) = value.split_last() {
        if !last.is_ascii_whitespace() {
            break;
        }
        value = rest;
    }
    value
}

fn looks_like_json_number(value: &[u8]) -> bool {
    matches!(value.first(), Some(b'-' | b'0'..=b'9'))
}

fn contains_weight_key(value: &[u8]) -> bool {
    value
        .windows(br#""weight""#.len())
        .any(|window| window == br#""weight""#)
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
