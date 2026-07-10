//! Per-domain Loom + Graph CF build job (#73).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, CxId, SlotId};
use calyx_lodestar::{KernelParams, RecallQuery, RecallTestParams};
use calyx_loom::{CrossTermKind, CrossTermValue, MaterializationAction};
use calyx_mincut::{AgreementEdge as MincutAgreementEdge, FrequencyEntry};

use crate::domain::Domain;
pub use crate::domain_graph_build_job_types::*;
use crate::error::{PolyError, Result};
use crate::graph_weight::canonical_positive_weight;
use crate::kernel_recall_admission::{ComputedKernelRecallRequest, measure_computed_kernel_recall};
use crate::loom_shape_weave::run_shape_aware_loom_weave_for_cx_ids;
use crate::pair_gain_gate::{
    DEFAULT_PAIR_GAIN_K, compute_pair_gain_plan, read_pair_gain_plan, write_pair_gain_plan,
};
use crate::panel_diagnostics::PanelMatrix;

const LOOM_SLOT_NODE_PANEL_VERSION: u32 = 73;
const LOOM_SLOT_NODE_SALT: &[u8] = b"poly-domain-graph-build-loom-slot-v1";

pub struct DomainGraphBuildRequest<'a> {
    pub domain: Domain,
    pub collection: &'a str,
    pub panel_version: u32,
    pub source_cx_ids: &'a [CxId],
    pub supplied_edges: &'a [DomainGraphEdgeInput],
    pub pair_gain_matrix: &'a PanelMatrix,
    pub recall_corpus: &'a [RecallQuery],
    pub kernel_anchors: &'a [CxId],
    pub kernel_params: &'a KernelParams,
    pub recall_params: &'a RecallTestParams,
    pub output_dir: &'a Path,
    pub loom_cache_capacity: usize,
}

pub fn run_domain_graph_build_job<C: Clock>(
    vault: &AsterVault<C>,
    request: &DomainGraphBuildRequest<'_>,
    clock: &dyn Clock,
) -> Result<DomainGraphBuildRun> {
    validate_request(request)?;
    let pair_gain = compute_pair_gain_plan(
        request.domain.slug(),
        request.panel_version,
        request.pair_gain_matrix,
        clock,
        DEFAULT_PAIR_GAIN_K,
    )?;
    let pair_gain_path = write_pair_gain_plan(request.output_dir, &pair_gain)?;
    let pair_gain_readback = read_pair_gain_plan(&pair_gain_path)?;
    if pair_gain_readback != pair_gain {
        return Err(readback_error("pair-gain plan readback mismatch"));
    }

    let loom = run_shape_aware_loom_weave_for_cx_ids(
        vault,
        request.domain.slug(),
        request.panel_version,
        request.source_cx_ids,
        request.output_dir,
        request.loom_cache_capacity,
    )?;
    let loom_edges = loom_edges_from_xterms(&loom.report.xterm_rows)?;
    let mut all_edges = request.supplied_edges.to_vec();
    all_edges.extend(loom_edges);
    if all_edges.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_DOMAIN_GRAPH_EMPTY,
            "domain graph build produced no Graph CF edges",
        ));
    }

    let graph = PlainGraph::new(vault, request.collection)?;
    for node in graph_nodes(&all_edges) {
        graph.put_node(node, &node_value(request.domain, node)?)?;
    }
    for edge in &all_edges {
        graph.put_edge(edge.src, &edge.edge_type, edge.dst, &edge_bytes(edge)?)?;
    }
    let edge_snapshot = vault.latest_seq();
    let readback_edges = readback_graph_edges(&graph, edge_snapshot, &all_edges)?;
    let csr = graph.rebuild_csr(edge_snapshot)?;
    let graph_snapshot_seq = vault.latest_seq();
    let kernel_edges = kernel_edges_from_readback(&readback_edges);
    if kernel_edges.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_DOMAIN_GRAPH_EMPTY,
            "domain graph build had no readback edge marked include_in_kernel",
        ));
    }
    let frequencies = frequencies_from_edges(&kernel_edges);
    let computed_kernel_recall = measure_computed_kernel_recall(&ComputedKernelRecallRequest {
        domain: request.domain,
        corpus: request.recall_corpus,
        agreements: &kernel_edges,
        frequencies: &frequencies,
        anchors: request.kernel_anchors,
        kernel_params: request.kernel_params,
        recall_params: request.recall_params,
    })?;
    let report = DomainGraphBuildReport {
        schema_version: DOMAIN_GRAPH_BUILD_SCHEMA_VERSION.to_string(),
        domain: request.domain,
        collection: request.collection.to_string(),
        panel_version: request.panel_version,
        source_cx_ids: request.source_cx_ids.to_vec(),
        graph_node_count: graph_nodes(&all_edges).len(),
        graph_edge_count: all_edges.len(),
        loom_edge_count: all_edges
            .iter()
            .filter(|edge| edge.source == "shape_aware_loom_weave")
            .count(),
        supplied_edge_count: request.supplied_edges.len(),
        kernel_edge_count: kernel_edges.len(),
        disconnected_component_count: component_count(&all_edges),
        graph_cf_row_count: vault.scan_cf_at(graph_snapshot_seq, ColumnFamily::Graph)?.len(),
        csr_node_count: csr.projection.nodes.len(),
        csr_edge_count: csr.projection.edges.len(),
        xterm_count: loom.report.xterm_count,
        pair_gain: pair_gain_summary(pair_gain_path, pair_gain),
        computed_kernel_recall,
        readback_edges,
        source_of_truth:
            "shape-aware Loom XTerm CF readback + Graph CF byte readback + CSR rebuild + computed-kernel recall over Graph CF readback edges"
                .to_string(),
    };
    let report_path = write_domain_graph_build_report(request.output_dir, &report)?;
    Ok(DomainGraphBuildRun {
        report_path,
        report,
        graph_snapshot_seq,
    })
}

pub fn write_domain_graph_build_report(
    dir: &Path,
    report: &DomainGraphBuildReport,
) -> Result<std::path::PathBuf> {
    let file_name = format!("domain_graph_build_{}.json", report.domain.slug());
    crate::diagnostics_store::write_json(dir, &file_name, report)
}

pub fn read_domain_graph_build_report(path: &Path) -> Result<DomainGraphBuildReport> {
    crate::diagnostics_store::read_json(path)
}

pub fn loom_slot_node_id(cx_id: CxId, slot: SlotId) -> CxId {
    let input = format!("poly:domain_graph_build:loom_slot:{cx_id}:{}", slot.get());
    CxId::from_input(
        input.as_bytes(),
        LOOM_SLOT_NODE_PANEL_VERSION,
        LOOM_SLOT_NODE_SALT,
    )
}

fn validate_request(request: &DomainGraphBuildRequest<'_>) -> Result<()> {
    if request.collection.trim().is_empty() {
        return invalid("Graph CF collection is required");
    }
    if request.source_cx_ids.is_empty() && request.supplied_edges.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_DOMAIN_GRAPH_EMPTY,
            "domain graph build requires at least one source CxId or supplied edge",
        ));
    }
    for edge in request.supplied_edges {
        validate_edge(edge)?;
    }
    Ok(())
}

fn validate_edge(edge: &DomainGraphEdgeInput) -> Result<()> {
    if edge.edge_type.trim().is_empty()
        || edge.relation_key.trim().is_empty()
        || edge.source.trim().is_empty()
    {
        return invalid("edge_type, relation_key, and source are required");
    }
    positive_weight(edge.weight)?;
    Ok(())
}

fn loom_edges_from_xterms(
    rows: &[calyx_loom::agreement_graph::XtermRow],
) -> Result<Vec<DomainGraphEdgeInput>> {
    let mut edges = Vec::new();
    for row in rows {
        let CrossTermValue::Scalar(value) = row.value else {
            continue;
        };
        edges.push(DomainGraphEdgeInput {
            src: loom_slot_node_id(row.key.cx_id, row.key.a),
            dst: loom_slot_node_id(row.key.cx_id, row.key.b),
            edge_type: EDGE_LOOM_AGREEMENT.to_string(),
            relation_key: format!(
                "loom:{}:{}:{}:{:?}",
                row.key.cx_id,
                row.key.a.get(),
                row.key.b.get(),
                row.key.kind
            ),
            source: "shape_aware_loom_weave".to_string(),
            weight: positive_weight(value.abs())?,
            include_in_kernel: false,
        });
    }
    Ok(edges)
}

fn readback_graph_edges<C: Clock>(
    graph: &PlainGraph<'_, C>,
    snapshot: calyx_core::Seq,
    edges: &[DomainGraphEdgeInput],
) -> Result<Vec<DomainGraphReadbackEdge>> {
    let mut out = Vec::with_capacity(edges.len());
    for edge in edges {
        let bytes = graph
            .get_edge(snapshot, edge.src, &edge.edge_type, edge.dst)?
            .ok_or_else(|| readback_error(format!("missing Graph CF edge {}", edge_id(edge))))?;
        let expected = edge_bytes(edge)?;
        if bytes != expected {
            return Err(readback_error(format!(
                "Graph CF edge {} bytes mismatch expected_blake3={} actual_blake3={}",
                edge_id(edge),
                blake3::hash(&expected).to_hex(),
                blake3::hash(&bytes).to_hex()
            )));
        }
        let value: DomainGraphEdgeValue =
            serde_json::from_slice(&bytes).map_err(|err| readback_error(err.to_string()))?;
        out.push(DomainGraphReadbackEdge {
            src: edge.src,
            dst: edge.dst,
            edge_type: edge.edge_type.clone(),
            value,
            value_blake3: blake3::hash(&bytes).to_hex().to_string(),
        });
    }
    Ok(out)
}

fn kernel_edges_from_readback(edges: &[DomainGraphReadbackEdge]) -> Vec<MincutAgreementEdge> {
    edges
        .iter()
        .filter(|edge| edge.value.include_in_kernel)
        .map(|edge| MincutAgreementEdge {
            src: edge.src,
            dst: edge.dst,
            agreement: edge.value.weight.clamp(0.0, 1.0),
            directional_confidence: 1.0,
        })
        .collect()
}

fn frequencies_from_edges(edges: &[MincutAgreementEdge]) -> Vec<FrequencyEntry> {
    let mut nodes = BTreeSet::new();
    for edge in edges {
        nodes.insert(edge.src);
        nodes.insert(edge.dst);
    }
    nodes
        .into_iter()
        .map(|cx_id| FrequencyEntry {
            cx_id,
            frequency: 1.0,
        })
        .collect()
}

fn graph_nodes(edges: &[DomainGraphEdgeInput]) -> BTreeSet<CxId> {
    let mut nodes = BTreeSet::new();
    for edge in edges {
        nodes.insert(edge.src);
        nodes.insert(edge.dst);
    }
    nodes
}

fn component_count(edges: &[DomainGraphEdgeInput]) -> usize {
    let mut adjacency = BTreeMap::<CxId, BTreeSet<CxId>>::new();
    for edge in edges {
        adjacency.entry(edge.src).or_default().insert(edge.dst);
        adjacency.entry(edge.dst).or_default().insert(edge.src);
    }
    let mut seen = BTreeSet::new();
    let mut count = 0usize;
    for start in adjacency.keys().copied().collect::<Vec<_>>() {
        if !seen.insert(start) {
            continue;
        }
        count += 1;
        let mut stack = vec![start];
        while let Some(node) = stack.pop() {
            if let Some(next) = adjacency.get(&node) {
                for neighbor in next {
                    if seen.insert(*neighbor) {
                        stack.push(*neighbor);
                    }
                }
            }
        }
    }
    count
}

fn pair_gain_summary(
    path: std::path::PathBuf,
    record: crate::pair_gain_gate::PairGainMaterializationRecord,
) -> DomainGraphPairGainSummary {
    let interaction_lazy_count = record
        .plan
        .entries
        .iter()
        .filter(|entry| {
            entry.kind == CrossTermKind::Interaction
                && entry.action != MaterializationAction::EagerStore
        })
        .count();
    let provisional_count = record
        .measurements
        .iter()
        .filter(|measurement| measurement.provisional)
        .count();
    DomainGraphPairGainSummary {
        path,
        interaction_eager_count: record.interaction_eager_count,
        interaction_lazy_count,
        provisional_count,
        record,
    }
}

fn edge_bytes(edge: &DomainGraphEdgeInput) -> Result<Vec<u8>> {
    serde_json::to_vec(&edge_value(edge)?).map_err(|err| readback_error(err.to_string()))
}

fn edge_value(edge: &DomainGraphEdgeInput) -> Result<DomainGraphEdgeValue> {
    Ok(DomainGraphEdgeValue {
        schema_version: DOMAIN_GRAPH_BUILD_SCHEMA_VERSION.to_string(),
        edge_type: edge.edge_type.clone(),
        relation_key: edge.relation_key.clone(),
        source: edge.source.clone(),
        weight: positive_weight(edge.weight)?,
        include_in_kernel: edge.include_in_kernel,
    })
}

fn node_value(domain: Domain, node: CxId) -> Result<Vec<u8>> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": DOMAIN_GRAPH_BUILD_SCHEMA_VERSION,
        "domain": domain.slug(),
        "cx_id": node,
    }))
    .map_err(|err| readback_error(err.to_string()))
}

fn positive_weight(value: f32) -> Result<f32> {
    canonical_positive_weight(value).ok_or_else(|| {
        PolyError::diagnostics(
            ERR_DOMAIN_GRAPH_INVALID_INPUT,
            format!("graph edge weight {value} must remain positive after canonicalization"),
        )
    })
}

fn edge_id(edge: &DomainGraphEdgeInput) -> String {
    format!("{} -{}-> {}", edge.src, edge.edge_type, edge.dst)
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_DOMAIN_GRAPH_INVALID_INPUT,
        message,
    ))
}

fn readback_error(message: impl Into<String>) -> PolyError {
    PolyError::diagnostics(ERR_DOMAIN_GRAPH_READBACK_MISMATCH, message)
}
