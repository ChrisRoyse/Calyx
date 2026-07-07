//! Temporal Assay edge materialization into Graph CF (#51).

use std::collections::BTreeSet;

use calyx_assay::{
    AutocorrelationReport, Direction, InterEventHazardReport, MIN_HAZARD_GAPS, MIN_PEARSON_SAMPLES,
    MIN_PERIODICITY_SAMPLES, MIN_TE_QUORUM, TEResult, Timestamp, TransferEntropyConfig,
    autocorrelation, cross_correlation_profile, inter_event_hazard_with_alpha,
    transfer_entropy_sweep_with_config,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, CxId};

use crate::error::{PolyError, Result};
pub use crate::temporal_graph_edges_types::*;

const TEMPORAL_NODE_PANEL_VERSION: u32 = 51;
const TEMPORAL_NODE_SALT: &[u8] = b"poly-temporal-graph-edge-node-v1";

pub fn compute_temporal_graph_edges(
    request: &TemporalGraphRequest,
    clock: &dyn Clock,
) -> Result<TemporalGraphEdgeSet> {
    validate_request(request)?;
    let paired = paired_streams(request)?;
    let nodes = temporal_nodes(request);
    let driver_node = nodes[1].cx_id;
    let period_node = nodes[2].cx_id;
    let hazard_node = nodes[3].cx_id;

    let ccf = cross_correlation_profile(
        &paired.driver_values,
        &paired.response_values,
        request.config.max_lag,
    )
    .map_err(map_assay_error)?;
    if ccf.peak_lag <= 0 {
        return Err(PolyError::diagnostics(
            ERR_TEMPORAL_GRAPH_NO_DIRECTION,
            format!(
                "cross-correlation did not find {} leading {}",
                request.driver_name, request.response_name
            ),
        ));
    }
    let te_sweep = transfer_entropy_sweep_with_config(
        &paired.driver_stream,
        &paired.response_stream,
        &request.config.candidate_lags,
        clock,
        &TransferEntropyConfig::from(request.config.te_config),
    );
    let selected_te = select_driver_to_response(&te_sweep)?;
    let acf =
        autocorrelation(&paired.times, &paired.response_values_f64).map_err(map_assay_error)?;
    if acf.dominant_period.is_none() {
        return Err(PolyError::diagnostics(
            ERR_TEMPORAL_GRAPH_LOW_SIGNAL,
            "periodicity estimator returned no positive local maximum",
        ));
    }
    let hazard = inter_event_hazard_with_alpha(
        &request.recurrence_event_times,
        request.now,
        request.config.overdue_alpha,
    )
    .map_err(map_assay_error)?;

    let market = request.market_cx_id;
    let edges = vec![
        edge(
            driver_node,
            market,
            EDGE_TEMPORAL_LEAD_LAG,
            relation_key(request, "lead_lag"),
            ccf.peak_abs_correlation,
            TemporalGraphEvidence::LeadLag { report: ccf },
        ),
        edge(
            driver_node,
            market,
            EDGE_TEMPORAL_TRANSFER_ENTROPY,
            relation_key(request, "transfer_entropy"),
            selected_te.t_a_to_b - selected_te.t_b_to_a,
            TemporalGraphEvidence::TransferEntropy {
                selected: selected_te.clone(),
                sweep: te_sweep,
            },
        ),
        edge(
            market,
            period_node,
            EDGE_TEMPORAL_PERIODICITY,
            relation_key(request, "periodicity"),
            periodicity_weight(&acf),
            TemporalGraphEvidence::Periodicity { report: acf },
        ),
        edge(
            hazard_node,
            market,
            EDGE_TEMPORAL_HAZARD,
            relation_key(request, "hazard"),
            hazard_weight(&hazard),
            TemporalGraphEvidence::Hazard { report: hazard },
        ),
    ];
    Ok(TemporalGraphEdgeSet {
        schema_version: TEMPORAL_GRAPH_SCHEMA_VERSION.to_string(),
        domain: request.domain.clone(),
        market_id: request.market_id.clone(),
        paired_sample_count: paired.driver_values.len(),
        recurrence_event_count: request.recurrence_event_times.len(),
        node_count: nodes.len(),
        edge_count: edges.len(),
        nodes,
        edges,
    })
}

pub fn persist_temporal_graph_edges<C: Clock>(
    vault: &AsterVault<C>,
    collection: &str,
    request: &TemporalGraphRequest,
    clock: &dyn Clock,
) -> Result<TemporalGraphRun> {
    let computed = compute_temporal_graph_edges(request, clock)?;
    if computed.edges.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_TEMPORAL_GRAPH_EMPTY,
            "temporal graph materialization produced no edges",
        ));
    }
    let graph = PlainGraph::new(vault, collection)?;
    for node in &computed.nodes {
        graph.put_node(node.cx_id, &node_value(request, node)?)?;
    }
    for edge in &computed.edges {
        graph.put_edge(edge.src, &edge.edge_type, edge.dst, &edge_bytes(edge)?)?;
    }
    let snapshot_seq = vault.latest_seq();
    let mut readback_edges = Vec::new();
    for edge in &computed.edges {
        let bytes = graph
            .get_edge(snapshot_seq, edge.src, &edge.edge_type, edge.dst)?
            .ok_or_else(|| readback_error(format!("missing temporal edge {}", edge_id(edge))))?;
        let expected = edge_bytes(edge)?;
        if bytes != expected {
            return Err(readback_error(format!(
                "temporal edge {} bytes mismatch expected_blake3={} actual_blake3={}",
                edge_id(edge),
                blake3::hash(&expected).to_hex(),
                blake3::hash(&bytes).to_hex()
            )));
        }
        let value: TemporalGraphEdgeValue =
            serde_json::from_slice(&bytes).map_err(|err| readback_error(err.to_string()))?;
        readback_edges.push(TemporalGraphReadback {
            src: edge.src,
            dst: edge.dst,
            edge_type: edge.edge_type.clone(),
            value,
            value_blake3: blake3::hash(&bytes).to_hex().to_string(),
        });
    }
    Ok(TemporalGraphRun {
        schema_version: TEMPORAL_GRAPH_SCHEMA_VERSION.to_string(),
        collection: collection.to_string(),
        domain: request.domain.clone(),
        snapshot_seq,
        graph_cf_row_count: vault.scan_cf_at(snapshot_seq, ColumnFamily::Graph)?.len(),
        computed,
        readback_edges,
    })
}

pub fn temporal_signal_cx_id(request: &TemporalGraphRequest, node_kind: &str, label: &str) -> CxId {
    let input = format!(
        "poly:temporal:{}:{}:{}:{}:{}",
        request.domain, request.market_id, request.market_cx_id, node_kind, label
    );
    CxId::from_input(
        input.as_bytes(),
        TEMPORAL_NODE_PANEL_VERSION,
        TEMPORAL_NODE_SALT,
    )
}

fn validate_request(request: &TemporalGraphRequest) -> Result<()> {
    if request.domain.trim().is_empty()
        || request.market_id.trim().is_empty()
        || request.driver_name.trim().is_empty()
        || request.response_name.trim().is_empty()
        || request.recurrence_name.trim().is_empty()
    {
        return invalid("domain, market_id, driver, response, and recurrence names are required");
    }
    if request.config.max_lag == 0 || request.config.candidate_lags.is_empty() {
        return invalid("max_lag and at least one positive candidate lag are required");
    }
    if !request.config.overdue_alpha.is_finite()
        || !(0.0..1.0).contains(&request.config.overdue_alpha)
    {
        return invalid("overdue_alpha must be finite and in [0, 1)");
    }
    if request.config.te_config.window_size == 0
        || request.config.te_config.k == 0
        || request.config.te_config.bootstrap_resamples == 0
    {
        return invalid("TE window_size, k, and bootstrap_resamples must be positive");
    }
    let mut seen_lags = BTreeSet::new();
    for lag in &request.config.candidate_lags {
        if *lag == 0 || *lag > request.config.max_lag {
            return invalid("candidate lags must be positive and <= max_lag");
        }
        if !seen_lags.insert(*lag) {
            return invalid(format!("duplicate candidate lag {lag}"));
        }
    }
    let max_lag = *request.config.candidate_lags.iter().max().unwrap_or(&0);
    let min_samples = (MIN_TE_QUORUM + max_lag).max(MIN_PERIODICITY_SAMPLES);
    if request.driver_series.len() < min_samples || request.response_series.len() < min_samples {
        return Err(PolyError::diagnostics(
            ERR_TEMPORAL_GRAPH_INSUFFICIENT,
            format!("temporal graph requires at least {min_samples} paired samples"),
        ));
    }
    if request.recurrence_event_times.len() < MIN_HAZARD_GAPS + 1 {
        return Err(PolyError::diagnostics(
            ERR_TEMPORAL_GRAPH_INSUFFICIENT,
            format!(
                "temporal hazard requires at least {} recurrence events",
                MIN_HAZARD_GAPS + 1
            ),
        ));
    }
    Ok(())
}

struct PairedStreams {
    driver_stream: Vec<(Timestamp, f32)>,
    response_stream: Vec<(Timestamp, f32)>,
    driver_values: Vec<f32>,
    response_values: Vec<f32>,
    response_values_f64: Vec<f64>,
    times: Vec<f64>,
}

fn paired_streams(request: &TemporalGraphRequest) -> Result<PairedStreams> {
    if request.driver_series.len() != request.response_series.len() {
        return invalid(format!(
            "paired series length mismatch: driver={} response={}",
            request.driver_series.len(),
            request.response_series.len()
        ));
    }
    let mut driver_stream = Vec::with_capacity(request.driver_series.len());
    let mut response_stream = Vec::with_capacity(request.response_series.len());
    let mut driver_values = Vec::with_capacity(request.driver_series.len());
    let mut response_values = Vec::with_capacity(request.response_series.len());
    let mut response_values_f64 = Vec::with_capacity(request.response_series.len());
    let mut times = Vec::with_capacity(request.driver_series.len());
    let mut previous_ts = None;
    for (index, (driver, response)) in request
        .driver_series
        .iter()
        .zip(&request.response_series)
        .enumerate()
    {
        if driver.ts != response.ts {
            return invalid(format!(
                "paired timestamp mismatch at index {index}: driver={} response={}",
                driver.ts, response.ts
            ));
        }
        if let Some(previous) = previous_ts
            && driver.ts != previous + 1
        {
            return invalid(format!(
                "paired timestamps must be contiguous unit-spaced bins; index {index} has {} after {previous}",
                driver.ts
            ));
        }
        if !driver.value.is_finite() || !response.value.is_finite() {
            return invalid(format!("non-finite paired value at index {index}"));
        }
        previous_ts = Some(driver.ts);
        driver_stream.push((driver.ts, driver.value));
        response_stream.push((response.ts, response.value));
        driver_values.push(driver.value);
        response_values.push(response.value);
        response_values_f64.push(response.value as f64);
        times.push(driver.ts as f64);
    }
    ensure_variance("driver", &driver_values)?;
    ensure_variance("response", &response_values)?;
    Ok(PairedStreams {
        driver_stream,
        response_stream,
        driver_values,
        response_values,
        response_values_f64,
        times,
    })
}

fn select_driver_to_response(sweep: &[TEResult]) -> Result<&TEResult> {
    sweep
        .iter()
        .filter(|result| {
            !result.provisional
                && result.error_code.is_none()
                && result.dominant_direction == Direction::AToB
                && result.t_a_to_b > result.t_b_to_a
        })
        .max_by(|left, right| {
            (left.t_a_to_b - left.t_b_to_a).total_cmp(&(right.t_a_to_b - right.t_b_to_a))
        })
        .ok_or_else(|| {
            PolyError::diagnostics(
                ERR_TEMPORAL_GRAPH_NO_DIRECTION,
                "transfer entropy sweep found no decisive driver -> response signal",
            )
        })
}

fn temporal_nodes(request: &TemporalGraphRequest) -> Vec<TemporalGraphNode> {
    vec![
        TemporalGraphNode {
            cx_id: request.market_cx_id,
            node_kind: "market_response".to_string(),
            label: request.response_name.clone(),
        },
        TemporalGraphNode {
            cx_id: temporal_signal_cx_id(request, "driver", &request.driver_name),
            node_kind: "driver_signal".to_string(),
            label: request.driver_name.clone(),
        },
        TemporalGraphNode {
            cx_id: temporal_signal_cx_id(request, "periodicity", &request.response_name),
            node_kind: "periodicity_signal".to_string(),
            label: request.response_name.clone(),
        },
        TemporalGraphNode {
            cx_id: temporal_signal_cx_id(request, "hazard", &request.recurrence_name),
            node_kind: "hazard_signal".to_string(),
            label: request.recurrence_name.clone(),
        },
    ]
}

fn edge(
    src: CxId,
    dst: CxId,
    edge_type: &str,
    relation_key: String,
    weight: f32,
    evidence: TemporalGraphEvidence,
) -> TemporalGraphEdge {
    TemporalGraphEdge {
        src,
        dst,
        edge_type: edge_type.to_string(),
        relation_key,
        weight: positive_weight(weight),
        evidence,
    }
}

fn edge_bytes(edge: &TemporalGraphEdge) -> Result<Vec<u8>> {
    serde_json::to_vec(&edge_value(edge)).map_err(|err| {
        PolyError::diagnostics(
            ERR_TEMPORAL_GRAPH_READBACK_MISMATCH,
            format!("encode temporal graph edge: {err}"),
        )
    })
}

fn edge_value(edge: &TemporalGraphEdge) -> TemporalGraphEdgeValue {
    TemporalGraphEdgeValue {
        schema_version: TEMPORAL_GRAPH_SCHEMA_VERSION.to_string(),
        edge_type: edge.edge_type.clone(),
        relation_key: edge.relation_key.clone(),
        weight: edge.weight,
        evidence: edge.evidence.clone(),
    }
}

fn node_value(request: &TemporalGraphRequest, node: &TemporalGraphNode) -> Result<Vec<u8>> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": TEMPORAL_GRAPH_SCHEMA_VERSION,
        "domain": request.domain,
        "market_id": request.market_id,
        "market_cx_id": request.market_cx_id,
        "cx_id": node.cx_id,
        "node_kind": node.node_kind,
        "label": node.label,
    }))
    .map_err(|err| readback_error(err.to_string()))
}

fn relation_key(request: &TemporalGraphRequest, relation: &str) -> String {
    format!(
        "{}|{}|{}->{}|{}",
        request.domain, request.market_id, request.driver_name, request.response_name, relation
    )
}

fn periodicity_weight(report: &AutocorrelationReport) -> f32 {
    report
        .coefficients
        .iter()
        .copied()
        .filter(|value| value.is_finite() && *value > 0.0)
        .fold(0.0_f64, f64::max) as f32
}

fn hazard_weight(report: &InterEventHazardReport) -> f32 {
    if report.overdue {
        (1.0 - report.survival).min(1.0) as f32
    } else {
        report.hazard.min(1.0) as f32
    }
}

fn positive_weight(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        (value * 1_000_000.0).round() / 1_000_000.0
    } else {
        0.000001
    }
}

fn ensure_variance(name: &str, values: &[f32]) -> Result<()> {
    let mean = values.iter().map(|value| *value as f64).sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| (*value as f64 - mean).powi(2))
        .sum::<f64>()
        / values.len() as f64;
    if variance <= 0.0 || !variance.is_finite() {
        return Err(PolyError::diagnostics(
            ERR_TEMPORAL_GRAPH_LOW_SIGNAL,
            format!("{name} series has zero variance"),
        ));
    }
    Ok(())
}

fn map_assay_error(err: calyx_core::CalyxError) -> PolyError {
    match err.code {
        "CALYX_ASSAY_LOW_SIGNAL" | "CALYX_ASSAY_DEGENERATE_INPUT" => {
            PolyError::diagnostics(ERR_TEMPORAL_GRAPH_LOW_SIGNAL, err.message)
        }
        _ => PolyError::diagnostics(ERR_TEMPORAL_GRAPH_INSUFFICIENT, err.message),
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_TEMPORAL_GRAPH_INVALID_INPUT,
        message,
    ))
}

fn readback_error(message: impl Into<String>) -> PolyError {
    PolyError::diagnostics(ERR_TEMPORAL_GRAPH_READBACK_MISMATCH, message)
}

fn edge_id(edge: &TemporalGraphEdge) -> String {
    format!("{} -{}-> {}", edge.src, edge.edge_type, edge.dst)
}

#[allow(dead_code)]
fn _minimum_assay_floors() -> (usize, usize, usize, usize) {
    (
        MIN_PEARSON_SAMPLES,
        MIN_TE_QUORUM,
        MIN_PERIODICITY_SAMPLES,
        MIN_HAZARD_GAPS,
    )
}
