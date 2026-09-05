//! Typed, fail-closed citation traversal for `kernel-answer` (#1858).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::net::SocketAddr;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::ledger_view::AsterLedgerCfStore;
use calyx_aster::plain_graph::{PhysicalPlainGraph, PlainGraphEdge};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CalyxError, CxId, LedgerRef, SlotId, VaultStore};
use calyx_ledger::{ActorId, EntryKind, LedgerCfStore, RedactionPolicy, SubjectId, decode};
use calyx_lodestar::{
    AsterAssocNodeProps, FsKernelStore, RecallPassMode, TypedPanelFusionHit, TypedPanelFusionLane,
    kernel_health,
};
use calyx_registry::{load_vault_panel_state, require_vault_registry_contracts};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::super::citation_overlay::{
    EDGE_TYPE, FRONTIER_EDGE_TYPE, FRONTIER_NODE_TYPE, FRONTIER_REASON, FRONTIER_SCHEMA,
};
use super::super::kernel_generation::{
    decode_sha256, hex32, load_generation_by_sha256, physical_graph_contract, sha256_bytes,
};
use super::super::vault::{ResolvedVault, vault_salt};
use super::kernel_answer::{
    measure_kernel_query_vectors, nearest_graph_node, parse_graph_props, query_panel_vectors,
    retain_kernel_query,
};
use super::parse::KernelAnswerArgs;
use super::roster::SearchTextRoster;
use crate::cf_read::hex_bytes;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const ANSWER_TYPE: &str = "kernel_citation_answer_v1";
const HOP_TYPE: &str = "kernel_citation_answer_hop_v1";
const FRONTIER_TYPE: &str = "kernel_citation_frontier_v1";

#[derive(Clone)]
pub(super) struct CitationAnswerRequest<'a> {
    pub args: &'a KernelAnswerArgs,
    pub resolved: &'a ResolvedVault,
    pub roster: &'a SearchTextRoster<'a>,
    pub kernel_id: CxId,
    pub kernel_manifest_sha256: &'a str,
    pub embedding_slots: &'a [SlotId],
    pub fusion: &'a str,
    pub rrf_k: u32,
    pub nearest: &'a TypedPanelFusionHit,
    pub admission_threshold: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct CitationAnswerContext {
    answer_id: String,
    query_input_sha256: String,
    kernel_manifest_sha256: String,
    embedding_slots: Vec<u16>,
    fusion: String,
    rrf_k: u32,
    nearest_score: f32,
    nearest_lanes: Vec<TypedPanelFusionLane>,
    admission_threshold: f32,
    resident_addr: String,
    citation_collection: String,
    citation_target_opinion_id: String,
    max_hops: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CitationGraphContract {
    collection: String,
    schema: String,
    metadata_sha256: String,
    node_props_sha256: String,
    csr_sha256: String,
    physical_contract_sha256: String,
    nodes: usize,
    edges: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CitationRecordEvidence {
    cx_id: CxId,
    opinion_id: String,
    node_type: String,
    authority_name: Option<String>,
    boundary_reason: Option<String>,
    base_ledger_seq: Option<u64>,
    base_ledger_hash: Option<String>,
    input_blake3: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CitationHopEvidence {
    hop_index: u32,
    edge_kind: String,
    source: CitationRecordEvidence,
    target: CitationRecordEvidence,
    citation_depth: u32,
    weight_bps: u16,
    provenance_dataset: String,
    source_row_id: String,
    evidence_pointer: CitationEdgePointer,
    graph_value_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CitationEdgePointer {
    collection: String,
    source_cx_id: CxId,
    edge_kind: String,
    target_node_id: CxId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CitationFrontierStop {
    code: String,
    reason: String,
    authority_name: String,
    cited_opinion_id: String,
    boundary_node_id: CxId,
    at_hop: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CitationDerivation {
    traversal_mode: String,
    start_cx_id: CxId,
    start_opinion_id: String,
    target_opinion_id: String,
    graph: CitationGraphContract,
    hops: Vec<CitationHopEvidence>,
    frontier: Option<CitationFrontierStop>,
}

#[derive(Clone, Debug)]
struct ParsedEdge {
    row: PlainGraphEdge,
    value: CitationEdgeValue,
}

#[derive(Clone, Debug, Deserialize)]
struct CitationEdgeValue {
    edge_type: String,
    weight: f64,
    depth: u32,
    citing_opinion_id: String,
    cited_opinion_id: String,
    provenance_dataset: String,
    source_row_id: String,
}

pub(super) fn run(request: CitationAnswerRequest<'_>) -> CliResult {
    let target =
        request.args.citation_target.as_deref().ok_or_else(|| {
            CliError::runtime("citation answer execution requires --citation-target")
        })?;
    let derivation = derive_citation_path(
        request.resolved,
        request.nearest.cx_id,
        &request.args.citation_collection,
        target,
        request.args.max_hops,
    )?;
    let query_sha256 = sha256_bytes(request.args.query.as_bytes());
    let query_hash = decode_sha256(&query_sha256, "query_input_sha256")?;
    let manifest_hash = decode_sha256(request.kernel_manifest_sha256, "kernel_manifest_sha256")?;
    let answer_id = citation_answer_id(
        request.resolved.vault_id.to_string().as_bytes(),
        &query_hash,
        &manifest_hash,
        &request.args.citation_collection,
        target,
        request.args.max_hops,
    );
    retain_kernel_query(
        &request.resolved.path,
        &query_sha256,
        request.args.query.as_bytes(),
    )?;
    let context = CitationAnswerContext {
        answer_id: hex32(&answer_id),
        query_input_sha256: query_sha256,
        kernel_manifest_sha256: request.kernel_manifest_sha256.to_string(),
        embedding_slots: request
            .embedding_slots
            .iter()
            .map(|slot| slot.get())
            .collect(),
        fusion: request.fusion.to_string(),
        rrf_k: request.rrf_k,
        nearest_score: request.nearest.score,
        nearest_lanes: request.nearest.lanes.clone(),
        admission_threshold: request.admission_threshold,
        resident_addr: request
            .args
            .resident_addr
            .map(|addr| addr.to_string())
            .unwrap_or_else(|| "local-cpu".to_string()),
        citation_collection: request.args.citation_collection.clone(),
        citation_target_opinion_id: target.to_string(),
        max_hops: request.args.max_hops,
    };
    let derivation_hash = citation_derivation_hash(&context, &derivation)?;
    let (refs, complete_payload) = publish_answer(
        request.resolved,
        request.kernel_id,
        &answer_id,
        &context,
        &derivation,
        derivation_hash,
    )?;
    verify_published_rows(&request.resolved.path, &answer_id, &refs, &complete_payload)?;
    request.roster.emit_runtime_line();
    print_json(&json!({
        "status": if derivation.frontier.is_some() { "frontier" } else { "grounded" },
        "answer_id": hex32(&answer_id),
        "traversal_mode": "citation",
        "query_cx_id": request.nearest.cx_id.to_string(),
        "kernel_id": request.kernel_id.to_string(),
        "kernel_manifest_sha256": request.kernel_manifest_sha256,
        "embedding_slots": context.embedding_slots,
        "fusion": context.fusion,
        "rrf_k": context.rrf_k,
        "nearest_score": context.nearest_score,
        "nearest_lanes": context.nearest_lanes,
        "admission_threshold": context.admission_threshold,
        "citation_graph": derivation.graph,
        "typed_hops": derivation.hops,
        "frontier": derivation.frontier,
        "derivation_hash": hex32(&derivation_hash),
        "ledger_refs": refs.iter().map(ledger_ref_json).collect::<Vec<_>>(),
        "physical_readback": "verified",
    }))
}

pub(crate) fn rederive_kernel_citation_answer_hash(
    resolved: &ResolvedVault,
    answer_id: &[u8],
    payload: &Value,
    resident_override: Option<SocketAddr>,
) -> CliResult<[u8; 32]> {
    if payload.get("type").and_then(Value::as_str) != Some(ANSWER_TYPE) {
        return Err(CliError::runtime(
            "citation reproduce received a non-citation Answer payload",
        ));
    }
    let recorded_context: CitationAnswerContext = serde_json::from_value(
        payload
            .get("context")
            .cloned()
            .ok_or_else(|| CliError::runtime("citation Answer context is missing"))?,
    )
    .map_err(|error| CliError::runtime(format!("decode citation Answer context: {error}")))?;
    if recorded_context.answer_id != hex_bytes(answer_id) {
        return Err(CliError::runtime(
            "citation Answer id differs from its ledger subject",
        ));
    }
    let loaded =
        load_generation_by_sha256(&resolved.path, &recorded_context.kernel_manifest_sha256)?;
    let slots = recorded_context
        .embedding_slots
        .iter()
        .copied()
        .map(SlotId::new)
        .collect::<Vec<_>>();
    if slots != loaded.manifest.graph.embedding_slots
        || recorded_context.fusion != loaded.manifest.graph.fusion
        || recorded_context.rrf_k != loaded.manifest.graph.rrf_k
    {
        return Err(CliError::runtime(
            "citation Answer panel contract differs from its kernel manifest",
        ));
    }
    let query_path = resolved
        .path
        .join("inputs")
        .join("queries")
        .join(format!("{}.txt", recorded_context.query_input_sha256));
    let query = fs::read(&query_path).map_err(|error| {
        CliError::io(format!(
            "read retained citation query {}: {error}",
            query_path.display()
        ))
    })?;
    if sha256_bytes(&query) != recorded_context.query_input_sha256 {
        return Err(CliError::runtime(
            "retained citation query fails its recorded SHA-256",
        ));
    }
    let query = std::str::from_utf8(&query)
        .map_err(|error| CliError::runtime(format!("citation query is not UTF-8: {error}")))?;
    let resident_addr = match resident_override {
        Some(addr) => Some(addr),
        None if recorded_context.resident_addr == "local-cpu" => None,
        None => Some(super::parse::parse_resident_addr(
            &recorded_context.resident_addr,
        )?),
    };
    require_vault_registry_contracts(&resolved.path)?;
    let state = load_vault_panel_state(&resolved.path)?;
    let roster = SearchTextRoster::derive(&state);
    let measured = measure_kernel_query_vectors(&state, &roster, resolved, query, resident_addr)?;
    let panel = query_panel_vectors(&measured, &slots)?;
    let association = PhysicalPlainGraph::open_latest_isolated(
        &resolved.path,
        calyx_lodestar::PANEL_ASTER_ASSOC_COLLECTION,
    )?;
    let props = parse_graph_props(&association.node_props()?, &slots)?;
    let nearest = nearest_graph_node(&props, &panel, &slots)?;
    if nearest.score < loaded.manifest.admission.threshold {
        return Err(CalyxError::reproduce_drift_exceeded(
            "citation query fell below its immutable admission threshold",
        )
        .into());
    }
    let store = FsKernelStore::new(&resolved.path);
    let health = kernel_health(loaded.manifest.kernel_id, &store)?;
    if health.recall.pass_mode != RecallPassMode::Passed || health.grounded_fraction != 1.0 {
        return Err(CalyxError::kernel_ungrounded(
            "citation reproduce refuses an unhealthy immutable kernel",
        )
        .into());
    }
    let derivation = derive_citation_path(
        resolved,
        nearest.cx_id,
        &recorded_context.citation_collection,
        &recorded_context.citation_target_opinion_id,
        recorded_context.max_hops,
    )?;
    let reproduced_context = CitationAnswerContext {
        nearest_score: nearest.score,
        nearest_lanes: nearest.lanes,
        admission_threshold: loaded.manifest.admission.threshold,
        resident_addr: recorded_context.resident_addr.clone(),
        ..recorded_context.clone()
    };
    citation_derivation_hash(&reproduced_context, &derivation)
}

fn derive_citation_path(
    resolved: &ResolvedVault,
    start: CxId,
    collection: &str,
    target_opinion_id: &str,
    max_hops: usize,
) -> CliResult<CitationDerivation> {
    let physical = PhysicalPlainGraph::open_latest(&resolved.path, collection)?;
    let metadata = physical
        .get_metadata("citation_overlay_summary")?
        .ok_or_else(|| CliError::runtime("citation overlay summary metadata is missing"))?;
    let metadata_json: Value = serde_json::from_slice(&metadata)
        .map_err(|error| CliError::runtime(format!("decode citation overlay metadata: {error}")))?;
    let schema = metadata_json
        .get("schema")
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::runtime("citation overlay schema is missing"))?;
    if schema != FRONTIER_SCHEMA {
        return Err(CliError::from(CalyxError {
            code: "CALYX_CITATION_GRAPH_FRONTIER_UNSEALED",
            message: format!("citation collection {collection} has schema {schema}"),
            remediation: "materialize and accept a legal_citation_overlay_v2 collection with frontier rows",
        }));
    }
    let node_rows = physical.node_props()?;
    let csr = physical
        .read_csr_bytes()?
        .ok_or_else(|| CliError::runtime("citation overlay CSR is missing"))?;
    let (physical_hash, node_hash) = physical_graph_contract(&metadata, &node_rows, &csr);
    let graph_contract = CitationGraphContract {
        collection: collection.to_string(),
        schema: schema.to_string(),
        metadata_sha256: sha256_bytes(&metadata),
        node_props_sha256: node_hash,
        csr_sha256: sha256_bytes(&csr),
        physical_contract_sha256: hex32(&physical_hash),
        nodes: node_rows.len(),
        edges: physical.edge_out_props()?.len(),
    };
    let nodes = parse_citation_nodes(&node_rows)?;
    let start_node = nodes.get(&start).ok_or_else(|| {
        CliError::from(CalyxError {
            code: "CALYX_CITATION_START_NOT_IN_OVERLAY",
            message: format!("nearest constellation {start} is absent from citation collection {collection}"),
            remediation: "query an opinion represented in the sealed citation overlay or choose association traversal",
        })
    })?;
    if start_node.node_type != "opinion" {
        return Err(CliError::runtime(
            "citation traversal cannot start from a frontier node",
        ));
    }
    let targets = nodes
        .iter()
        .filter(|(_, node)| node.opinion_id == target_opinion_id)
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    if targets.len() != 1 {
        return Err(CliError::from(CalyxError {
            code: "CALYX_CITATION_TARGET_NOT_UNIQUE",
            message: format!(
                "citation target opinion {target_opinion_id} resolves to {} graph nodes",
                targets.len()
            ),
            remediation: "materialize exactly one live or frontier node per target opinion identity",
        }));
    }
    let target = targets[0];
    let mut adjacency: BTreeMap<CxId, Vec<ParsedEdge>> = BTreeMap::new();
    for row in physical.edge_out_props()? {
        if row.edge_type != EDGE_TYPE && row.edge_type != FRONTIER_EDGE_TYPE {
            continue;
        }
        let value: CitationEdgeValue = serde_json::from_slice(&row.value).map_err(|error| {
            CliError::runtime(format!(
                "decode citation edge {} -{}-> {}: {error}",
                row.src, row.edge_type, row.dst
            ))
        })?;
        if value.edge_type != row.edge_type {
            return Err(CliError::runtime(format!(
                "citation edge key type {} differs from value {}",
                row.edge_type, value.edge_type
            )));
        }
        adjacency
            .entry(row.src)
            .or_default()
            .push(ParsedEdge { row, value });
    }
    for edges in adjacency.values_mut() {
        edges.sort_by(|left, right| {
            left.row
                .edge_type
                .cmp(&right.row.edge_type)
                .then(left.row.dst.cmp(&right.row.dst))
        });
    }
    let path = shortest_typed_path(start, target, max_hops, &nodes, &adjacency)?;
    let base = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions {
            restore_mvcc_rows: false,
            restore_ledger_hook: false,
            read_only: true,
            selected_cfs: Some(vec![ColumnFamily::Base]),
            ..VaultOptions::default()
        },
    )?;
    let ledger = AsterLedgerCfStore::open(&resolved.path)?;
    let mut hops = Vec::with_capacity(path.len());
    for (index, edge) in path.iter().enumerate() {
        let source_node = nodes
            .get(&edge.row.src)
            .ok_or_else(|| CliError::runtime("citation edge source node is missing"))?;
        let target_node = nodes
            .get(&edge.row.dst)
            .ok_or_else(|| CliError::runtime("citation edge target node is missing"))?;
        if edge.value.citing_opinion_id != source_node.opinion_id
            || edge.value.cited_opinion_id != target_node.opinion_id
        {
            return Err(CliError::runtime(format!(
                "citation edge opinion identities differ from endpoint node metadata at {} -> {}",
                edge.row.src, edge.row.dst
            )));
        }
        let persisted = physical
            .get_edge(edge.row.src, &edge.row.edge_type, edge.row.dst)?
            .ok_or_else(|| CliError::runtime("citation edge disappeared during readback"))?;
        if persisted != edge.row.value {
            return Err(CliError::runtime(
                "citation edge physical readback differs from traversal bytes",
            ));
        }
        hops.push(CitationHopEvidence {
            hop_index: u32::try_from(index + 1)
                .map_err(|_| CliError::runtime("citation hop index overflow"))?,
            edge_kind: edge.row.edge_type.clone(),
            source: record_evidence(&base, &ledger, edge.row.src, source_node)?,
            target: record_evidence(&base, &ledger, edge.row.dst, target_node)?,
            citation_depth: edge.value.depth,
            weight_bps: citation_weight_bps(edge.value.depth, edge.value.weight)?,
            provenance_dataset: edge.value.provenance_dataset.clone(),
            source_row_id: edge.value.source_row_id.clone(),
            evidence_pointer: CitationEdgePointer {
                collection: collection.to_string(),
                source_cx_id: edge.row.src,
                edge_kind: edge.row.edge_type.clone(),
                target_node_id: edge.row.dst,
            },
            graph_value_sha256: sha256_bytes(&persisted),
        });
    }
    let frontier = hops.last().and_then(|hop| {
        (hop.edge_kind == FRONTIER_EDGE_TYPE).then(|| CitationFrontierStop {
            code: "CALYX_CITATION_FRONTIER".to_string(),
            reason: hop
                .target
                .boundary_reason
                .clone()
                .unwrap_or_else(|| FRONTIER_REASON.to_string()),
            authority_name: hop
                .target
                .authority_name
                .clone()
                .unwrap_or_else(|| format!("CourtListener opinion {}", hop.target.opinion_id)),
            cited_opinion_id: hop.target.opinion_id.clone(),
            boundary_node_id: hop.target.cx_id,
            at_hop: hop.hop_index,
        })
    });
    Ok(CitationDerivation {
        traversal_mode: "citation".to_string(),
        start_cx_id: start,
        start_opinion_id: start_node.opinion_id.clone(),
        target_opinion_id: target_opinion_id.to_string(),
        graph: graph_contract,
        hops,
        frontier,
    })
}

fn parse_citation_nodes(
    rows: &[(CxId, Vec<u8>)],
) -> CliResult<BTreeMap<CxId, CitationRecordEvidence>> {
    rows.iter()
        .map(|(id, bytes)| {
            let props: AsterAssocNodeProps = serde_json::from_slice(bytes).map_err(|error| {
                CliError::runtime(format!("decode citation node {id}: {error}"))
            })?;
            let metadata = props.metadata;
            let opinion_id = metadata
                .get("opinion_id")
                .filter(|value| !value.is_empty())
                .cloned()
                .ok_or_else(|| {
                    CliError::runtime(format!("citation node {id} has no opinion_id"))
                })?;
            let node_type = metadata
                .get("node_type")
                .filter(|value| !value.is_empty())
                .cloned()
                .ok_or_else(|| CliError::runtime(format!("citation node {id} has no node_type")))?;
            Ok((
                *id,
                CitationRecordEvidence {
                    cx_id: *id,
                    opinion_id,
                    node_type,
                    authority_name: metadata.get("authority_name").cloned(),
                    boundary_reason: metadata.get("boundary_reason").cloned(),
                    base_ledger_seq: None,
                    base_ledger_hash: None,
                    input_blake3: None,
                },
            ))
        })
        .collect()
}

fn shortest_typed_path(
    start: CxId,
    target: CxId,
    max_hops: usize,
    nodes: &BTreeMap<CxId, CitationRecordEvidence>,
    adjacency: &BTreeMap<CxId, Vec<ParsedEdge>>,
) -> CliResult<Vec<ParsedEdge>> {
    let mut queue = VecDeque::from([(start, 0_usize)]);
    let mut seen = BTreeSet::from([start]);
    let mut parent: BTreeMap<CxId, ParsedEdge> = BTreeMap::new();
    while let Some((node, depth)) = queue.pop_front() {
        if depth >= max_hops {
            continue;
        }
        for edge in adjacency.get(&node).into_iter().flatten() {
            if !seen.insert(edge.row.dst) {
                continue;
            }
            parent.insert(edge.row.dst, edge.clone());
            if edge.row.dst == target {
                return reconstruct_path(start, target, &parent);
            }
            let target_node = nodes
                .get(&edge.row.dst)
                .ok_or_else(|| CliError::runtime("citation adjacency targets a missing node"))?;
            if target_node.node_type != FRONTIER_NODE_TYPE {
                queue.push_back((edge.row.dst, depth + 1));
            }
        }
    }
    Err(CliError::from(CalyxError {
        code: "CALYX_CITATION_TARGET_UNREACHABLE",
        message: format!(
            "no typed citation path from {start} to opinion target within {max_hops} hops"
        ),
        remediation: "increase --max-hops within the bound or choose a target reachable from the queried opinion",
    }))
}

fn reconstruct_path(
    start: CxId,
    mut current: CxId,
    parent: &BTreeMap<CxId, ParsedEdge>,
) -> CliResult<Vec<ParsedEdge>> {
    let mut path = Vec::new();
    while current != start {
        let edge = parent
            .get(&current)
            .cloned()
            .ok_or_else(|| CliError::runtime("citation BFS parent chain is incomplete"))?;
        current = edge.row.src;
        path.push(edge);
    }
    path.reverse();
    Ok(path)
}

fn record_evidence(
    vault: &AsterVault,
    ledger: &AsterLedgerCfStore,
    id: CxId,
    node: &CitationRecordEvidence,
) -> CliResult<CitationRecordEvidence> {
    if node.node_type == FRONTIER_NODE_TYPE {
        if vault.get_base(id, vault.snapshot()).is_ok() {
            return Err(CliError::runtime(format!(
                "frontier node {id} unexpectedly resolves to a live Base record"
            )));
        }
        if node.boundary_reason.as_deref() != Some(FRONTIER_REASON)
            || node.authority_name.as_deref().is_none_or(str::is_empty)
        {
            return Err(CliError::runtime(format!(
                "frontier node {id} lacks a named fail-closed boundary contract"
            )));
        }
        return Ok(node.clone());
    }
    let base = vault.get_base(id, vault.snapshot())?;
    if base.cx_id != id {
        return Err(CliError::runtime("citation Base row identity mismatch"));
    }
    let row = ledger
        .read_seq(base.provenance.seq)?
        .ok_or_else(|| CliError::runtime("citation Base provenance ledger row is missing"))?;
    let entry = decode(&row.bytes)?;
    if row.seq != base.provenance.seq
        || entry.seq != base.provenance.seq
        || entry.entry_hash != base.provenance.hash
    {
        return Err(CliError::runtime(
            "citation Base provenance differs from physical Ledger CF bytes",
        ));
    }
    let mut evidence = node.clone();
    evidence.base_ledger_seq = Some(base.provenance.seq);
    evidence.base_ledger_hash = Some(hex_bytes(&base.provenance.hash));
    evidence.input_blake3 = Some(hex_bytes(&base.input_ref.hash));
    Ok(evidence)
}

fn citation_weight_bps(depth: u32, stored_weight: f64) -> CliResult<u16> {
    let expected = depth.min(10) * 1_000;
    let observed = (stored_weight * 10_000.0).round();
    if !stored_weight.is_finite() || observed != f64::from(expected) {
        return Err(CliError::runtime(format!(
            "citation depth {depth} disagrees with stored weight {stored_weight}"
        )));
    }
    u16::try_from(expected).map_err(|_| CliError::runtime("citation weight overflow"))
}

fn publish_answer(
    resolved: &ResolvedVault,
    kernel_id: CxId,
    answer_id: &[u8; 32],
    context: &CitationAnswerContext,
    derivation: &CitationDerivation,
    derivation_hash: [u8; 32],
) -> CliResult<(Vec<LedgerRef>, Vec<u8>)> {
    let subject = SubjectId::Query(answer_id.to_vec());
    let actor = ActorId::Service("calyx-kernel-citation-answer".to_string());
    let mut entries = derivation
        .hops
        .iter()
        .map(|hop| {
            let payload = checked_payload(
                json!({
                    "type": HOP_TYPE,
                    "answer_id": context.answer_id,
                    "hop": hop,
                }),
                "citation hop",
            )?;
            Ok((EntryKind::Answer, subject.clone(), payload, actor.clone()))
        })
        .collect::<CliResult<Vec<_>>>()?;
    if let Some(frontier) = &derivation.frontier {
        let payload = checked_payload(
            json!({
                "type": FRONTIER_TYPE,
                "answer_id": context.answer_id,
                "frontier": frontier,
            }),
            "citation frontier",
        )?;
        entries.push((EntryKind::Answer, subject.clone(), payload, actor.clone()));
    }
    let prefix_count = entries.len();
    let context_for_final = context.clone();
    let derivation_for_final = derivation.clone();
    let subject_for_final = subject.clone();
    let actor_for_final = actor.clone();
    let vault = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::default(),
    )?;
    let refs = vault.append_ledger_entries_with_final(entries, |prefix_refs| {
        let payload = complete_payload(
            kernel_id,
            &context_for_final,
            &derivation_for_final,
            derivation_hash,
            prefix_refs,
        )
        .map_err(|error| CalyxError::ledger_group_commit_failed(error.to_string()))?;
        Ok((
            EntryKind::Answer,
            subject_for_final,
            payload,
            actor_for_final,
        ))
    })?;
    if refs.len() != prefix_count + 1 {
        return Err(CliError::runtime(
            "atomic citation Answer publication returned an unexpected ledger ref count",
        ));
    }
    let complete_payload = complete_payload(
        kernel_id,
        context,
        derivation,
        derivation_hash,
        &refs[..prefix_count],
    )?;
    Ok((refs, complete_payload))
}

fn complete_payload(
    kernel_id: CxId,
    context: &CitationAnswerContext,
    derivation: &CitationDerivation,
    derivation_hash: [u8; 32],
    prefix_refs: &[LedgerRef],
) -> CliResult<Vec<u8>> {
    checked_payload(
        json!({
            "type": ANSWER_TYPE,
            "answer_id": context.answer_id,
            "query_id": derivation.start_cx_id,
            "kernel_id": kernel_id,
            "context": context,
            "derivation": derivation,
            "derivation_hash": hex32(&derivation_hash),
            "prefix_ledger_refs": prefix_refs.iter().map(ledger_ref_json).collect::<Vec<_>>(),
        }),
        "citation answer completion",
    )
}

fn verify_published_rows(
    vault_dir: &std::path::Path,
    answer_id: &[u8; 32],
    refs: &[LedgerRef],
    expected_complete: &[u8],
) -> CliResult {
    let physical = AsterLedgerCfStore::open(vault_dir)?;
    for (index, reference) in refs.iter().enumerate() {
        let row = physical
            .read_seq(reference.seq)?
            .ok_or_else(|| CliError::runtime("citation Answer ledger row is physically absent"))?;
        let entry = decode(&row.bytes)?;
        if entry.entry_hash != reference.hash
            || entry.kind != EntryKind::Answer
            || !matches!(&entry.subject, SubjectId::Query(id) if id == answer_id)
        {
            return Err(CliError::runtime(
                "citation Answer ledger physical readback differs from its receipt",
            ));
        }
        if index + 1 == refs.len() && entry.payload != expected_complete {
            return Err(CliError::runtime(
                "citation Answer completion bytes differ after physical readback",
            ));
        }
    }
    Ok(())
}

fn checked_payload(value: Value, label: &str) -> CliResult<Vec<u8>> {
    let bytes = serde_json::to_vec(&value)
        .map_err(|error| CliError::runtime(format!("encode {label}: {error}")))?;
    RedactionPolicy::check_payload(&bytes)?;
    Ok(bytes)
}

fn citation_derivation_hash(
    context: &CitationAnswerContext,
    derivation: &CitationDerivation,
) -> CliResult<[u8; 32]> {
    let bytes = serde_json::to_vec(&json!({
        "schema_version": 1,
        "context": context,
        "derivation": derivation,
    }))
    .map_err(|error| CliError::runtime(format!("encode citation derivation: {error}")))?;
    Ok(*blake3::hash(&bytes).as_bytes())
}

fn citation_answer_id(
    vault_id: &[u8],
    query_sha256: &[u8; 32],
    manifest_sha256: &[u8; 32],
    collection: &str,
    target: &str,
    max_hops: usize,
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"calyx-kernel-citation-answer-v1");
    hasher.update(&(vault_id.len() as u64).to_be_bytes());
    hasher.update(vault_id);
    hasher.update(query_sha256);
    hasher.update(manifest_sha256);
    hasher.update(&(collection.len() as u64).to_be_bytes());
    hasher.update(collection.as_bytes());
    hasher.update(&(target.len() as u64).to_be_bytes());
    hasher.update(target.as_bytes());
    hasher.update(&(max_hops as u64).to_be_bytes());
    *hasher.finalize().as_bytes()
}

fn ledger_ref_json(reference: &LedgerRef) -> Value {
    json!({
        "seq": reference.seq,
        "hash": hex_bytes(&reference.hash),
    })
}
