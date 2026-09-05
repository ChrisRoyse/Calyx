//! Cold verification and publication of a computation-complete weave generation.
//!
//! This command exists for failures after all reconstructible Graph/XTerm data
//! was durably materialized but before the lifecycle acceptance barrier. It
//! never resumes computation and never trusts the failed command's output.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use calyx_aster::base_page_index::{read_base_page_index_manifest, visit_indexed_base_row_pages};
use calyx_aster::cf::{
    ColumnFamily, WEAVE_XTERM_GENERATION_SCHEMA, WEAVE_XTERM_STORAGE_SCHEMA_DETAIL,
    weave_xterm_generation_prefix_range,
};
use calyx_aster::plain_graph::{
    GRAPH_GENERATION_STORAGE_COLLECTION_DETAIL, GRAPH_GENERATION_STORAGE_SCHEMA,
    GRAPH_GENERATION_STORAGE_SCHEMA_DETAIL, GraphCollectionGenerationReadback,
    GraphCollectionGenerationState, GraphCollectionGenerationStatus,
    PhysicalGraphCollectionLifecycle, PhysicalPlainGraph, accept_verified_graph_generation,
    graph_generation_storage_collection,
};
use calyx_aster::vault::{AsterVault, VaultOptions, encode};
use calyx_core::{CalyxError, CxId, SlotId, SlotShape, SlotState};
use calyx_lodestar::{
    ASTER_ASSOC_METADATA_KEY, AsterAssocMetadata, CorpusWeaveReport, CorpusWeaveReportParams,
    PANEL_ASTER_ASSOC_COLLECTION, PANEL_RRF_K, corpus_weave_report,
};
use calyx_loom::agreement_graph::XtermRow;
use calyx_loom::{CrossTermKind, CrossTermValue, SignalProvenanceTag};
use calyx_registry::load_vault_stored_vector_panel_state;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::vault::{home_dir, resolve_vault_info, vault_salt};
use super::{Subcommand, value};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const FAILED_WEAVE_REASON: &str = "weave-loom exited before accepted generation physical readback completed; inspect WEAVE_LOOM_PROGRESS for the typed phase error";
const XTERM_READBACK_PAGE_ROWS: usize = 2_048;
const XTERM_GENERATION_PREFIX_BYTES: usize = 32;
const XTERM_CANONICAL_KEY_BYTES: usize = 21;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WeaveGenerationVerifyAcceptArgs {
    pub vault: String,
    pub generation: String,
    pub max_groundedness_distance: usize,
    pub home: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct BaseCorpusEvidence {
    manifest_live_entries: usize,
    visited_rows: usize,
    anchor_rows: usize,
    row_sha256: String,
    ordered_cx_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct GraphEvidence {
    storage_collection: String,
    csr_bytes: usize,
    csr_sha256: String,
    source_snapshot: u64,
    node_rows: usize,
    edge_rows: usize,
    association_edges: usize,
    ordered_cx_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct XtermEvidence {
    rows: usize,
    expected_rows: usize,
    compatible_slot_pairs: usize,
    key_bytes: u64,
    value_bytes: u64,
    sha256: String,
    rows_by_slot_pair: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct VerifiedArtifacts {
    graph: GraphEvidence,
    base: BaseCorpusEvidence,
    contract: AsterAssocMetadata,
    xterm: XtermEvidence,
    acceptance_report: CorpusWeaveReport,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct VerificationReadback {
    snapshot_seq: u64,
    artifact_sha256: String,
    artifacts: VerifiedArtifacts,
}

#[derive(Debug, Serialize)]
struct WeaveGenerationVerifyAcceptReport {
    status: &'static str,
    source_of_truth: &'static str,
    vault: String,
    vault_id: String,
    vault_dir: String,
    collection: &'static str,
    generation: String,
    lifecycle_before: GraphCollectionGenerationReadback,
    acceptance_write_seq: u64,
    verification_before: VerificationReadback,
    lifecycle_after_cold_reopen: GraphCollectionGenerationReadback,
    verification_after_cold_reopen: VerificationReadback,
    accepted_route_storage_collection: String,
}

pub(crate) fn parse_weave_generation_verify_accept(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("weave-generation-verify-accept requires <vault>"))?
        .clone();
    let mut generation = None;
    let mut max_groundedness_distance = None;
    let mut home = None;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--generation" => {
                idx += 1;
                generation = Some(value(rest, idx, "--generation")?.to_string());
            }
            "--max-groundedness-distance" => {
                idx += 1;
                let parsed = value(rest, idx, "--max-groundedness-distance")?
                    .parse::<usize>()
                    .map_err(|_| {
                        CliError::usage("--max-groundedness-distance requires an integer")
                    })?;
                if parsed == 0 {
                    return Err(CliError::usage(
                        "--max-groundedness-distance must be greater than zero",
                    ));
                }
                max_groundedness_distance = Some(parsed);
            }
            "--home" => {
                idx += 1;
                home = Some(value(rest, idx, "--home")?.into());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected weave-generation-verify-accept flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(Subcommand::WeaveGenerationVerifyAccept(
        WeaveGenerationVerifyAcceptArgs {
            vault,
            generation: generation.ok_or_else(|| {
                CliError::usage("weave-generation-verify-accept requires --generation <id>")
            })?,
            max_groundedness_distance: max_groundedness_distance.ok_or_else(|| {
                CliError::usage(
                    "weave-generation-verify-accept requires --max-groundedness-distance <n> so the recovery acceptance gate is explicit",
                )
            })?,
            home,
        },
    ))
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::WeaveGenerationVerifyAccept(args) = command else {
        unreachable!("non weave-generation-verify-accept command routed here")
    };
    verify_accept(args)
}

fn verify_accept(args: WeaveGenerationVerifyAcceptArgs) -> CliResult {
    let home = args
        .home
        .as_deref()
        .map_or_else(home_dir, |path| Ok(path.to_path_buf()))?;
    let resolved = resolve_vault_info(&home, &args.vault)?;
    let lifecycle_before = read_generation(&resolved.path, &args.generation)?;
    validate_failed_lifecycle(&lifecycle_before, &args.generation)?;

    let vault = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::write_latest_readback(),
    )?;
    let verification_before = verify_artifacts(
        &vault,
        &resolved.path,
        &args.generation,
        args.max_groundedness_distance,
    )?;
    let artifacts = &verification_before.artifacts;
    let accepted = accepted_state(
        &args.generation,
        verification_before.snapshot_seq,
        &verification_before.artifact_sha256,
        artifacts,
        args.max_groundedness_distance,
    );
    let acceptance_write_seq = accept_verified_graph_generation(
        &vault,
        verification_before.snapshot_seq,
        &lifecycle_before.state,
        &accepted,
    )?;
    drop(vault);

    let lifecycle_after_cold_reopen = read_generation(&resolved.path, &args.generation)?;
    if lifecycle_after_cold_reopen.state != accepted {
        return Err(verification_error(format!(
            "cold lifecycle readback differs from the accepted state for {}/{}",
            PANEL_ASTER_ASSOC_COLLECTION, args.generation
        )));
    }
    let cold = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::write_latest_readback(),
    )?;
    let verification_after_cold_reopen = verify_artifacts(
        &cold,
        &resolved.path,
        &args.generation,
        args.max_groundedness_distance,
    )?;
    drop(cold);
    if verification_after_cold_reopen.artifacts != verification_before.artifacts {
        return Err(verification_error(format!(
            "cold artifact verification changed after acceptance for {}/{}",
            PANEL_ASTER_ASSOC_COLLECTION, args.generation
        )));
    }

    let accepted_route =
        PhysicalPlainGraph::open_latest_isolated(&resolved.path, PANEL_ASTER_ASSOC_COLLECTION)?;
    let accepted_route_storage_collection = accepted_route.storage_collection();
    if accepted_route_storage_collection != artifacts.graph.storage_collection {
        return Err(verification_error(format!(
            "accepted route resolved to {accepted_route_storage_collection}, expected {}",
            artifacts.graph.storage_collection
        )));
    }

    print_json(&WeaveGenerationVerifyAcceptReport {
        status: "accepted",
        source_of_truth: "cold Base page-index rows + exact isolated Graph CSR/row counts + complete generation XTerm scan + atomically guarded lifecycle publication + second cold reopen",
        vault: resolved.name,
        vault_id: resolved.vault_id.to_string(),
        vault_dir: resolved.path.display().to_string(),
        collection: PANEL_ASTER_ASSOC_COLLECTION,
        generation: args.generation,
        lifecycle_before,
        acceptance_write_seq,
        verification_before,
        lifecycle_after_cold_reopen,
        verification_after_cold_reopen,
        accepted_route_storage_collection,
    })
}

fn validate_failed_lifecycle(
    readback: &GraphCollectionGenerationReadback,
    generation: &str,
) -> CliResult {
    let state = &readback.state;
    let expected_storage =
        graph_generation_storage_collection(PANEL_ASTER_ASSOC_COLLECTION, generation)?;
    if state.collection != PANEL_ASTER_ASSOC_COLLECTION
        || state.generation != generation
        || state.status != GraphCollectionGenerationStatus::Failed
        || state.command != "weave-loom"
        || state.reason.as_deref() != Some(FAILED_WEAVE_REASON)
        || state
            .detail
            .get(GRAPH_GENERATION_STORAGE_SCHEMA_DETAIL)
            .map(String::as_str)
            != Some(GRAPH_GENERATION_STORAGE_SCHEMA)
        || state.detail.get(GRAPH_GENERATION_STORAGE_COLLECTION_DETAIL) != Some(&expected_storage)
        || state
            .detail
            .get(WEAVE_XTERM_STORAGE_SCHEMA_DETAIL)
            .map(String::as_str)
            != Some(WEAVE_XTERM_GENERATION_SCHEMA)
    {
        return Err(recovery_refused(format!(
            "{}/{} is not an exact failed isolated weave generation eligible for verified publication",
            PANEL_ASTER_ASSOC_COLLECTION, generation
        )));
    }
    Ok(())
}

fn verify_artifacts(
    vault: &AsterVault,
    vault_dir: &std::path::Path,
    generation: &str,
    max_groundedness_distance: usize,
) -> CliResult<VerificationReadback> {
    let snapshot_seq = vault.latest_seq();
    let storage = graph_generation_storage_collection(PANEL_ASTER_ASSOC_COLLECTION, generation)?;
    let physical = PhysicalPlainGraph::open_generation_unchecked(
        vault_dir,
        PANEL_ASTER_ASSOC_COLLECTION,
        generation,
    )?;

    let csr_bytes = physical
        .read_csr_bytes()?
        .ok_or_else(|| verification_error("isolated generation has no persisted CSR bytes"))?;
    let csr_len = csr_bytes.len();
    let csr_sha256 = sha256_hex(&csr_bytes);
    drop(csr_bytes);
    let csr = physical
        .read_csr()?
        .ok_or_else(|| verification_error("isolated generation has no decodable CSR"))?;
    if csr.collection != storage {
        return Err(verification_error(format!(
            "CSR collection {} does not match isolated storage {storage}",
            csr.collection
        )));
    }
    if csr.source_snapshot > snapshot_seq {
        return Err(verification_error(format!(
            "CSR source snapshot {} is newer than verification snapshot {snapshot_seq}",
            csr.source_snapshot
        )));
    }
    let csr_node_set = csr.nodes.iter().copied().collect::<BTreeSet<_>>();
    if csr_node_set.len() != csr.nodes.len() {
        return Err(verification_error("CSR contains duplicate node ids"));
    }
    validate_csr_edge_contract(&csr, &csr_node_set)?;
    let assoc = physical.assoc_graph()?;
    let physical_nodes = physical.node_key_count()?;
    let physical_edges = physical.edge_out_key_count()?;
    if physical_nodes != csr.nodes.len()
        || physical_edges != csr.edges.len()
        || assoc.node_count() != csr.nodes.len()
        || assoc.edge_count() != csr.edges.len()
        || csr.association_edge_count != csr.edges.len()
    {
        return Err(verification_error(format!(
            "Graph source-of-truth mismatch: CSR nodes={} edges={} assoc_edges={}, decoded graph nodes={} edges={}, physical rows nodes={physical_nodes} edges={physical_edges}",
            csr.nodes.len(),
            csr.edges.len(),
            csr.association_edge_count,
            assoc.node_count(),
            assoc.edge_count()
        )));
    }

    let metadata_raw = physical
        .get_metadata(ASTER_ASSOC_METADATA_KEY)?
        .ok_or_else(|| verification_error("isolated generation has no graph contract metadata"))?;
    let contract =
        serde_json::from_slice::<AsterAssocMetadata>(&metadata_raw).map_err(|error| {
            verification_error(format!("graph contract metadata is not decodable: {error}"))
        })?;
    let panel = load_vault_stored_vector_panel_state(vault_dir)?.panel;
    let expected_pairs = validate_contract(
        &contract,
        &panel,
        snapshot_seq,
        csr.nodes.len(),
        csr.edges.len(),
    )?;

    let (base, base_nodes, anchors) = read_base_evidence(vault_dir)?;
    if base_nodes != csr_node_set {
        let missing_from_graph = base_nodes
            .difference(&csr_node_set)
            .take(3)
            .collect::<Vec<_>>();
        let missing_from_base = csr_node_set
            .difference(&base_nodes)
            .take(3)
            .collect::<Vec<_>>();
        return Err(verification_error(format!(
            "CSR node set differs from physical Base corpus: base_only={missing_from_graph:?} graph_only={missing_from_base:?}"
        )));
    }
    let graph_ordered_cx_sha256 = digest_ids(csr.nodes.iter().copied());
    if graph_ordered_cx_sha256 != base.ordered_cx_sha256 {
        return Err(verification_error(
            "CSR and physical Base contain the same ids but not the same canonical order",
        ));
    }

    let xterm = read_xterm_evidence(
        vault,
        snapshot_seq,
        generation,
        &csr_node_set,
        &expected_pairs,
    )?;
    let acceptance_report = corpus_weave_report(
        &assoc,
        &anchors,
        &CorpusWeaveReportParams {
            max_groundedness_distance,
            ..CorpusWeaveReportParams::default()
        },
    )?;
    if !acceptance_report.gate_passed {
        return Err(verification_error(format!(
            "corpus weave acceptance gate failed: groundedness_fraction={} minimum={}",
            acceptance_report.groundedness_fraction, acceptance_report.min_groundedness_fraction
        )));
    }
    if vault.latest_seq() != snapshot_seq {
        return Err(CliError::from(CalyxError::stale_derived(format!(
            "vault advanced from sequence {snapshot_seq} during weave generation verification"
        ))));
    }

    let artifacts = VerifiedArtifacts {
        graph: GraphEvidence {
            storage_collection: storage,
            csr_bytes: csr_len,
            csr_sha256,
            source_snapshot: csr.source_snapshot,
            node_rows: physical_nodes,
            edge_rows: physical_edges,
            association_edges: assoc.edge_count(),
            ordered_cx_sha256: graph_ordered_cx_sha256,
        },
        base,
        contract,
        xterm,
        acceptance_report,
    };
    let encoded = serde_json::to_vec(&artifacts).map_err(|error| {
        verification_error(format!("serialize verified artifact evidence: {error}"))
    })?;
    Ok(VerificationReadback {
        snapshot_seq,
        artifact_sha256: sha256_hex(&encoded),
        artifacts,
    })
}

fn validate_csr_edge_contract(
    csr: &calyx_aster::plain_graph::PlainGraphCsr,
    node_set: &BTreeSet<CxId>,
) -> CliResult {
    if csr.offsets.len() != csr.nodes.len() + 1
        || csr.offsets.first().copied() != Some(0)
        || csr.offsets.last().copied() != Some(csr.edges.len())
        || csr
            .offsets
            .windows(2)
            .any(|window| window[0] > window[1] || window[1] > csr.edges.len())
    {
        return Err(verification_error(
            "CSR offsets do not form a bounded monotonic partition of the edge rows",
        ));
    }
    for (source_index, offsets) in csr.offsets.windows(2).enumerate() {
        let source = csr.nodes.get(source_index).ok_or_else(|| {
            verification_error("CSR offset window has no corresponding source node")
        })?;
        let mut destinations = BTreeSet::new();
        for edge in &csr.edges[offsets[0]..offsets[1]] {
            if edge.edge_type != "knn"
                || edge.dst == *source
                || !node_set.contains(&edge.dst)
                || !destinations.insert(edge.dst)
            {
                return Err(verification_error(format!(
                    "CSR source {source} has an invalid, self, missing, or duplicate k-NN edge to {}",
                    edge.dst
                )));
            }
        }
    }
    Ok(())
}

fn validate_contract(
    contract: &AsterAssocMetadata,
    panel: &calyx_core::Panel,
    snapshot_seq: u64,
    node_count: usize,
    edge_count: usize,
) -> CliResult<BTreeMap<(u16, u16), u32>> {
    let mut content_slots = panel
        .slots
        .iter()
        .filter(|slot| slot.state == SlotState::Active && !slot.retrieval_only)
        .map(|slot| slot.slot_id)
        .collect::<Vec<_>>();
    content_slots.sort();
    content_slots.dedup();
    let threshold = contract.edge_score_threshold.unwrap_or(f32::NAN);
    let source_seq = contract.graph_source_seq.unwrap_or(u64::MAX);
    let knn = contract.knn.unwrap_or(0);
    if contract.embedding_slot.is_some()
        || contract.embedding_slots != content_slots
        || contract.fusion.as_deref() != Some("typed_block_rrf")
        || contract.rrf_k != Some(PANEL_RRF_K)
        || contract.panel_version != Some(u64::from(panel.version))
        || source_seq > snapshot_seq
        || knn == 0
        || !threshold.is_finite()
        || !(0.0..=1.0).contains(&threshold)
    {
        return Err(verification_error(
            "graph contract does not match the current frozen panel-native weave contract",
        ));
    }
    let maximum_edges = node_count
        .checked_mul(knn)
        .filter(|maximum_edges| *maximum_edges > 0)
        .ok_or_else(|| verification_error("graph contract node_count * knn overflow or zero"))?;
    if edge_count == 0 || edge_count > maximum_edges {
        return Err(verification_error(format!(
            "graph edge count {edge_count} is outside the sealed k-NN bound 1..={maximum_edges}"
        )));
    }

    let slots_by_id = panel
        .slots
        .iter()
        .map(|slot| (slot.slot_id, slot))
        .collect::<BTreeMap<_, _>>();
    let mut groups = BTreeMap::<u32, Vec<SlotId>>::new();
    for slot_id in &contract.embedding_slots {
        let slot = slots_by_id.get(slot_id).ok_or_else(|| {
            verification_error(format!(
                "graph contract slot {slot_id} is absent from the panel"
            ))
        })?;
        let ambient_dim = match slot.shape {
            SlotShape::Dense(dim) | SlotShape::Sparse(dim) => dim,
            SlotShape::Multi { token_dim } => token_dim,
        };
        if ambient_dim == 0 {
            return Err(verification_error(format!(
                "graph contract slot {slot_id} has zero ambient dimension"
            )));
        }
        groups.entry(ambient_dim).or_default().push(*slot_id);
    }
    let mut pairs = BTreeMap::new();
    for (ambient_dim, slots) in groups {
        for left in 0..slots.len() {
            for right in left + 1..slots.len() {
                pairs.insert((slots[left].get(), slots[right].get()), ambient_dim);
            }
        }
    }
    if pairs.is_empty() {
        return Err(verification_error(
            "graph contract has no dimension-compatible XTerm slot pair",
        ));
    }
    Ok(pairs)
}

fn read_base_evidence(
    vault_dir: &std::path::Path,
) -> CliResult<(BaseCorpusEvidence, BTreeSet<CxId>, Vec<CxId>)> {
    let manifest = read_base_page_index_manifest(vault_dir)?;
    let mut nodes = BTreeSet::new();
    let mut anchors = Vec::new();
    let mut row_hasher = Sha256::new();
    let visited_rows = visit_indexed_base_row_pages(vault_dir, |_, rows| -> CliResult<bool> {
        for (key, value) in rows {
            let key_bytes: [u8; 16] = key.as_slice().try_into().map_err(|_| {
                verification_error(format!("Base key has {} bytes, expected 16", key.len()))
            })?;
            let id = CxId::from_bytes(key_bytes);
            let constellation = encode::decode_constellation_base(&value)?;
            if constellation.cx_id != id || !nodes.insert(id) {
                return Err(verification_error(format!(
                    "Base page index has a mismatched or duplicate constellation {id}"
                )));
            }
            if !constellation.anchors.is_empty() {
                anchors.push(id);
            }
            row_hasher.update((key.len() as u64).to_be_bytes());
            row_hasher.update(&key);
            row_hasher.update((value.len() as u64).to_be_bytes());
            row_hasher.update(&value);
        }
        Ok(true)
    })?;
    if visited_rows != manifest.live_entries || nodes.len() != manifest.live_entries {
        return Err(verification_error(format!(
            "Base page-index readback visited {visited_rows} unique={} but manifest declares {}",
            nodes.len(),
            manifest.live_entries
        )));
    }
    let evidence = BaseCorpusEvidence {
        manifest_live_entries: manifest.live_entries,
        visited_rows,
        anchor_rows: anchors.len(),
        row_sha256: format!("{:x}", row_hasher.finalize()),
        ordered_cx_sha256: digest_ids(nodes.iter().copied()),
    };
    Ok((evidence, nodes, anchors))
}

fn read_xterm_evidence(
    vault: &AsterVault,
    snapshot_seq: u64,
    generation: &str,
    node_set: &BTreeSet<CxId>,
    expected_pairs: &BTreeMap<(u16, u16), u32>,
) -> CliResult<XtermEvidence> {
    let expected_rows = node_set
        .len()
        .checked_mul(expected_pairs.len())
        .ok_or_else(|| verification_error("expected XTerm row count overflow"))?;
    let range = weave_xterm_generation_prefix_range(PANEL_ASTER_ASSOC_COLLECTION, generation)?;
    let mut rows = 0usize;
    let mut key_bytes = 0u64;
    let mut value_bytes = 0u64;
    let mut hasher = Sha256::new();
    let mut pair_counts = expected_pairs
        .keys()
        .map(|pair| (*pair, 0usize))
        .collect::<BTreeMap<_, _>>();
    vault.scan_cf_range_pages_at_renewing_latest(
        snapshot_seq,
        ColumnFamily::XTerm,
        &range,
        XTERM_READBACK_PAGE_ROWS,
        |page| -> CliResult {
            for (key, value) in page {
                let expected_key_len = XTERM_GENERATION_PREFIX_BYTES + XTERM_CANONICAL_KEY_BYTES;
                if key.len() != expected_key_len {
                    return Err(verification_error(format!(
                        "generation XTerm key has {} bytes, expected {expected_key_len}",
                        key.len()
                    )));
                }
                let id = CxId::from_bytes(
                    key[XTERM_GENERATION_PREFIX_BYTES..XTERM_GENERATION_PREFIX_BYTES + 16]
                        .try_into()
                        .expect("validated XTerm key length"),
                );
                let a = u16::from_be_bytes(
                    key[XTERM_GENERATION_PREFIX_BYTES + 16..XTERM_GENERATION_PREFIX_BYTES + 18]
                        .try_into()
                        .expect("validated XTerm key length"),
                );
                let b = u16::from_be_bytes(
                    key[XTERM_GENERATION_PREFIX_BYTES + 18..XTERM_GENERATION_PREFIX_BYTES + 20]
                        .try_into()
                        .expect("validated XTerm key length"),
                );
                let kind = key[XTERM_GENERATION_PREFIX_BYTES + 20];
                let ambient_dim = expected_pairs.get(&(a, b)).ok_or_else(|| {
                    verification_error(format!(
                        "XTerm row {id}/{a}/{b} is outside the frozen compatible-pair contract"
                    ))
                })?;
                if !node_set.contains(&id) || kind != 2 {
                    return Err(verification_error(format!(
                        "XTerm row {id}/{a}/{b} has an unknown node or non-agreement kind {kind}"
                    )));
                }
                let row = serde_json::from_slice::<XtermRow>(&value).map_err(|error| {
                    verification_error(format!("decode XTerm row {id}/{a}/{b}: {error}"))
                })?;
                if row.key.cx_id != id
                    || row.key.a.get() != a
                    || row.key.b.get() != b
                    || row.key.kind != CrossTermKind::Agreement
                    || row.tag != SignalProvenanceTag::Derived
                {
                    return Err(verification_error(format!(
                        "XTerm key/value contract mismatch for {id}/{a}/{b}"
                    )));
                }
                match row.value {
                    CrossTermValue::BlockAgreement(evidence) => {
                        evidence.validate()?;
                        if evidence.ambient_dim != *ambient_dim {
                            return Err(verification_error(format!(
                                "XTerm {id}/{a}/{b} ambient dimension {} differs from panel {}",
                                evidence.ambient_dim, ambient_dim
                            )));
                        }
                    }
                    _ => {
                        return Err(verification_error(format!(
                            "typed weave XTerm {id}/{a}/{b} is not block-agreement evidence"
                        )));
                    }
                }
                *pair_counts
                    .get_mut(&(a, b))
                    .expect("validated expected pair") += 1;
                rows = rows
                    .checked_add(1)
                    .ok_or_else(|| verification_error("XTerm row count overflow"))?;
                key_bytes = key_bytes
                    .checked_add(key.len() as u64)
                    .ok_or_else(|| verification_error("XTerm key-byte count overflow"))?;
                value_bytes = value_bytes
                    .checked_add(value.len() as u64)
                    .ok_or_else(|| verification_error("XTerm value-byte count overflow"))?;
                hasher.update((key.len() as u64).to_be_bytes());
                hasher.update(&key);
                hasher.update((value.len() as u64).to_be_bytes());
                hasher.update(&value);
            }
            Ok(())
        },
    )?;
    if rows != expected_rows
        || pair_counts
            .values()
            .any(|pair_rows| *pair_rows != node_set.len())
    {
        return Err(verification_error(format!(
            "XTerm completeness mismatch: rows={rows} expected={expected_rows}, per-pair counts={pair_counts:?}"
        )));
    }
    Ok(XtermEvidence {
        rows,
        expected_rows,
        compatible_slot_pairs: expected_pairs.len(),
        key_bytes,
        value_bytes,
        sha256: format!("{:x}", hasher.finalize()),
        rows_by_slot_pair: pair_counts
            .into_iter()
            .map(|((a, b), rows)| (format!("{a}:{b}"), rows))
            .collect(),
    })
}

fn accepted_state(
    generation: &str,
    verification_snapshot: u64,
    artifact_sha256: &str,
    artifacts: &VerifiedArtifacts,
    max_groundedness_distance: usize,
) -> GraphCollectionGenerationState {
    GraphCollectionGenerationState::new(
        PANEL_ASTER_ASSOC_COLLECTION,
        generation,
        GraphCollectionGenerationStatus::Accepted,
        "weave-generation-verify-accept",
    )
    .with_reason(
        "independent cold Graph/CSR/Base/XTerm verification and weave acceptance report passed",
    )
    .with_detail(
        GRAPH_GENERATION_STORAGE_SCHEMA_DETAIL,
        GRAPH_GENERATION_STORAGE_SCHEMA,
    )
    .with_detail(
        GRAPH_GENERATION_STORAGE_COLLECTION_DETAIL,
        artifacts.graph.storage_collection.clone(),
    )
    .with_detail(
        WEAVE_XTERM_STORAGE_SCHEMA_DETAIL,
        WEAVE_XTERM_GENERATION_SCHEMA,
    )
    .with_detail("node_rows", artifacts.graph.node_rows.to_string())
    .with_detail("edge_rows", artifacts.graph.edge_rows.to_string())
    .with_detail("xterm_rows", artifacts.xterm.rows.to_string())
    .with_detail("xterm_sha256", artifacts.xterm.sha256.clone())
    .with_detail("csr_sha256", artifacts.graph.csr_sha256.clone())
    .with_detail("base_rows_sha256", artifacts.base.row_sha256.clone())
    .with_detail("artifact_sha256", artifact_sha256)
    .with_detail("verification_snapshot", verification_snapshot.to_string())
    .with_detail(
        "max_groundedness_distance",
        max_groundedness_distance.to_string(),
    )
}

fn read_generation(
    vault_dir: &std::path::Path,
    generation: &str,
) -> CliResult<GraphCollectionGenerationReadback> {
    PhysicalGraphCollectionLifecycle::open_latest(vault_dir)?
        .list_states()?
        .into_iter()
        .find(|row| {
            row.state.collection == PANEL_ASTER_ASSOC_COLLECTION
                && row.state.generation == generation
        })
        .ok_or_else(|| {
            recovery_refused(format!(
                "lifecycle row does not exist for {}/{}",
                PANEL_ASTER_ASSOC_COLLECTION, generation
            ))
        })
}

fn digest_ids(ids: impl IntoIterator<Item = CxId>) -> String {
    let mut hasher = Sha256::new();
    for id in ids {
        hasher.update(id.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn recovery_refused(message: impl Into<String>) -> CliError {
    CliError::from(CalyxError {
        code: "CALYX_WEAVE_GENERATION_RECOVERY_REFUSED",
        message: message.into(),
        remediation: "select the exact failed isolated weave generation; never publish writing, accepted, tombstoned, foreign, or incomplete state",
    })
}

fn verification_error(message: impl Into<String>) -> CliError {
    CliError::from(CalyxError {
        code: "CALYX_WEAVE_GENERATION_VERIFY_FAILED",
        message: message.into(),
        remediation: "preserve the isolated generation, inspect the named physical mismatch, and recompute only if its Graph/CSR/Base/XTerm contract cannot be proven",
    })
}
