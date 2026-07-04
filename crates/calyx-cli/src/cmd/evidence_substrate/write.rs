use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::{
    GraphCollectionGenerationState, GraphCollectionGenerationStatus, GraphCollectionLifecycle,
    PhysicalGraphCollectionLifecycle, PhysicalPlainGraph, PlainGraph, PlainGraphCsr,
    PlainGraphCsrEdge, plain_graph_edge_raw_weight, plain_graph_normalized_edge_weight,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, CalyxError, CxId, VaultStore};
use calyx_lodestar::{AsterAssocNodeProps, encode_assoc_node_props};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::model::{EvidenceGraphDraft, edge_value};
use super::source::SourceLoadReport;
use super::{DEFAULT_COLLECTION, MaterializeEvidenceSubstrateArgs};
use crate::cmd::vault::{ResolvedVault, resolve_vault_info, vault_salt};
use crate::error::{CliError, CliResult};

const GRAPH_ID_VERSION: u32 = 0;

#[derive(Debug, Serialize)]
pub(crate) struct MaterializeEvidenceSubstrateReport {
    pub status: &'static str,
    pub vault: String,
    pub vault_id: String,
    pub vault_dir: String,
    pub collection: String,
    pub graph_generation: String,
    pub panel_version: u32,
    pub source_report: SourceLoadReport,
    pub graph_summary: serde_json::Value,
    pub readback: EvidenceSubstrateReadback,
}

#[derive(Debug, Serialize)]
pub(crate) struct EvidenceSubstrateReadback {
    pub source_of_truth: &'static str,
    pub node_rows_written: usize,
    pub edge_rows_written: usize,
    pub metadata_rows_written: usize,
    pub physical_node_keys: usize,
    pub physical_edge_out_keys: usize,
    pub csr_nodes: usize,
    pub csr_edges: usize,
    pub association_edge_count: usize,
    pub assoc_graph_nodes: usize,
    pub assoc_graph_edges: usize,
    pub csr_bytes: usize,
    pub csr_sha256: String,
    pub csr_blake3: String,
    pub source_snapshot: u64,
    pub all_node_values_read_back: bool,
    pub all_edge_values_read_back: bool,
}

pub(crate) fn write_to_calyx(
    home: &Path,
    args: &MaterializeEvidenceSubstrateArgs,
    draft: EvidenceGraphDraft,
    source_report: SourceLoadReport,
) -> CliResult<MaterializeEvidenceSubstrateReport> {
    if draft.nodes.is_empty() || draft.edges.is_empty() {
        return Err(CliError::runtime(
            "evidence substrate draft is empty; refusing to write an empty Calyx graph",
        ));
    }
    let collection = args
        .collection
        .clone()
        .unwrap_or_else(|| DEFAULT_COLLECTION.to_string());
    let resolved = resolve_vault_info(home, &args.vault)?;
    let vault = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions {
            restore_mvcc_rows: false,
            ..VaultOptions::default()
        },
    )?;
    let graph = PlainGraph::new(&vault, &collection)?;
    let generation = format!("materialize-evidence-substrate-{}", ulid::Ulid::new());
    let lifecycle = GraphCollectionLifecycle::new(&vault)?;
    lifecycle.put_state(
        &GraphCollectionGenerationState::new(
            collection.clone(),
            generation.clone(),
            GraphCollectionGenerationStatus::Writing,
            "materialize-evidence-substrate",
        )
        .with_reason("graph materialization started")
        .with_detail("schema", "evidence_substrate_v1"),
    )?;
    let salt = resolved.vault_id.to_string();
    let mut node_ids = BTreeMap::new();
    let mut node_values = BTreeMap::new();
    let mut graph_batch = Vec::with_capacity(draft.nodes.len() + (draft.edges.len() * 2));
    for node in draft.nodes.values() {
        let id = CxId::from_input(
            node.stable_key.as_bytes(),
            GRAPH_ID_VERSION,
            salt.as_bytes(),
        );
        let props = node_props(node);
        let value = encode_assoc_node_props(&props)?;
        graph_batch.push((ColumnFamily::Graph, graph.node_key(id), value.clone()));
        node_ids.insert(node.stable_key.clone(), id);
        node_values.insert(id, value);
    }
    let mut edge_values = Vec::with_capacity(draft.edges.len());
    for edge in draft.edges.values() {
        let src = *node_ids.get(&edge.src_key).ok_or_else(|| {
            CliError::runtime(format!(
                "edge source {} has no materialized node id",
                edge.src_key
            ))
        })?;
        let dst = *node_ids.get(&edge.dst_key).ok_or_else(|| {
            CliError::runtime(format!(
                "edge destination {} has no materialized node id",
                edge.dst_key
            ))
        })?;
        let value = serde_json::to_vec(&edge_value(edge)).map_err(|error| {
            CliError::runtime(format!(
                "serialize edge value {} -> {}: {error}",
                edge.src_key, edge.dst_key
            ))
        })?;
        let edge_key = graph.edge_out_key(src, &edge.edge_type, dst)?;
        let reverse_key = graph.edge_in_key(dst, &edge.edge_type, src)?;
        graph_batch.push((ColumnFamily::Graph, edge_key.clone(), value.clone()));
        graph_batch.push((ColumnFamily::Graph, reverse_key, edge_key));
        edge_values.push((src, edge.edge_type.clone(), dst, value));
    }
    vault.write_cf_batch(graph_batch)?;
    let metadata_value = serde_json::to_vec(&json!({
        "collection": collection,
        "schema": "evidence_substrate_v1",
        "summary": draft.association_summary(),
    }))
    .map_err(|error| CliError::runtime(format!("serialize graph metadata: {error}")))?;
    graph.put_metadata("evidence_substrate_summary", &metadata_value)?;
    let projection = build_csr_projection(&collection, vault.snapshot(), &node_ids, &edge_values)?;
    let commit = graph.write_csr_projection(projection)?;
    vault.flush()?;
    drop(graph);
    drop(vault);
    let physical = PhysicalPlainGraph::open_latest_unchecked(&resolved.path, &collection)?;
    read_back_nodes(&physical, &node_values)?;
    let physical_edge_out_keys = read_back_edges(&physical, &edge_values)?;
    let raw = physical.read_csr_bytes()?.ok_or_else(|| {
        CliError::from(CalyxError {
            code: "CALYX_EVIDENCE_SUBSTRATE_CSR_READBACK_MISSING",
            message: format!(
                "persisted CSR row is missing for evidence substrate collection {collection}"
            ),
            remediation: "rerun materialize-evidence-substrate and inspect Graph CF flush state",
        })
    })?;
    let csr = physical.read_csr()?.ok_or_else(|| {
        CliError::from(CalyxError {
            code: "CALYX_EVIDENCE_SUBSTRATE_CSR_DECODE_MISSING",
            message: format!("persisted CSR row did not decode for collection {collection}"),
            remediation: "rerun materialize-evidence-substrate and inspect CSR segment rows",
        })
    })?;
    let assoc = physical.assoc_graph()?;
    let physical_nodes = node_values.len();
    let physical_edges = physical_edge_out_keys;
    if physical_nodes != draft.nodes.len()
        || physical_edges != draft.edges.len()
        || csr.nodes.len() != draft.nodes.len()
        || csr.edges.len() != draft.edges.len()
        || assoc.node_count() != draft.nodes.len()
        || assoc.edge_count() != commit.projection.association_edge_count
    {
        return Err(CliError::from(CalyxError {
            code: "CALYX_EVIDENCE_SUBSTRATE_GRAPH_READBACK_MISMATCH",
            message: format!(
                "Graph CF readback mismatch for collection={collection}: expected nodes={} edges={}, physical nodes={physical_nodes} edges={physical_edges}, csr nodes={} edges={}, assoc nodes={} edges={}",
                draft.nodes.len(),
                draft.edges.len(),
                csr.nodes.len(),
                csr.edges.len(),
                assoc.node_count(),
                assoc.edge_count()
            ),
            remediation: "do not run downstream association mining on this collection until the Graph CF and CSR counts match",
        }));
    }
    let graph_summary = draft.association_summary();
    let report = MaterializeEvidenceSubstrateReport {
        status: "ok",
        vault: resolved.name.clone(),
        vault_id: resolved.vault_id.to_string(),
        vault_dir: resolved.path.display().to_string(),
        collection,
        graph_generation: generation.clone(),
        panel_version: GRAPH_ID_VERSION,
        source_report,
        graph_summary,
        readback: EvidenceSubstrateReadback {
            source_of_truth: "physical Aster Graph CF via PhysicalPlainGraph node/edge/CSR readback",
            node_rows_written: draft.nodes.len(),
            edge_rows_written: draft.edges.len(),
            metadata_rows_written: 1,
            physical_node_keys: physical_nodes,
            physical_edge_out_keys: physical_edges,
            csr_nodes: csr.nodes.len(),
            csr_edges: csr.edges.len(),
            association_edge_count: csr.association_edge_count,
            assoc_graph_nodes: assoc.node_count(),
            assoc_graph_edges: assoc.edge_count(),
            csr_bytes: raw.len(),
            csr_sha256: sha256_hex(&raw),
            csr_blake3: blake3::hash(&raw).to_hex().to_string(),
            source_snapshot: csr.source_snapshot,
            all_node_values_read_back: true,
            all_edge_values_read_back: true,
        },
    };
    if let Some(path) = &args.report {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(&report)
            .map_err(|error| CliError::runtime(format!("serialize report: {error}")))?;
        fs::write(path, &bytes)?;
        let readback = fs::read(path)?;
        if readback != bytes {
            return Err(CliError::runtime(format!(
                "report readback mismatch at {}",
                path.display()
            )));
        }
    }
    accept_generation(
        &resolved,
        &report.collection,
        &generation,
        &report,
        args.report.as_deref(),
    )?;
    Ok(report)
}

fn accept_generation(
    resolved: &ResolvedVault,
    collection: &str,
    generation: &str,
    report: &MaterializeEvidenceSubstrateReport,
    report_path: Option<&Path>,
) -> CliResult {
    let vault = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions {
            restore_mvcc_rows: false,
            ..VaultOptions::default()
        },
    )?;
    let lifecycle = GraphCollectionLifecycle::new(&vault)?;
    let mut state = GraphCollectionGenerationState::new(
        collection.to_string(),
        generation.to_string(),
        GraphCollectionGenerationStatus::Accepted,
        "materialize-evidence-substrate",
    )
    .with_reason("physical graph, CSR, and report readback passed")
    .with_detail("schema", "evidence_substrate_v1")
    .with_detail("node_rows", report.readback.node_rows_written.to_string())
    .with_detail("edge_rows", report.readback.edge_rows_written.to_string())
    .with_detail("csr_sha256", report.readback.csr_sha256.clone());
    if let Some(path) = report_path {
        state = state.with_detail("report", path.display().to_string());
    }
    lifecycle.put_state(&state)?;
    vault.flush()?;
    drop(vault);
    let lifecycle = PhysicalGraphCollectionLifecycle::open_latest(&resolved.path)?;
    let accepted = lifecycle.list_states()?.into_iter().any(|row| {
        row.state.collection == collection
            && row.state.generation == generation
            && row.state.status == GraphCollectionGenerationStatus::Accepted
    });
    if !accepted {
        return Err(CliError::runtime(format!(
            "accepted graph collection lifecycle row missing after readback: {collection}/{generation}"
        )));
    }
    Ok(())
}

fn read_back_nodes(
    physical: &PhysicalPlainGraph,
    node_values: &BTreeMap<CxId, Vec<u8>>,
) -> CliResult {
    let physical_nodes = physical.node_props()?;
    if physical_nodes.len() != node_values.len() {
        return Err(CliError::from(CalyxError {
            code: "CALYX_EVIDENCE_SUBSTRATE_NODE_KEY_READBACK_MISMATCH",
            message: format!(
                "physical Graph CF node range count mismatch: expected {} read {}",
                node_values.len(),
                physical_nodes.len()
            ),
            remediation: "do not trust the evidence substrate collection until the physical node range count matches the written node set",
        }));
    }
    for (id, actual) in physical_nodes {
        let expected = node_values.get(&id).ok_or_else(|| {
            CliError::from(CalyxError {
                code: "CALYX_EVIDENCE_SUBSTRATE_NODE_READBACK_EXTRA",
                message: format!("physical Graph CF node row {id} was not in the written node set"),
                remediation: "do not trust the evidence substrate collection until every node reads back",
            })
        })?;
        if &actual != expected {
            return Err(CliError::from(CalyxError {
                code: "CALYX_EVIDENCE_SUBSTRATE_NODE_READBACK_MISMATCH",
                message: format!("physical Graph CF node row {id} differed after flush"),
                remediation: "do not trust the evidence substrate collection until the node value mismatch is fixed and rerun",
            }));
        }
    }
    Ok(())
}

fn read_back_edges(
    physical: &PhysicalPlainGraph,
    edge_values: &[(CxId, String, CxId, Vec<u8>)],
) -> CliResult<usize> {
    let physical_edges = physical.edge_out_props()?;
    if physical_edges.len() != edge_values.len() {
        return Err(CliError::from(CalyxError {
            code: "CALYX_EVIDENCE_SUBSTRATE_EDGE_KEY_READBACK_MISMATCH",
            message: format!(
                "physical Graph CF edge range count mismatch: expected {} read {}",
                edge_values.len(),
                physical_edges.len()
            ),
            remediation: "do not trust the evidence substrate collection until the physical edge range count matches the written edge set",
        }));
    }
    let expected = edge_values
        .iter()
        .map(|(src, edge_type, dst, value)| ((*src, edge_type.clone(), *dst), value))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    for edge in physical_edges {
        let key = (edge.src, edge.edge_type, edge.dst);
        let expected_value = expected.get(&key).ok_or_else(|| {
            CliError::from(CalyxError {
                code: "CALYX_EVIDENCE_SUBSTRATE_EDGE_READBACK_EXTRA",
                message: format!(
                    "physical Graph CF edge row {} -{}-> {} was not in the written edge set",
                    key.0, key.1, key.2
                ),
                remediation: "do not trust the evidence substrate collection until every edge reads back exactly",
            })
        })?;
        if edge.value != **expected_value {
            return Err(CliError::from(CalyxError {
                code: "CALYX_EVIDENCE_SUBSTRATE_EDGE_READBACK_MISMATCH",
                message: format!(
                    "physical Graph CF edge row {} -{}-> {} differed after flush",
                    key.0, key.1, key.2
                ),
                remediation: "do not trust the evidence substrate collection until the edge value mismatch is fixed and rerun",
            }));
        }
        seen.insert(key);
    }
    if let Some((src, edge_type, dst)) = expected.keys().find(|key| !seen.contains(*key)) {
        return Err(CliError::from(CalyxError {
            code: "CALYX_EVIDENCE_SUBSTRATE_EDGE_READBACK_MISSING",
            message: format!("missing physical Graph CF edge row {src} -{edge_type}-> {dst}"),
            remediation: "do not trust the evidence substrate collection until every edge reads back",
        }));
    }
    Ok(seen.len())
}

fn node_props(node: &super::model::EvidenceNode) -> AsterAssocNodeProps {
    let mut metadata = node.metadata.clone();
    metadata.insert("stable_key".to_string(), node.stable_key.clone());
    metadata.insert("node_type".to_string(), node.node_type.clone());
    metadata.insert("label".to_string(), node.label.clone());
    metadata.insert("schema".to_string(), "evidence_substrate_v1".to_string());
    AsterAssocNodeProps {
        anchors: vec![
            AnchorKind::Label("evidence_substrate".to_string()),
            AnchorKind::Label(format!("evidence_substrate:{}", node.node_type)),
        ],
        metadata,
        ..Default::default()
    }
}

fn build_csr_projection(
    collection: &str,
    snapshot: u64,
    node_ids: &BTreeMap<String, CxId>,
    edge_values: &[(CxId, String, CxId, Vec<u8>)],
) -> CliResult<PlainGraphCsr> {
    let mut nodes = node_ids.values().copied().collect::<Vec<_>>();
    nodes.sort();
    let node_index = nodes
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect::<BTreeMap<_, _>>();
    let mut drafts = Vec::new();
    let mut max_raw_weight = 0.0_f32;
    let mut association_edges = BTreeSet::new();
    for (src, edge_type, dst, value) in edge_values {
        let Some(src_index) = node_index.get(src).copied() else {
            return Err(CliError::runtime(format!(
                "CSR source {src} has no node row"
            )));
        };
        if !node_index.contains_key(dst) {
            return Err(CliError::runtime(format!(
                "CSR destination {dst} has no node row"
            )));
        }
        let raw_weight = plain_graph_edge_raw_weight(value)?;
        max_raw_weight = max_raw_weight.max(raw_weight);
        drafts.push((src_index, *dst, edge_type.clone(), raw_weight));
        association_edges.insert((*src, *dst));
    }
    let mut by_src = vec![Vec::<PlainGraphCsrEdge>::new(); nodes.len()];
    for (src_index, dst, edge_type, raw_weight) in drafts {
        by_src[src_index].push(PlainGraphCsrEdge {
            dst,
            edge_type,
            weight: plain_graph_normalized_edge_weight(raw_weight, max_raw_weight)?,
        });
    }
    let mut offsets = Vec::with_capacity(nodes.len() + 1);
    let mut edges = Vec::with_capacity(edge_values.len());
    offsets.push(0);
    for mut list in by_src {
        list.sort_by(|left, right| {
            left.dst
                .cmp(&right.dst)
                .then(left.edge_type.cmp(&right.edge_type))
        });
        edges.extend(list);
        offsets.push(edges.len());
    }
    Ok(PlainGraphCsr {
        collection: collection.to_string(),
        source_snapshot: snapshot,
        nodes,
        offsets,
        edges,
        association_edge_count: association_edges.len(),
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
