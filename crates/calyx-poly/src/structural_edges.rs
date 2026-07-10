//! Deterministic structural/arbitrage edge materialization for Graph CF (#53).

use std::collections::{BTreeMap, BTreeSet};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, CxId};

use crate::error::{PolyError, Result};
use crate::features::{negrisk_sum_residual, yes_no_residual};
use crate::model::MarketSnapshot;
pub use crate::structural_edges_types::*;

impl StructuralMarketInput {
    pub fn from_snapshot(
        cx_id: CxId,
        snapshot: &MarketSnapshot,
        expected_neg_risk_outcomes: Option<usize>,
        date_range: Option<StructuralDateRange>,
    ) -> Self {
        Self {
            cx_id,
            condition_id: snapshot.condition_id.clone(),
            token_id: snapshot.token_id.clone(),
            outcome_index: snapshot.outcome_index,
            event_id: snapshot.event_id.clone(),
            neg_risk: snapshot.neg_risk,
            expected_neg_risk_outcomes,
            price: snapshot.price,
            date_range,
        }
    }
}

pub fn compute_structural_edges(inputs: &[StructuralMarketInput]) -> Result<StructuralEdgeSet> {
    validate_inputs(inputs)?;
    let mut edges = Vec::new();
    let mut absent = Vec::new();
    let by_condition = group_by(inputs, |m| Some(m.condition_id.clone()));
    for (condition, group) in &by_condition {
        compute_yes_no_edges(condition, group, &mut edges, &mut absent)?;
    }
    let by_event = group_by(inputs, |m| m.event_id.clone());
    for (event, group) in &by_event {
        compute_sibling_edges(event, group, &mut edges, &mut absent);
        compute_negrisk_edges(event, group, &mut edges, &mut absent)?;
        compute_nested_date_edges(event, group, &mut edges);
    }
    Ok(StructuralEdgeSet {
        schema_version: STRUCTURAL_GRAPH_SCHEMA_VERSION.to_string(),
        input_count: inputs.len(),
        edge_count: edges.len(),
        absent,
        edges,
    })
}

pub fn persist_structural_edges_to_graph<C: Clock>(
    vault: &AsterVault<C>,
    collection: &str,
    inputs: &[StructuralMarketInput],
) -> Result<StructuralGraphRun> {
    let computed = compute_structural_edges(inputs)?;
    if computed.edges.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_STRUCTURAL_GRAPH_EMPTY,
            "structural graph materialization requires at least one computed edge",
        ));
    }
    let graph = PlainGraph::new(vault, collection)?;
    for input in inputs {
        graph.put_node(input.cx_id, &node_value(input)?)?;
    }
    for edge in &computed.edges {
        graph.put_edge(
            edge.src,
            &edge.edge_type,
            edge.dst,
            &serde_json::to_vec(&edge_value(edge)).map_err(|e| {
                PolyError::diagnostics(
                    ERR_STRUCTURAL_GRAPH_INVALID_INPUT,
                    format!("encode structural edge value: {e}"),
                )
            })?,
        )?;
    }
    let snapshot_seq = vault.latest_seq();
    let mut readback_edges = Vec::new();
    for edge in &computed.edges {
        let bytes = graph
            .get_edge(snapshot_seq, edge.src, &edge.edge_type, edge.dst)?
            .ok_or_else(|| {
                PolyError::diagnostics(
                    ERR_STRUCTURAL_GRAPH_READBACK_MISMATCH,
                    format!("missing Graph CF edge {} -> {}", edge.src, edge.dst),
                )
            })?;
        let value: StructuralGraphEdgeValue = serde_json::from_slice(&bytes).map_err(|e| {
            PolyError::diagnostics(
                ERR_STRUCTURAL_GRAPH_READBACK_MISMATCH,
                format!("decode Graph CF edge {} -> {}: {e}", edge.src, edge.dst),
            )
        })?;
        let expected = edge_value(edge);
        if value != expected {
            return Err(PolyError::diagnostics(
                ERR_STRUCTURAL_GRAPH_READBACK_MISMATCH,
                format!(
                    "Graph CF edge {} -> {} value mismatch expected={} actual={}",
                    edge.src,
                    edge.dst,
                    serde_json::to_string(&expected).unwrap_or_else(|_| "<encode>".to_string()),
                    serde_json::to_string(&value).unwrap_or_else(|_| "<encode>".to_string())
                ),
            ));
        }
        readback_edges.push(StructuralGraphReadback {
            src: edge.src,
            dst: edge.dst,
            edge_type: edge.edge_type.clone(),
            value,
            value_blake3: blake3::hash(&bytes).to_hex().to_string(),
        });
    }
    Ok(StructuralGraphRun {
        schema_version: STRUCTURAL_GRAPH_SCHEMA_VERSION.to_string(),
        collection: collection.to_string(),
        snapshot_seq,
        graph_cf_row_count: vault.scan_cf_at(snapshot_seq, ColumnFamily::Graph)?.len(),
        computed,
        readback_edges,
    })
}

fn compute_yes_no_edges(
    condition: &str,
    group: &[&StructuralMarketInput],
    edges: &mut Vec<StructuralEdge>,
    absent: &mut Vec<StructuralAbsence>,
) -> Result<()> {
    let yes = group.iter().find(|m| m.outcome_index == 0);
    let no = group.iter().find(|m| m.outcome_index == 1);
    let (Some(yes), Some(no)) = (yes, no) else {
        absent.push(absence(
            "single_outcome_market",
            "yes_no_complement",
            condition,
            "condition does not have both outcome_index 0 and 1",
        ));
        return Ok(());
    };
    let (Some(yes_price), Some(no_price)) = (yes.price, no.price) else {
        absent.push(absence(
            "missing_price",
            "yes_no_complement",
            condition,
            "YES/NO complement requires both outcome prices",
        ));
        return Ok(());
    };
    let residual = canonical_residual(yes_no_residual(yes_price, no_price));
    push_bidir(
        edges,
        yes.cx_id,
        no.cx_id,
        StructuralEdgeKind::YesNoComplement,
        condition.to_string(),
        Some(residual),
        residual_weight(residual),
    );
    Ok(())
}

fn compute_sibling_edges(
    event: &str,
    group: &[&StructuralMarketInput],
    edges: &mut Vec<StructuralEdge>,
    absent: &mut Vec<StructuralAbsence>,
) {
    let mut by_condition = BTreeMap::new();
    for market in group {
        by_condition
            .entry(market.condition_id.as_str())
            .or_insert(*market);
    }
    if by_condition.len() < 2 {
        absent.push(absence(
            "missing_sibling",
            "event_sibling",
            event,
            "event has fewer than two distinct market conditions",
        ));
        return;
    }
    let representatives = by_condition.values().copied().collect::<Vec<_>>();
    for_pair(&representatives, |a, b| {
        push_bidir(
            edges,
            a.cx_id,
            b.cx_id,
            StructuralEdgeKind::EventSibling,
            event.to_string(),
            None,
            1.0,
        );
    });
}

fn compute_negrisk_edges(
    event: &str,
    group: &[&StructuralMarketInput],
    edges: &mut Vec<StructuralEdge>,
    absent: &mut Vec<StructuralAbsence>,
) -> Result<()> {
    if !group.iter().any(|market| market.neg_risk) {
        return Ok(());
    }
    let mut by_condition = BTreeMap::new();
    for market in group
        .iter()
        .copied()
        .filter(|market| market.neg_risk && market.outcome_index == 0)
    {
        by_condition
            .entry(market.condition_id.as_str())
            .or_insert(market);
    }
    let neg = by_condition.values().copied().collect::<Vec<_>>();
    let expected = neg
        .iter()
        .filter_map(|m| m.expected_neg_risk_outcomes)
        .collect::<BTreeSet<_>>();
    if expected.len() != 1 || neg.len() != *expected.iter().next().unwrap_or(&0) {
        absent.push(absence(
            "incomplete_negrisk_set",
            "negrisk_sum",
            event,
            "negRisk event must carry one expected outcome count matching observed rows",
        ));
        return Ok(());
    }
    let prices = neg
        .iter()
        .map(|m| {
            m.price
                .ok_or_else(|| invalid(format!("missing negRisk price in event {event}")))
        })
        .collect::<Result<Vec<_>>>()?;
    let residual = canonical_residual(negrisk_sum_residual(&prices));
    for_pair(&neg, |a, b| {
        push_bidir(
            edges,
            a.cx_id,
            b.cx_id,
            StructuralEdgeKind::NegRiskSibling,
            event.to_string(),
            Some(residual),
            residual_weight(residual),
        );
    });
    Ok(())
}

fn compute_nested_date_edges(
    event: &str,
    group: &[&StructuralMarketInput],
    edges: &mut Vec<StructuralEdge>,
) {
    for_pair(group, |a, b| {
        let (Some(ar), Some(br)) = (a.date_range, b.date_range) else {
            return;
        };
        if contains_range(ar, br) {
            push_edge(
                edges,
                a.cx_id,
                b.cx_id,
                StructuralEdgeKind::NestedDateContains,
                event.to_string(),
                None,
                1.0,
            );
        } else if contains_range(br, ar) {
            push_edge(
                edges,
                b.cx_id,
                a.cx_id,
                StructuralEdgeKind::NestedDateContains,
                event.to_string(),
                None,
                1.0,
            );
        }
    });
}

fn validate_inputs(inputs: &[StructuralMarketInput]) -> Result<()> {
    if inputs.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_STRUCTURAL_GRAPH_INVALID_INPUT,
            "structural edge computation requires at least one market input",
        ));
    }
    let mut seen = BTreeSet::new();
    for input in inputs {
        if input.condition_id.trim().is_empty() || input.token_id.trim().is_empty() {
            return Err(invalid("condition_id and token_id must be non-empty"));
        }
        if !seen.insert(input.cx_id) {
            return Err(invalid(format!("duplicate cx_id {}", input.cx_id)));
        }
        if let Some(price) = input.price
            && (!price.is_finite() || !(0.0..=1.0).contains(&price))
        {
            return Err(invalid(format!(
                "price for {} must be finite and in [0,1]",
                input.condition_id
            )));
        }
        if let Some(range) = input.date_range
            && range.start_ts >= range.end_ts
        {
            return Err(invalid(format!(
                "date range for {} must have start < end",
                input.condition_id
            )));
        }
    }
    Ok(())
}

fn group_by<F>(
    inputs: &[StructuralMarketInput],
    key: F,
) -> BTreeMap<String, Vec<&StructuralMarketInput>>
where
    F: Fn(&StructuralMarketInput) -> Option<String>,
{
    let mut groups: BTreeMap<String, Vec<&StructuralMarketInput>> = BTreeMap::new();
    for input in inputs {
        if let Some(key) = key(input).filter(|k| !k.trim().is_empty()) {
            groups.entry(key).or_default().push(input);
        }
    }
    groups
}

fn for_pair<T>(items: &[T], mut f: impl FnMut(T, T))
where
    T: Copy,
{
    for i in 0..items.len() {
        for j in (i + 1)..items.len() {
            f(items[i], items[j]);
        }
    }
}

fn push_bidir(
    edges: &mut Vec<StructuralEdge>,
    a: CxId,
    b: CxId,
    kind: StructuralEdgeKind,
    relation_key: String,
    residual: Option<f64>,
    weight: f64,
) {
    push_edge(
        edges,
        a,
        b,
        kind.clone(),
        relation_key.clone(),
        residual,
        weight,
    );
    push_edge(edges, b, a, kind, relation_key, residual, weight);
}

fn push_edge(
    edges: &mut Vec<StructuralEdge>,
    src: CxId,
    dst: CxId,
    kind: StructuralEdgeKind,
    relation_key: String,
    residual: Option<f64>,
    weight: f64,
) {
    let edge_type = kind.edge_type().to_string();
    edges.push(StructuralEdge {
        src,
        dst,
        kind,
        edge_type,
        relation_key,
        residual,
        weight,
    });
}

fn edge_value(edge: &StructuralEdge) -> StructuralGraphEdgeValue {
    StructuralGraphEdgeValue {
        schema_version: STRUCTURAL_GRAPH_SCHEMA_VERSION.to_string(),
        kind: edge.kind.clone(),
        edge_type: edge.edge_type.clone(),
        relation_key: edge.relation_key.clone(),
        residual: edge.residual,
        weight: edge.weight,
    }
}

fn node_value(input: &StructuralMarketInput) -> Result<Vec<u8>> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": STRUCTURAL_GRAPH_SCHEMA_VERSION,
        "node_kind": "market",
        "condition_id": input.condition_id,
        "token_id": input.token_id,
        "outcome_index": input.outcome_index,
        "event_id": input.event_id,
        "neg_risk": input.neg_risk
    }))
    .map_err(|e| {
        PolyError::diagnostics(
            ERR_STRUCTURAL_GRAPH_INVALID_INPUT,
            format!("encode structural node value: {e}"),
        )
    })
}

fn contains_range(parent: StructuralDateRange, child: StructuralDateRange) -> bool {
    parent.start_ts <= child.start_ts
        && parent.end_ts >= child.end_ts
        && (parent.start_ts, parent.end_ts) != (child.start_ts, child.end_ts)
}

fn residual_weight(residual: f64) -> f64 {
    canonical_residual(1.0 / (1.0 + residual.abs()))
}

fn canonical_residual(value: f64) -> f64 {
    (value * 1.0e12).round() / 1.0e12
}

fn absence(code: &str, relation: &str, relation_key: &str, reason: &str) -> StructuralAbsence {
    StructuralAbsence {
        code: code.to_string(),
        relation: relation.to_string(),
        relation_key: relation_key.to_string(),
        reason: reason.to_string(),
    }
}

fn invalid(message: impl Into<String>) -> PolyError {
    PolyError::diagnostics(ERR_STRUCTURAL_GRAPH_INVALID_INPUT, message)
}
