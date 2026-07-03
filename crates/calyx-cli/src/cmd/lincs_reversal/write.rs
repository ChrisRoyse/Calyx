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

use super::model::{LincsGraphDraft, edge_value};
use super::source::LincsSourceReport;
use super::{DEFAULT_COLLECTION, MaterializeLincsReversalArgs};
use crate::cmd::vault::{resolve_vault_info, vault_salt};
use crate::error::{CliError, CliResult};

const GRAPH_ID_VERSION: u32 = 0;

#[derive(Debug, Serialize)]
pub(crate) struct MaterializeLincsReversalReport {
    pub status: &'static str,
    pub vault: String,
    pub vault_id: String,
    pub vault_dir: String,
    pub collection: String,
    pub panel_version: u32,
    pub source_report: LincsSourceReport,
    pub graph_summary: serde_json::Value,
    pub readback: LincsReversalReadback,
}

#[derive(Debug, Serialize)]
pub(crate) struct LincsReversalReadback {
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
    pub edge_value_readback_mode: &'static str,
    pub sampled_edge_values_read_back: usize,
    pub all_edge_values_read_back: bool,
}

pub(crate) fn write_to_calyx(
    home: &Path,
    args: &MaterializeLincsReversalArgs,
    draft: LincsGraphDraft,
    source_report: LincsSourceReport,
) -> CliResult<MaterializeLincsReversalReport> {
    if draft.nodes.is_empty() || draft.edges.is_empty() {
        return Err(CliError::runtime(
            "LINCS reversal draft is empty; refusing to write an empty Calyx graph",
        ));
    }
    let collection = args
        .collection
        .clone()
        .unwrap_or_else(|| DEFAULT_COLLECTION.to_string());
    eprintln!("lincs-reversal: opening vault for collection {collection}");
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
    eprintln!("lincs-reversal: encoding {} nodes", draft.nodes.len());
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
    eprintln!("lincs-reversal: encoding {} edges", draft.edges.len());
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
    eprintln!(
        "lincs-reversal: writing {} Graph CF batch rows",
        graph_batch.len()
    );
    vault.write_cf_batch(graph_batch)?;
    eprintln!("lincs-reversal: writing metadata and CSR projection");
    let metadata_value = serde_json::to_vec(&json!({
        "collection": collection,
        "schema": "lincs_reversal_v1",
        "boundary": "transcriptomic reversal is a lead signal only",
        "summary": draft.association_summary(),
    }))
    .map_err(|error| CliError::runtime(format!("serialize graph metadata: {error}")))?;
    graph.put_metadata("lincs_reversal_summary", &metadata_value)?;
    let projection = build_csr_projection(&collection, vault.snapshot(), &node_ids, &edge_values)?;
    let commit = graph.write_csr_projection(projection)?;
    eprintln!("lincs-reversal: flushing vault");
    vault.flush()?;
    drop(graph);
    drop(vault);
    eprintln!("lincs-reversal: reopening physical graph for readback");
    let physical = PhysicalPlainGraph::open_latest(&resolved.path, &collection)?;
    eprintln!("lincs-reversal: reading back {} nodes", node_values.len());
    read_back_nodes(&physical, &node_values)?;
    eprintln!("lincs-reversal: counting physical edge keys");
    let physical_edge_out_keys = physical.edge_out_key_count()?;
    if physical_edge_out_keys != edge_values.len() {
        return Err(CliError::from(CalyxError {
            code: "CALYX_LINCS_REVERSAL_EDGE_KEY_READBACK_MISMATCH",
            message: format!(
                "Graph CF edge key count mismatch for collection={collection}: expected {} read {physical_edge_out_keys}",
                edge_values.len()
            ),
            remediation: "do not trust the LINCS reversal collection until the physical edge key range count matches the written edge set",
        }));
    }
    let edge_sample = deterministic_edge_sample(&edge_values, 512);
    eprintln!(
        "lincs-reversal: reading back {} sampled edge values",
        edge_sample.len()
    );
    read_back_edges(&physical, &edge_sample)?;
    eprintln!("lincs-reversal: reading back CSR");
    let raw = physical.read_csr_bytes()?.ok_or_else(|| {
        CliError::from(CalyxError {
            code: "CALYX_LINCS_REVERSAL_CSR_READBACK_MISSING",
            message: format!("persisted CSR row is missing for LINCS collection {collection}"),
            remediation: "rerun materialize-lincs-reversal and inspect Graph CF flush state",
        })
    })?;
    let csr = physical.read_csr()?.ok_or_else(|| {
        CliError::from(CalyxError {
            code: "CALYX_LINCS_REVERSAL_CSR_DECODE_MISSING",
            message: format!("persisted CSR row did not decode for LINCS collection {collection}"),
            remediation: "rerun materialize-lincs-reversal and inspect CSR segment rows",
        })
    })?;
    eprintln!("lincs-reversal: building assoc graph from persisted CSR");
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
            code: "CALYX_LINCS_REVERSAL_GRAPH_READBACK_MISMATCH",
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
    let report = MaterializeLincsReversalReport {
        status: "ok",
        vault: resolved.name,
        vault_id: resolved.vault_id.to_string(),
        vault_dir: resolved.path.display().to_string(),
        collection,
        panel_version: GRAPH_ID_VERSION,
        source_report,
        graph_summary,
        readback: LincsReversalReadback {
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
            edge_value_readback_mode: "all physical edge keys counted by collection range; deterministic edge values sampled",
            sampled_edge_values_read_back: edge_sample.len(),
            all_edge_values_read_back: false,
        },
    };
    if let Some(path) = &args.report {
        write_report(path, &report)?;
    }
    Ok(report)
}

fn read_back_nodes(
    physical: &PhysicalPlainGraph,
    node_values: &BTreeMap<CxId, Vec<u8>>,
) -> CliResult {
    for (id, expected) in node_values {
        let actual = physical.get_node(*id)?.ok_or_else(|| {
            CliError::from(CalyxError {
                code: "CALYX_LINCS_REVERSAL_NODE_READBACK_MISSING",
                message: format!("missing physical Graph CF node row {id}"),
                remediation: "do not trust the LINCS reversal collection until every node reads back",
            })
        })?;
        if &actual != expected {
            return Err(CliError::from(CalyxError {
                code: "CALYX_LINCS_REVERSAL_NODE_READBACK_MISMATCH",
                message: format!("physical Graph CF node row {id} differed after flush"),
                remediation: "do not trust the LINCS reversal collection until the mismatch is fixed and rerun",
            }));
        }
    }
    Ok(())
}

fn read_back_edges(
    physical: &PhysicalPlainGraph,
    edge_values: &[&(CxId, String, CxId, Vec<u8>)],
) -> CliResult {
    for (src, edge_type, dst, expected) in edge_values.iter().copied() {
        let actual = physical.get_edge(*src, edge_type, *dst)?.ok_or_else(|| {
            CliError::from(CalyxError {
                code: "CALYX_LINCS_REVERSAL_EDGE_READBACK_MISSING",
                message: format!("missing physical Graph CF edge row {src} -{edge_type}-> {dst}"),
                remediation: "do not trust the LINCS reversal collection until every edge reads back",
            })
        })?;
        if &actual != expected {
            return Err(CliError::from(CalyxError {
                code: "CALYX_LINCS_REVERSAL_EDGE_READBACK_MISMATCH",
                message: format!(
                    "physical Graph CF edge row {src} -{edge_type}-> {dst} differed after flush"
                ),
                remediation: "do not trust the LINCS reversal collection until the mismatch is fixed and rerun",
            }));
        }
    }
    Ok(())
}

fn deterministic_edge_sample<T>(items: &[T], max_items: usize) -> Vec<&T> {
    if items.len() <= max_items || max_items == 0 {
        return items.iter().collect();
    }
    if max_items == 1 {
        return vec![&items[0]];
    }
    let last = items.len() - 1;
    let denom = max_items - 1;
    let mut out = Vec::with_capacity(max_items);
    let mut previous = None;
    for index in 0..max_items {
        let position = (index * last) / denom;
        if previous != Some(position) {
            out.push(&items[position]);
            previous = Some(position);
        }
    }
    out
}

fn write_report(path: &Path, report: &MaterializeLincsReversalReport) -> CliResult {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(report)
        .map_err(|error| CliError::runtime(format!("serialize report: {error}")))?;
    fs::write(path, &bytes)?;
    let readback = fs::read(path)?;
    if readback != bytes {
        return Err(CliError::runtime(format!(
            "report readback mismatch at {}",
            path.display()
        )));
    }
    Ok(())
}

fn node_props(node: &super::model::LincsNode) -> AsterAssocNodeProps {
    let mut metadata = node.metadata.clone();
    metadata.insert("stable_key".to_string(), node.stable_key.clone());
    metadata.insert("node_type".to_string(), node.node_type.clone());
    metadata.insert("label".to_string(), node.label.clone());
    metadata.insert("schema".to_string(), "lincs_reversal_v1".to_string());
    AsterAssocNodeProps {
        anchors: vec![
            AnchorKind::Label("lincs_reversal".to_string()),
            AnchorKind::Label(format!("lincs_reversal:{}", node.node_type)),
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
