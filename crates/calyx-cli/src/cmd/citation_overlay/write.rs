use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::{
    GRAPH_GENERATION_STORAGE_COLLECTION_DETAIL, GRAPH_GENERATION_STORAGE_SCHEMA,
    GRAPH_GENERATION_STORAGE_SCHEMA_DETAIL, GraphCollectionGenerationState,
    GraphCollectionGenerationStatus, GraphCollectionLifecycle, PhysicalGraphCollectionLifecycle,
    PhysicalPlainGraph, PlainGraph, PlainGraphCsr, PlainGraphCsrEdge,
    graph_generation_storage_collection, plain_graph_edge_raw_weight,
    plain_graph_normalized_edge_weight,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, CalyxError, CxId, VaultStore};
use calyx_lodestar::{AsterAssocNodeProps, encode_assoc_node_props};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::{
    CitationOverlayDraft, CitesEdge, DEFAULT_COLLECTION, EDGE_TYPE, FRONTIER_EDGE_TYPE,
    FRONTIER_NODE_TYPE, FRONTIER_SCHEMA, MaterializeCitationOverlayArgs, OpinionNode,
    PROVENANCE_DATASET, SCHEMA, SkipReport,
};
use crate::cmd::vault::{ResolvedVault, resolve_vault_info, vault_salt};
use crate::durable_write::write_bytes_atomic_new;
use crate::error::{CliError, CliResult};

const GRAPH_WAL_ROWS_PER_BATCH: usize = 10_000;

#[derive(Debug, Serialize)]
pub(crate) struct CitationOverlayReport {
    pub status: &'static str,
    pub vault: String,
    pub vault_id: String,
    pub vault_dir: String,
    pub collection: String,
    pub graph_generation: String,
    pub graph_storage_collection: String,
    pub provenance_dataset: &'static str,
    pub edge_type: &'static str,
    pub skip_report: SkipReport,
    pub readback: CitationOverlayReadback,
}

#[derive(Debug, Serialize)]
pub(crate) struct CitationOverlayReadback {
    pub source_of_truth: &'static str,
    pub node_rows_written: usize,
    pub edge_rows_written: usize,
    pub physical_node_keys: usize,
    pub physical_edge_out_keys: usize,
    pub csr_nodes: usize,
    pub csr_edges: usize,
    pub assoc_graph_nodes: usize,
    pub assoc_graph_edges: usize,
    pub csr_bytes: usize,
    pub csr_sha256: String,
    pub all_node_values_read_back: bool,
    pub all_edge_values_read_back: bool,
    pub graph_wal_batches: usize,
    pub graph_wal_rows_per_batch_cap: usize,
}

pub(crate) fn write_to_calyx(
    home: &Path,
    args: &MaterializeCitationOverlayArgs,
    draft: CitationOverlayDraft,
) -> CliResult<CitationOverlayReport> {
    let collection = args
        .collection
        .clone()
        .unwrap_or_else(|| DEFAULT_COLLECTION.to_string());
    let schema = if args.frontier.is_some() {
        FRONTIER_SCHEMA
    } else {
        SCHEMA
    };
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
    let generation = format!("materialize-citation-overlay-{}", ulid::Ulid::new());
    let storage_collection = graph_generation_storage_collection(&collection, &generation)?;
    let lifecycle = GraphCollectionLifecycle::new(&vault)?;
    lifecycle.put_state(
        &GraphCollectionGenerationState::new(
            collection.clone(),
            generation.clone(),
            GraphCollectionGenerationStatus::Writing,
            "materialize-citation-overlay",
        )
        .with_reason("citation overlay materialization started")
        .with_detail("schema", schema)
        .with_detail(
            GRAPH_GENERATION_STORAGE_SCHEMA_DETAIL,
            GRAPH_GENERATION_STORAGE_SCHEMA,
        )
        .with_detail(
            GRAPH_GENERATION_STORAGE_COLLECTION_DETAIL,
            storage_collection.clone(),
        ),
    )?;
    vault.flush()?;
    let graph = PlainGraph::new_generation(&vault, &collection, &generation)?;

    let mut node_values: BTreeMap<CxId, Vec<u8>> = BTreeMap::new();
    let mut graph_batch = Vec::with_capacity(draft.nodes.len() + (draft.edges.len() * 2));
    for node in draft.nodes.values() {
        let value = encode_assoc_node_props(&node_props(node))?;
        graph_batch.push((
            ColumnFamily::Graph,
            graph.node_key(node.cx_id),
            value.clone(),
        ));
        node_values.insert(node.cx_id, value);
    }
    let mut edge_values = Vec::with_capacity(draft.edges.len());
    for edge in &draft.edges {
        let value = serde_json::to_vec(&edge_value(edge))
            .map_err(|error| CliError::runtime(format!("serialize cites edge value: {error}")))?;
        let edge_key = graph.edge_out_key(edge.src, edge.edge_type, edge.dst)?;
        let reverse_key = graph.edge_in_key(edge.dst, edge.edge_type, edge.src)?;
        graph_batch.push((ColumnFamily::Graph, edge_key.clone(), value.clone()));
        graph_batch.push((ColumnFamily::Graph, reverse_key, edge_key));
        edge_values.push((edge.src, edge.edge_type.to_string(), edge.dst, value));
    }
    let graph_wal_batches = graph_batch.len().div_ceil(GRAPH_WAL_ROWS_PER_BATCH);
    for rows in graph_batch.chunks(GRAPH_WAL_ROWS_PER_BATCH) {
        vault.write_cf_batch(rows.to_vec())?;
    }
    let metadata_value = serde_json::to_vec(&json!({
        "collection": collection,
        "schema": schema,
        "edge_types": if args.frontier.is_some() {
            vec![EDGE_TYPE, FRONTIER_EDGE_TYPE]
        } else {
            vec![EDGE_TYPE]
        },
        "provenance_dataset": PROVENANCE_DATASET,
        "source_inputs": source_input_contract(args)?,
        "edges": draft.edges.len(),
        "nodes": draft.nodes.len(),
        "frontier_edges": draft.skip.frontier_edges_built,
        "frontier_nodes": draft.skip.frontier_nodes_built,
        "skipped_total": draft.skip.skipped_total,
    }))
    .map_err(|error| CliError::runtime(format!("serialize graph metadata: {error}")))?;
    graph.put_metadata("citation_overlay_summary", &metadata_value)?;
    let projection = build_csr_projection(
        &storage_collection,
        vault.snapshot(),
        &node_values,
        &edge_values,
    )?;
    graph.write_csr_projection(projection)?;
    vault.flush()?;
    drop(graph);
    drop(vault);

    let physical =
        PhysicalPlainGraph::open_generation_unchecked(&resolved.path, &collection, &generation)?;
    if physical.storage_collection() != storage_collection {
        return Err(CliError::from(CalyxError {
            code: "CALYX_CITATION_OVERLAY_GENERATION_ROUTE_MISMATCH",
            message: format!(
                "citation generation {collection}/{generation} reopened storage {}, expected {storage_collection}",
                physical.storage_collection()
            ),
            remediation: "do not publish the citation generation until its deterministic storage route reads back exactly",
        }));
    }
    read_back_nodes(&physical, &node_values)?;
    let physical_edge_out_keys = read_back_edges(&physical, &edge_values)?;
    let raw = physical.read_csr_bytes()?.ok_or_else(|| {
        CliError::from(CalyxError {
            code: "CALYX_CITATION_OVERLAY_CSR_READBACK_MISSING",
            message: format!("persisted CSR row is missing for citation overlay {collection}"),
            remediation: "rerun materialize-citation-overlay and inspect Graph CF flush state",
        })
    })?;
    let csr = physical.read_csr()?.ok_or_else(|| {
        CliError::from(CalyxError {
            code: "CALYX_CITATION_OVERLAY_CSR_DECODE_MISSING",
            message: format!("persisted CSR row did not decode for collection {collection}"),
            remediation: "rerun materialize-citation-overlay and inspect CSR segment rows",
        })
    })?;
    let assoc = physical.assoc_graph()?;
    if node_values.len() != draft.nodes.len()
        || physical_edge_out_keys != draft.edges.len()
        || csr.nodes.len() != draft.nodes.len()
        || csr.edges.len() != draft.edges.len()
        || assoc.node_count() != draft.nodes.len()
    {
        return Err(CliError::from(CalyxError {
            code: "CALYX_CITATION_OVERLAY_GRAPH_READBACK_MISMATCH",
            message: format!(
                "Graph CF readback mismatch for collection={collection}: expected nodes={} edges={}, physical edges={physical_edge_out_keys}, csr nodes={} edges={}, assoc nodes={}",
                draft.nodes.len(),
                draft.edges.len(),
                csr.nodes.len(),
                csr.edges.len(),
                assoc.node_count()
            ),
            remediation: "do not trust the citation overlay collection until Graph CF and CSR counts match",
        }));
    }

    let report = CitationOverlayReport {
        status: "ok",
        vault: resolved.name.clone(),
        vault_id: resolved.vault_id.to_string(),
        vault_dir: resolved.path.display().to_string(),
        collection: collection.clone(),
        graph_generation: generation.clone(),
        graph_storage_collection: storage_collection,
        provenance_dataset: PROVENANCE_DATASET,
        edge_type: if args.frontier.is_some() {
            "cites+cites_outside_corpus"
        } else {
            EDGE_TYPE
        },
        skip_report: draft.skip,
        readback: CitationOverlayReadback {
            source_of_truth: "isolated physical Aster Graph CF generation via exact node/edge/CSR readback",
            node_rows_written: draft.nodes.len(),
            edge_rows_written: draft.edges.len(),
            physical_node_keys: node_values.len(),
            physical_edge_out_keys,
            csr_nodes: csr.nodes.len(),
            csr_edges: csr.edges.len(),
            assoc_graph_nodes: assoc.node_count(),
            assoc_graph_edges: assoc.edge_count(),
            csr_bytes: raw.len(),
            csr_sha256: sha256_hex(&raw),
            all_node_values_read_back: true,
            all_edge_values_read_back: true,
            graph_wal_batches,
            graph_wal_rows_per_batch_cap: GRAPH_WAL_ROWS_PER_BATCH,
        },
    };
    accept_generation(&resolved, &collection, &generation, &report)?;
    write_skip_report(args, &report.skip_report)?;
    write_report(args, &report)?;
    Ok(report)
}

fn edge_value(edge: &CitesEdge) -> serde_json::Value {
    json!({
        "edge_type": edge.edge_type,
        "weight": edge.weight,
        "depth": edge.depth,
        "citing_opinion_id": edge.citing_opinion_id,
        "cited_opinion_id": edge.cited_opinion_id,
        "provenance_dataset": PROVENANCE_DATASET,
        "source_row_id": edge.source_row_id,
        "source_citation_count": edge.source_citations.len(),
        "source_citations": edge.source_citations,
    })
}

fn node_props(node: &OpinionNode) -> AsterAssocNodeProps {
    let mut metadata = BTreeMap::new();
    metadata.insert("opinion_id".to_string(), node.opinion_id.clone());
    metadata.insert("cx_id".to_string(), node.cx_id.to_string());
    metadata.insert("node_type".to_string(), node.node_type.to_string());
    metadata.insert(
        "schema".to_string(),
        if node.node_type == FRONTIER_NODE_TYPE {
            FRONTIER_SCHEMA
        } else {
            SCHEMA
        }
        .to_string(),
    );
    if let Some(name) = &node.authority_name {
        metadata.insert("authority_name".to_string(), name.clone());
    }
    if let Some(reason) = node.boundary_reason {
        metadata.insert("boundary_reason".to_string(), reason.to_string());
    }
    AsterAssocNodeProps {
        anchors: vec![
            AnchorKind::Label("legal_citation_overlay".to_string()),
            AnchorKind::Label(format!("legal_citation_overlay:{}", node.node_type)),
        ],
        metadata,
        ..Default::default()
    }
}

pub(crate) fn build_csr_projection(
    collection: &str,
    snapshot: u64,
    node_values: &BTreeMap<CxId, Vec<u8>>,
    edge_values: &[(CxId, String, CxId, Vec<u8>)],
) -> CliResult<PlainGraphCsr> {
    let mut nodes = node_values.keys().copied().collect::<Vec<_>>();
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

pub(crate) fn read_back_nodes(
    physical: &PhysicalPlainGraph,
    node_values: &BTreeMap<CxId, Vec<u8>>,
) -> CliResult {
    let physical_nodes = physical.node_props()?;
    if physical_nodes.len() != node_values.len() {
        return Err(CliError::from(CalyxError {
            code: "CALYX_CITATION_OVERLAY_NODE_KEY_READBACK_MISMATCH",
            message: format!(
                "physical Graph CF node range count mismatch: expected {} read {}",
                node_values.len(),
                physical_nodes.len()
            ),
            remediation: "do not trust the citation overlay until the physical node count matches",
        }));
    }
    for (id, actual) in physical_nodes {
        let expected = node_values.get(&id).ok_or_else(|| {
            CliError::from(CalyxError {
                code: "CALYX_CITATION_OVERLAY_NODE_READBACK_EXTRA",
                message: format!("physical Graph CF node row {id} was not in the written node set"),
                remediation: "do not trust the citation overlay until every node reads back",
            })
        })?;
        if actual != *expected {
            return Err(CliError::from(CalyxError {
                code: "CALYX_CITATION_OVERLAY_NODE_READBACK_MISMATCH",
                message: format!("physical Graph CF node row {id} differed after flush"),
                remediation: "do not trust the citation overlay until the node value mismatch is fixed",
            }));
        }
    }
    Ok(())
}

pub(crate) fn read_back_edges(
    physical: &PhysicalPlainGraph,
    edge_values: &[(CxId, String, CxId, Vec<u8>)],
) -> CliResult<usize> {
    let physical_edges = physical.edge_out_props()?;
    let expected = edge_values
        .iter()
        .map(|(src, edge_type, dst, value)| ((*src, edge_type.clone(), *dst), value))
        .collect::<BTreeMap<_, _>>();
    if physical_edges.len() != expected.len() {
        return Err(CliError::from(CalyxError {
            code: "CALYX_CITATION_OVERLAY_EDGE_KEY_READBACK_MISMATCH",
            message: format!(
                "physical Graph CF edge range count mismatch: expected {} read {}",
                expected.len(),
                physical_edges.len()
            ),
            remediation: "do not trust the citation overlay until the physical edge count matches",
        }));
    }
    let mut seen = BTreeSet::new();
    for edge in physical_edges {
        let key = (edge.src, edge.edge_type, edge.dst);
        let expected_value = expected.get(&key).ok_or_else(|| {
            CliError::from(CalyxError {
                code: "CALYX_CITATION_OVERLAY_EDGE_READBACK_EXTRA",
                message: format!("physical edge row {} -{}-> {} not in written set", key.0, key.1, key.2),
                remediation: "do not trust the citation overlay until every edge reads back exactly",
            })
        })?;
        if edge.value != **expected_value {
            return Err(CliError::from(CalyxError {
                code: "CALYX_CITATION_OVERLAY_EDGE_READBACK_MISMATCH",
                message: format!(
                    "physical edge row {} -{}-> {} differed after flush",
                    key.0, key.1, key.2
                ),
                remediation: "do not trust the citation overlay until the edge value mismatch is fixed",
            }));
        }
        seen.insert(key);
    }
    Ok(seen.len())
}

fn accept_generation(
    resolved: &ResolvedVault,
    collection: &str,
    generation: &str,
    report: &CitationOverlayReport,
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
    lifecycle.put_state(
        &GraphCollectionGenerationState::new(
            collection.to_string(),
            generation.to_string(),
            GraphCollectionGenerationStatus::Accepted,
            "materialize-citation-overlay",
        )
        .with_reason("physical graph, CSR, and edge readback passed")
        .with_detail(
            "schema",
            if report.skip_report.frontier_edges_built > 0 {
                FRONTIER_SCHEMA
            } else {
                SCHEMA
            },
        )
        .with_detail("node_rows", report.readback.node_rows_written.to_string())
        .with_detail("edge_rows", report.readback.edge_rows_written.to_string())
        .with_detail("csr_sha256", report.readback.csr_sha256.clone())
        .with_detail(
            GRAPH_GENERATION_STORAGE_SCHEMA_DETAIL,
            GRAPH_GENERATION_STORAGE_SCHEMA,
        )
        .with_detail(
            GRAPH_GENERATION_STORAGE_COLLECTION_DETAIL,
            report.graph_storage_collection.clone(),
        ),
    )?;
    vault.flush()?;
    drop(vault);
    let lifecycle = PhysicalGraphCollectionLifecycle::open_latest(&resolved.path)?;
    let accepted = lifecycle.list_states()?.into_iter().any(|row| {
        row.state.collection == collection
            && row.state.generation == generation
            && row.state.status == GraphCollectionGenerationStatus::Accepted
            && row
                .state
                .detail
                .get(GRAPH_GENERATION_STORAGE_SCHEMA_DETAIL)
                .map(String::as_str)
                == Some(GRAPH_GENERATION_STORAGE_SCHEMA)
            && row
                .state
                .detail
                .get(GRAPH_GENERATION_STORAGE_COLLECTION_DETAIL)
                == Some(&report.graph_storage_collection)
            && row.state.detail.get("csr_sha256") == Some(&report.readback.csr_sha256)
    });
    if !accepted {
        return Err(CliError::from(CalyxError {
            code: "CALYX_CITATION_OVERLAY_LIFECYCLE_READBACK_MISMATCH",
            message: format!(
                "accepted citation lifecycle row or its storage/hash contract is missing after readback: {collection}/{generation}"
            ),
            remediation: "do not serve the citation collection until accepted lifecycle metadata reads back exactly",
        }));
    }
    let routed = PhysicalPlainGraph::open_latest_isolated(&resolved.path, collection)?;
    if routed.storage_collection() != report.graph_storage_collection {
        return Err(CliError::from(CalyxError {
            code: "CALYX_CITATION_OVERLAY_ACCEPTED_ROUTE_MISMATCH",
            message: format!(
                "accepted citation route resolved to {}, expected {} for {collection}/{generation}",
                routed.storage_collection(),
                report.graph_storage_collection
            ),
            remediation: "do not serve the citation collection until a cold reader resolves the verified generation",
        }));
    }
    let routed_csr = routed.read_csr_bytes()?.ok_or_else(|| {
        CliError::from(CalyxError {
            code: "CALYX_CITATION_OVERLAY_ACCEPTED_CSR_MISSING",
            message: format!(
                "accepted citation route has no CSR bytes for {collection}/{generation}"
            ),
            remediation: "do not serve the citation collection until its accepted CSR reads back",
        })
    })?;
    let routed_hash = sha256_hex(&routed_csr);
    if routed_hash != report.readback.csr_sha256 {
        return Err(CliError::from(CalyxError {
            code: "CALYX_CITATION_OVERLAY_ACCEPTED_CSR_MISMATCH",
            message: format!(
                "accepted citation CSR hash {routed_hash} differs from verified hash {} for {collection}/{generation}",
                report.readback.csr_sha256
            ),
            remediation: "quarantine the accepted generation and inspect physical Graph CF publication",
        }));
    }
    Ok(())
}

fn write_skip_report(args: &MaterializeCitationOverlayArgs, skip: &SkipReport) -> CliResult {
    let Some(path) = &args.skip_report else {
        return Ok(());
    };
    let bytes = serde_json::to_vec_pretty(skip)
        .map_err(|error| CliError::runtime(format!("serialize skip report: {error}")))?;
    write_report_bytes(path, &bytes, "citation overlay skip report")
}

fn write_report(
    args: &MaterializeCitationOverlayArgs,
    report: &CitationOverlayReport,
) -> CliResult {
    let Some(path) = &args.report else {
        return Ok(());
    };
    let bytes = serde_json::to_vec_pretty(report)
        .map_err(|error| CliError::runtime(format!("serialize report: {error}")))?;
    write_report_bytes(path, &bytes, "citation overlay report")
}

fn write_report_bytes(path: &Path, bytes: &[u8], label: &str) -> CliResult {
    write_bytes_atomic_new(path, bytes, label)?;
    let readback = fs::read(path)
        .map_err(|error| CliError::io(format!("read back {label} {}: {error}", path.display())))?;
    if readback != bytes || sha256_hex(&readback) != sha256_hex(bytes) {
        return Err(CliError::runtime(format!(
            "{label} physical readback mismatch at {}",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn source_input_contract(args: &MaterializeCitationOverlayArgs) -> CliResult<serde_json::Value> {
    let mut inputs = vec![
        source_input("idmap", &args.idmap)?,
        source_input("citations", &args.citations)?,
    ];
    if let Some(path) = &args.frontier {
        inputs.push(source_input("frontier", path)?);
    }
    if let Some(path) = &args.frontier_authorities {
        inputs.push(source_input("frontier_authorities", path)?);
    }
    Ok(json!(inputs))
}

fn source_input(role: &str, path: &Path) -> CliResult<serde_json::Value> {
    let bytes = fs::read(path).map_err(|error| {
        CliError::io(format!(
            "read citation overlay {role} source {} for contract: {error}",
            path.display()
        ))
    })?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            CliError::usage(format!(
                "citation overlay {role} path has no UTF-8 filename"
            ))
        })?;
    Ok(json!({
        "role": role,
        "filename": name,
        "bytes": bytes.len(),
        "sha256": sha256_hex(&bytes),
    }))
}
