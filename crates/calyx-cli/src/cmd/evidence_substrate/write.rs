use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::{PhysicalPlainGraph, PlainGraph, PlainGraphCsr, PlainGraphCsrEdge};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, CalyxError, CxId, VaultStore};
use calyx_lodestar::{AsterAssocNodeProps, encode_assoc_node_props};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::model::{EvidenceGraphDraft, edge_value};
use super::source::SourceLoadReport;
use super::{DEFAULT_COLLECTION, MaterializeEvidenceSubstrateArgs};
use crate::cmd::vault::{resolve_vault_info, vault_salt};
use crate::error::{CliError, CliResult};

const GRAPH_ID_VERSION: u32 = 0;

#[derive(Debug, Serialize)]
pub(crate) struct MaterializeEvidenceSubstrateReport {
    pub status: &'static str,
    pub vault: String,
    pub vault_id: String,
    pub vault_dir: String,
    pub collection: String,
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
    let physical = PhysicalPlainGraph::open_latest(&resolved.path, &collection)?;
    for (id, expected) in &node_values {
        let actual = physical.get_node(*id)?.ok_or_else(|| {
            CliError::from(CalyxError {
                code: "CALYX_EVIDENCE_SUBSTRATE_NODE_READBACK_MISSING",
                message: format!("missing physical Graph CF node row {id}"),
                remediation: "do not trust the evidence substrate collection until the command rewrites and reads back every node",
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
    for (src, edge_type, dst, expected) in &edge_values {
        let actual = physical.get_edge(*src, edge_type, *dst)?.ok_or_else(|| {
            CliError::from(CalyxError {
                code: "CALYX_EVIDENCE_SUBSTRATE_EDGE_READBACK_MISSING",
                message: format!("missing physical Graph CF edge row {src} -{edge_type}-> {dst}"),
                remediation: "do not trust the evidence substrate collection until the command rewrites and reads back every edge",
            })
        })?;
        if &actual != expected {
            return Err(CliError::from(CalyxError {
                code: "CALYX_EVIDENCE_SUBSTRATE_EDGE_READBACK_MISMATCH",
                message: format!(
                    "physical Graph CF edge row {src} -{edge_type}-> {dst} differed after flush"
                ),
                remediation: "do not trust the evidence substrate collection until the edge value mismatch is fixed and rerun",
            }));
        }
    }
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
    let physical_edges = edge_values.len();
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
        vault: resolved.name,
        vault_id: resolved.vault_id.to_string(),
        vault_dir: resolved.path.display().to_string(),
        collection,
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
    Ok(report)
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
    let mut by_src = vec![Vec::<PlainGraphCsrEdge>::new(); nodes.len()];
    let mut association_edges = BTreeSet::new();
    for (src, edge_type, dst, _) in edge_values {
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
        by_src[src_index].push(PlainGraphCsrEdge {
            dst: *dst,
            edge_type: edge_type.clone(),
        });
        association_edges.insert((*src, *dst));
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
