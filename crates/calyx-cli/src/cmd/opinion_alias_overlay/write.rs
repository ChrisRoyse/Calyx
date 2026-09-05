use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::plain_graph::{
    GRAPH_GENERATION_STORAGE_COLLECTION_DETAIL, GRAPH_GENERATION_STORAGE_SCHEMA,
    GRAPH_GENERATION_STORAGE_SCHEMA_DETAIL, GraphCollectionGenerationState,
    GraphCollectionGenerationStatus, GraphCollectionLifecycle, PhysicalGraphCollectionLifecycle,
    PhysicalPlainGraph, PlainGraph, graph_generation_storage_collection,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, CalyxError, CxId, VaultStore};
use calyx_lodestar::{AsterAssocNodeProps, encode_assoc_node_props};
use serde::Serialize;
use serde_json::json;

use super::{
    AliasInput, AliasRow, EDGE_TYPE, MaterializeArgs, SCHEMA, SOURCE_NODE_TYPE, SUMMARY_KEY,
    TARGET_NODE_TYPE, alias_node_id, contract_error, sha256_hex,
};
use crate::cmd::vault::{ResolvedVault, resolve_vault_info, vault_salt};
use crate::durable_write::write_bytes_atomic_new;
use crate::error::{CliError, CliResult};

const GRAPH_WAL_ROWS_PER_BATCH: usize = 10_000;

type GraphNodeValues = BTreeMap<CxId, Vec<u8>>;
type GraphEdgeValue = (CxId, CxId, Vec<u8>);
type ExpectedGraphRows = (GraphNodeValues, Vec<GraphEdgeValue>);

mod graph;
use graph::{build_csr, verify_edges, verify_nodes};

#[derive(Debug, Serialize)]
pub(crate) struct MaterializeReport {
    status: &'static str,
    source_of_truth: &'static str,
    vault: String,
    vault_id: String,
    vault_dir: String,
    collection: String,
    graph_generation: String,
    graph_storage_collection: String,
    schema: &'static str,
    edge_type: &'static str,
    input: InputReport,
    accounting: Accounting,
    readback: Readback,
}

#[derive(Debug, Serialize)]
struct InputReport {
    path: String,
    bytes: usize,
    sha256: String,
}

#[derive(Debug, Serialize)]
struct Accounting {
    source_opinion_aliases: usize,
    canonical_constellations: usize,
    duplicate_aliases: usize,
    graph_nodes: usize,
    aliases_to_edges: usize,
}

#[derive(Debug, Serialize)]
struct Readback {
    base_rows_checked: usize,
    base_metadata_exact: bool,
    physical_node_keys: usize,
    physical_edge_out_keys: usize,
    all_node_values_exact: bool,
    all_edge_values_exact: bool,
    metadata_sha256: String,
    csr_nodes: usize,
    csr_edges: usize,
    csr_bytes: usize,
    csr_sha256: String,
    graph_wal_batches: usize,
    graph_wal_rows_per_batch_cap: usize,
    accepted_lifecycle_readback: bool,
}

pub(crate) fn materialize(
    home: &Path,
    args: &MaterializeArgs,
    input: &AliasInput,
) -> CliResult<MaterializeReport> {
    preflight_report(args.report.as_deref())?;
    let resolved = resolve_vault_info(home, &args.vault)?;
    reject_existing_collection(&resolved, &args.collection)?;
    let vault = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions {
            restore_mvcc_rows: false,
            ..VaultOptions::default()
        },
    )?;
    validate_base(&vault, input)?;
    let generation = format!("materialize-opinion-aliases-{}", ulid::Ulid::new());
    let storage_collection = graph_generation_storage_collection(&args.collection, &generation)?;
    let lifecycle = GraphCollectionLifecycle::new(&vault)?;
    lifecycle.put_state(
        &GraphCollectionGenerationState::new(
            args.collection.clone(),
            generation.clone(),
            GraphCollectionGenerationStatus::Writing,
            "materialize-opinion-aliases",
        )
        .with_reason("opinion-alias Graph materialization started")
        .with_detail("schema", SCHEMA)
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
    let graph = PlainGraph::new_generation(&vault, &args.collection, &generation)?;
    let (node_values, edge_values) = expected_graph_rows(input)?;
    let mut graph_rows = Vec::with_capacity(node_values.len() + edge_values.len() * 2);
    for (id, value) in &node_values {
        graph_rows.push((ColumnFamily::Graph, graph.node_key(*id), value.clone()));
    }
    for (src, dst, value) in &edge_values {
        let out = graph.edge_out_key(*src, EDGE_TYPE, *dst)?;
        let reverse = graph.edge_in_key(*dst, EDGE_TYPE, *src)?;
        graph_rows.push((ColumnFamily::Graph, out.clone(), value.clone()));
        graph_rows.push((ColumnFamily::Graph, reverse, out));
    }
    let graph_wal_batches = graph_rows.len().div_ceil(GRAPH_WAL_ROWS_PER_BATCH);
    for batch in graph_rows.chunks(GRAPH_WAL_ROWS_PER_BATCH) {
        vault.write_cf_batch(batch.to_vec())?;
    }
    let summary = serde_json::to_vec(&json!({
        "schema": SCHEMA,
        "collection": args.collection,
        "edge_type": EDGE_TYPE,
        "source": {
            "path": input.path,
            "bytes": input.bytes,
            "sha256": input.sha256,
        },
        "source_opinion_aliases": input.rows.len(),
        "canonical_constellations": input.canonical_rows,
        "duplicate_aliases": input.rows.len() - input.canonical_rows,
        "nodes": node_values.len(),
        "edges": edge_values.len(),
    }))
    .map_err(|error| CliError::runtime(format!("serialize opinion-alias summary: {error}")))?;
    graph.put_metadata(SUMMARY_KEY, &summary)?;
    let projection = build_csr(
        &storage_collection,
        vault.snapshot(),
        &node_values,
        &edge_values,
    )?;
    graph.write_csr_projection(projection)?;
    vault.flush()?;
    drop(graph);
    drop(vault);

    let physical = PhysicalPlainGraph::open_generation_unchecked(
        &resolved.path,
        &args.collection,
        &generation,
    )?;
    if physical.storage_collection() != storage_collection {
        return Err(contract_error(
            "CALYX_OPINION_ALIAS_GENERATION_ROUTE_MISMATCH",
            format!(
                "opinion-alias generation {}/{} reopened storage {}, expected {storage_collection}",
                args.collection,
                generation,
                physical.storage_collection()
            ),
            "do not publish the alias generation until its deterministic storage route reads back exactly",
        ));
    }
    verify_nodes(&physical, &node_values)?;
    verify_edges(&physical, &edge_values)?;
    let physical_summary = physical
        .get_metadata(SUMMARY_KEY)?
        .ok_or_else(|| CliError::runtime("physical opinion-alias summary is absent"))?;
    if physical_summary != summary {
        return Err(contract_error(
            "CALYX_OPINION_ALIAS_SUMMARY_READBACK_MISMATCH",
            "physical opinion-alias summary differs after flush",
            "quarantine the collection and inspect Graph CF persistence",
        ));
    }
    let csr_bytes = physical
        .read_csr_bytes()?
        .ok_or_else(|| CliError::runtime("physical opinion-alias CSR is absent"))?;
    let csr = physical
        .read_csr()?
        .ok_or_else(|| CliError::runtime("physical opinion-alias CSR does not decode"))?;
    if csr.nodes.len() != node_values.len() || csr.edges.len() != edge_values.len() {
        return Err(contract_error(
            "CALYX_OPINION_ALIAS_CSR_READBACK_MISMATCH",
            format!(
                "opinion-alias CSR has {} nodes/{} edges; expected {}/{}",
                csr.nodes.len(),
                csr.edges.len(),
                node_values.len(),
                edge_values.len()
            ),
            "quarantine the collection and rebuild its CSR from physical Graph rows",
        ));
    }
    let mut report = MaterializeReport {
        status: "verified",
        source_of_truth: "physical Aster Base CF plus Graph CF node/edge/metadata/CSR readback",
        vault: resolved.name.clone(),
        vault_id: resolved.vault_id.to_string(),
        vault_dir: resolved.path.display().to_string(),
        collection: args.collection.clone(),
        graph_generation: generation.clone(),
        graph_storage_collection: storage_collection,
        schema: SCHEMA,
        edge_type: EDGE_TYPE,
        input: InputReport {
            path: input.path.display().to_string(),
            bytes: input.bytes,
            sha256: input.sha256.clone(),
        },
        accounting: Accounting {
            source_opinion_aliases: input.rows.len(),
            canonical_constellations: input.canonical_rows,
            duplicate_aliases: input.rows.len() - input.canonical_rows,
            graph_nodes: node_values.len(),
            aliases_to_edges: edge_values.len(),
        },
        readback: Readback {
            base_rows_checked: input.canonical_rows,
            base_metadata_exact: true,
            physical_node_keys: physical.node_key_count()?,
            physical_edge_out_keys: physical.edge_out_key_count()?,
            all_node_values_exact: true,
            all_edge_values_exact: true,
            metadata_sha256: sha256_hex(&physical_summary),
            csr_nodes: csr.nodes.len(),
            csr_edges: csr.edges.len(),
            csr_bytes: csr_bytes.len(),
            csr_sha256: sha256_hex(&csr_bytes),
            graph_wal_batches,
            graph_wal_rows_per_batch_cap: GRAPH_WAL_ROWS_PER_BATCH,
            accepted_lifecycle_readback: false,
        },
    };
    accept_generation(&resolved, &args.collection, &generation, &report)?;
    report.readback.accepted_lifecycle_readback = true;
    write_report(args.report.as_deref(), &report)?;
    Ok(report)
}

fn validate_base(vault: &AsterVault, input: &AliasInput) -> CliResult {
    let snapshot = vault.snapshot();
    for row in input.rows.values().filter(|row| row.is_canonical) {
        let base = vault.get(row.cx_id, snapshot).map_err(|error| {
            contract_error(
                "CALYX_OPINION_ALIAS_BASE_TARGET_MISSING",
                format!(
                    "canonical opinion {} target {} is not readable: {error}",
                    row.opinion_id, row.cx_id
                ),
                "rebuild the alias import from an unbounded physical Base readback",
            )
        })?;
        if base.metadata.get("opinion_id") != Some(&row.opinion_id)
            || base.metadata.get("canonical_opinion_id") != Some(&row.canonical_opinion_id)
            || base.metadata.get("ingest_text_sha256") != Some(&row.content_sha256)
        {
            return Err(contract_error(
                "CALYX_OPINION_ALIAS_BASE_TARGET_MISMATCH",
                format!(
                    "canonical opinion {} metadata differs at Base row {}",
                    row.opinion_id, row.cx_id
                ),
                "quarantine the alias import and rebuild from the matching ingest/Base generation",
            ));
        }
    }
    Ok(())
}

fn expected_graph_rows(input: &AliasInput) -> CliResult<ExpectedGraphRows> {
    let mut nodes = BTreeMap::new();
    for row in input.rows.values().filter(|row| row.is_canonical) {
        insert_node(&mut nodes, row.cx_id, target_props(row)?)?;
    }
    let mut edges = Vec::with_capacity(input.rows.len());
    for row in input.rows.values() {
        let alias_id = alias_node_id(&row.opinion_id);
        if nodes.contains_key(&alias_id) {
            return Err(contract_error(
                "CALYX_OPINION_ALIAS_NODE_ID_COLLISION",
                format!("alias node {alias_id} collides with a canonical constellation"),
                "change the versioned alias-node content-address domain before materialization",
            ));
        }
        insert_node(&mut nodes, alias_id, source_props(row)?)?;
        let value = serde_json::to_vec(&json!({
            "schema": SCHEMA,
            "edge_type": EDGE_TYPE,
            "weight": 1.0,
            "opinion_id": row.opinion_id,
            "canonical_opinion_id": row.canonical_opinion_id,
            "canonical_cx_id": row.cx_id,
            "content_sha256": row.content_sha256,
            "is_canonical": row.is_canonical,
            "source_url": row.source_url,
        }))
        .map_err(|error| CliError::runtime(format!("serialize aliases_to edge: {error}")))?;
        edges.push((alias_id, row.cx_id, value));
    }
    Ok((nodes, edges))
}

fn insert_node(nodes: &mut BTreeMap<CxId, Vec<u8>>, id: CxId, value: Vec<u8>) -> CliResult {
    if nodes.insert(id, value).is_some() {
        return Err(contract_error(
            "CALYX_OPINION_ALIAS_NODE_DUPLICATE",
            format!("opinion-alias graph node {id} is duplicated"),
            "repair the alias identity relation before Graph mutation",
        ));
    }
    Ok(())
}

fn source_props(row: &AliasRow) -> CliResult<Vec<u8>> {
    let mut metadata = BTreeMap::new();
    metadata.insert("schema".into(), SCHEMA.into());
    metadata.insert("node_type".into(), SOURCE_NODE_TYPE.into());
    metadata.insert("opinion_id".into(), row.opinion_id.clone());
    metadata.insert(
        "canonical_opinion_id".into(),
        row.canonical_opinion_id.clone(),
    );
    metadata.insert("canonical_cx_id".into(), row.cx_id.to_string());
    metadata.insert("content_sha256".into(), row.content_sha256.clone());
    metadata.insert("is_canonical".into(), row.is_canonical.to_string());
    metadata.insert("source_url".into(), row.source_url.clone());
    encode_assoc_node_props(&AsterAssocNodeProps {
        anchors: vec![
            AnchorKind::Label("legal_opinion_alias".into()),
            AnchorKind::Label(format!("legal_opinion_alias:{SOURCE_NODE_TYPE}")),
        ],
        metadata,
        ..Default::default()
    })
    .map_err(CliError::from)
}

fn target_props(row: &AliasRow) -> CliResult<Vec<u8>> {
    let mut metadata = BTreeMap::new();
    metadata.insert("schema".into(), SCHEMA.into());
    metadata.insert("node_type".into(), TARGET_NODE_TYPE.into());
    metadata.insert(
        "canonical_opinion_id".into(),
        row.canonical_opinion_id.clone(),
    );
    metadata.insert("canonical_cx_id".into(), row.cx_id.to_string());
    metadata.insert("content_sha256".into(), row.content_sha256.clone());
    encode_assoc_node_props(&AsterAssocNodeProps {
        anchors: vec![
            AnchorKind::Label("legal_opinion_alias".into()),
            AnchorKind::Label(format!("legal_opinion_alias:{TARGET_NODE_TYPE}")),
        ],
        metadata,
        ..Default::default()
    })
    .map_err(CliError::from)
}

fn preflight_report(path: Option<&Path>) -> CliResult {
    let Some(path) = path else { return Ok(()) };
    match fs::symlink_metadata(path) {
        Ok(_) => {
            return Err(contract_error(
                "CALYX_OPINION_ALIAS_REPORT_EXISTS",
                format!("opinion-alias report {} already exists", path.display()),
                "choose a new immutable report destination",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(CliError::io(format!(
                "inspect opinion-alias report destination {}: {error}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn reject_existing_collection(resolved: &ResolvedVault, collection: &str) -> CliResult {
    let lifecycle = PhysicalGraphCollectionLifecycle::open_latest(&resolved.path)?;
    if lifecycle
        .list_states()?
        .iter()
        .any(|row| row.state.collection == collection)
    {
        return Err(contract_error(
            "CALYX_OPINION_ALIAS_COLLECTION_EXISTS",
            format!("opinion-alias collection {collection} already has lifecycle state"),
            "use a new versioned collection name; never overwrite accepted alias truth",
        ));
    }
    let physical = PhysicalPlainGraph::open_latest_unchecked(&resolved.path, collection)?;
    let nodes = physical.node_key_count()?;
    let edges = physical.edge_out_key_count()?;
    let metadata_present = physical.get_metadata(SUMMARY_KEY)?.is_some();
    let csr_present = physical.read_csr_bytes()?.is_some();
    if nodes != 0 || edges != 0 || metadata_present || csr_present {
        return Err(contract_error(
            "CALYX_OPINION_ALIAS_COLLECTION_PHYSICAL_ROWS_EXIST",
            format!(
                "opinion-alias collection {collection} has orphaned physical rows: nodes={nodes}, edges={edges}, metadata={metadata_present}, csr={csr_present}"
            ),
            "quarantine the orphaned collection and use a new versioned collection name; never merge alias truth into unidentified Graph bytes",
        ));
    }
    Ok(())
}

fn write_report(path: Option<&Path>, report: &MaterializeReport) -> CliResult {
    let Some(path) = path else { return Ok(()) };
    let bytes = serde_json::to_vec_pretty(report)
        .map_err(|error| CliError::runtime(format!("serialize opinion-alias report: {error}")))?;
    write_bytes_atomic_new(path, &bytes, "opinion-alias materialization report")?;
    let actual = fs::read(path).map_err(|error| {
        CliError::io(format!(
            "read back opinion-alias report {}: {error}",
            path.display()
        ))
    })?;
    if actual != bytes {
        return Err(CliError::runtime(
            "opinion-alias report byte readback mismatch",
        ));
    }
    Ok(())
}

fn accept_generation(
    resolved: &ResolvedVault,
    collection: &str,
    generation: &str,
    report: &MaterializeReport,
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
    GraphCollectionLifecycle::new(&vault)?.put_state(
        &GraphCollectionGenerationState::new(
            collection.to_string(),
            generation.to_string(),
            GraphCollectionGenerationStatus::Accepted,
            "materialize-opinion-aliases",
        )
        .with_reason("physical Base and alias Graph readback passed")
        .with_detail("schema", SCHEMA)
        .with_detail(
            "source_opinion_aliases",
            report.accounting.source_opinion_aliases.to_string(),
        )
        .with_detail(
            "canonical_constellations",
            report.accounting.canonical_constellations.to_string(),
        )
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
    let accepted = lifecycle.list_states()?.iter().any(|row| {
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
            code: "CALYX_OPINION_ALIAS_LIFECYCLE_READBACK_MISSING",
            message: format!("accepted lifecycle row is absent for {collection}/{generation}"),
            remediation: "do not consume the alias collection until accepted state reads back",
        }));
    }
    let routed = PhysicalPlainGraph::open_latest_isolated(&resolved.path, collection)?;
    if routed.storage_collection() != report.graph_storage_collection {
        return Err(contract_error(
            "CALYX_OPINION_ALIAS_ACCEPTED_ROUTE_MISMATCH",
            format!(
                "accepted alias route resolved to {}, expected {} for {collection}/{generation}",
                routed.storage_collection(),
                report.graph_storage_collection
            ),
            "do not consume the alias collection until a cold reader resolves the verified generation",
        ));
    }
    let routed_csr = routed.read_csr_bytes()?.ok_or_else(|| {
        contract_error(
            "CALYX_OPINION_ALIAS_ACCEPTED_CSR_MISSING",
            format!("accepted alias route has no CSR bytes for {collection}/{generation}"),
            "do not consume the alias collection until its accepted CSR reads back",
        )
    })?;
    let routed_hash = sha256_hex(&routed_csr);
    if routed_hash != report.readback.csr_sha256 {
        return Err(contract_error(
            "CALYX_OPINION_ALIAS_ACCEPTED_CSR_MISMATCH",
            format!(
                "accepted alias CSR hash {routed_hash} differs from verified hash {} for {collection}/{generation}",
                report.readback.csr_sha256
            ),
            "quarantine the accepted generation and inspect physical Graph CF publication",
        ));
    }
    Ok(())
}
