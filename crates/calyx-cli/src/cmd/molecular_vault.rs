//! `calyx materialize-molecular-vault` writes real modality rows into a vault.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Anchor, AnchorKind, AnchorValue, CxId, Input, VaultStore};
use calyx_lodestar::{
    AssocStore, AsterAssocNodeProps, DEFAULT_ASTER_ASSOC_COLLECTION, PhysicalAsterAssocSnapshot,
    encode_assoc_node_props,
};
use calyx_registry::{load_vault_panel_state, measure::measure_constellation};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::ingest::ensure_content_panel_floor;
use super::vault::{home_dir, resolve_vault_info, vault_salt};
use super::{Subcommand, value};
use crate::error::{CliError, CliResult};
use crate::output::print_json;
use rows::{PreparedRow, modality_name, read_rows, validate_row_set};

const GRAPH_ANCHOR: &str = "molecular-vault";

mod rows;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MaterializeMolecularVaultArgs {
    pub vault: String,
    pub rows: PathBuf,
    pub home: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
struct MaterializeMolecularVaultReport {
    status: &'static str,
    vault: String,
    vault_id: String,
    vault_dir: String,
    panel_version: u32,
    rows_jsonl: String,
    rows_jsonl_bytes: u64,
    rows_jsonl_sha256: String,
    row_count: usize,
    modality_counts: BTreeMap<String, usize>,
    domain_counts: BTreeMap<String, usize>,
    bridge_term_count: usize,
    affinity_row_count: usize,
    cx_ids: Vec<String>,
    readback: MolecularVaultReadback,
}

#[derive(Debug, Serialize)]
struct MolecularVaultReadback {
    base_rows: usize,
    anchor_rows: usize,
    measured_slot_rows: BTreeMap<String, usize>,
    graph_nodes: usize,
    graph_edges: usize,
    all_rows_read_back: bool,
}

pub(crate) fn parse_materialize_molecular_vault(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("materialize-molecular-vault requires <vault>"))?
        .clone();
    let mut rows = None;
    let mut home = None;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--rows" => {
                idx += 1;
                rows = Some(value(rest, idx, "--rows")?.into());
            }
            "--home" => {
                idx += 1;
                home = Some(value(rest, idx, "--home")?.into());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected materialize-molecular-vault flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(Subcommand::MaterializeMolecularVault(
        MaterializeMolecularVaultArgs {
            vault,
            rows: rows.ok_or_else(|| {
                CliError::usage("materialize-molecular-vault requires --rows <jsonl>")
            })?,
            home,
        },
    ))
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::MaterializeMolecularVault(args) = command else {
        unreachable!("non-materialize-molecular-vault command routed here");
    };
    let home = args.home.clone().map_or_else(home_dir, Ok)?;
    let report = materialize(&home, args)?;
    print_json(&report)
}

fn materialize(
    home: &Path,
    args: MaterializeMolecularVaultArgs,
) -> CliResult<MaterializeMolecularVaultReport> {
    let rows_bytes = fs::read(&args.rows)?;
    let rows_sha256 = sha256_hex(&rows_bytes);
    let rows = read_rows(&args.rows)?;
    validate_row_set(&rows)?;
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
    let state = load_vault_panel_state(&resolved.path)?;
    let mut expected = Vec::with_capacity(rows.len());
    let mut cx_ids = BTreeSet::new();
    let now = crate::cmd::vault::now_ms();
    for row in &rows {
        let mut cx = measure_constellation(
            &vault,
            &state,
            Input::new(row.modality, row.input.clone().into_bytes()),
            now,
        )?;
        if !cx_ids.insert(cx.cx_id) {
            return Err(CliError::usage(format!(
                "molecular vault rows produce duplicate cx_id {}",
                cx.cx_id
            )));
        }
        if vault
            .read_cf_at(vault.snapshot(), ColumnFamily::Base, &base_key(cx.cx_id))?
            .is_some()
        {
            return Err(CliError::usage(format!(
                "molecular vault row {} already exists as cx_id {}",
                row.id, cx.cx_id
            )));
        }
        cx.metadata = row_metadata(row);
        cx.anchors = row_anchors(row, now)?;
        cx.flags.ungrounded = false;
        ensure_content_panel_floor(&cx, &state)?;
        expected.push(cx);
    }
    vault.put_batch(expected.clone())?;
    let graph = PlainGraph::new(&vault, DEFAULT_ASTER_ASSOC_COLLECTION)?;
    let mut term_to_rows: BTreeMap<String, Vec<CxId>> = BTreeMap::new();
    for (row, cx) in rows.iter().zip(&expected) {
        graph.put_node(cx.cx_id, &encode_assoc_node_props(&node_props(row))?)?;
        for term in &row.bridge_terms {
            term_to_rows.entry(term.clone()).or_default().push(cx.cx_id);
        }
    }
    let mut graph_edges = 0usize;
    for (term, ids) in &term_to_rows {
        let term_id = CxId::from_input(
            format!("molecular-vault-term:{term}").as_bytes(),
            state.panel.version,
            resolved.vault_id.to_string().as_bytes(),
        );
        let props = AsterAssocNodeProps {
            anchors: vec![AnchorKind::Label(GRAPH_ANCHOR.to_string())],
            metadata: BTreeMap::from([
                ("domain".to_string(), "bridge_term".to_string()),
                ("term".to_string(), term.clone()),
                (
                    "source_dataset".to_string(),
                    "molecular_vault_terms".to_string(),
                ),
                ("source_id".to_string(), term.clone()),
                ("row_count".to_string(), ids.len().to_string()),
            ]),
            ..Default::default()
        };
        graph.put_node(term_id, &encode_assoc_node_props(&props)?)?;
        for id in ids {
            graph.put_edge(*id, "bridge_term", term_id, b"1")?;
            graph.put_edge(term_id, "bridge_term", *id, b"1")?;
            graph_edges += 2;
        }
    }
    graph.rebuild_csr(vault.snapshot())?;
    vault.flush()?;
    let readback = readback(&vault, &resolved.path, &state.panel.slots, &expected)?;
    if readback.graph_edges != graph_edges {
        return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
            "molecular vault graph edge readback mismatch: wrote {graph_edges}, read {}",
            readback.graph_edges
        ))
        .into());
    }
    let mut modality_counts = BTreeMap::new();
    let mut domain_counts = BTreeMap::new();
    for row in &rows {
        *modality_counts
            .entry(modality_name(row.modality).to_string())
            .or_insert(0) += 1;
        *domain_counts.entry(row.domain.clone()).or_insert(0) += 1;
    }
    Ok(MaterializeMolecularVaultReport {
        status: "ok",
        vault: resolved.name,
        vault_id: resolved.vault_id.to_string(),
        vault_dir: resolved.path.display().to_string(),
        panel_version: state.panel.version,
        rows_jsonl: args.rows.display().to_string(),
        rows_jsonl_bytes: rows_bytes.len() as u64,
        rows_jsonl_sha256: rows_sha256,
        row_count: rows.len(),
        modality_counts,
        domain_counts,
        bridge_term_count: term_to_rows.len(),
        affinity_row_count: rows
            .iter()
            .filter(|row| row.binding_affinity_nm.is_some())
            .count(),
        cx_ids: expected.iter().map(|cx| cx.cx_id.to_string()).collect(),
        readback,
    })
}

fn row_metadata(row: &PreparedRow) -> BTreeMap<String, String> {
    let mut metadata = row.metadata.clone();
    metadata.insert("molecular_row_id".to_string(), row.id.clone());
    metadata.insert("domain".to_string(), row.domain.clone());
    metadata.insert(
        "modality".to_string(),
        modality_name(row.modality).to_string(),
    );
    metadata.insert("text".to_string(), row.text.clone());
    if let Some(value) = row.binding_affinity_nm {
        metadata.insert("binding_affinity_nm".to_string(), value.to_string());
    }
    metadata
}

fn row_anchors(row: &PreparedRow, now: u64) -> CliResult<Vec<Anchor>> {
    let source = row
        .metadata
        .get("source_dataset")
        .cloned()
        .unwrap_or_else(|| "molecular-vault".to_string());
    let mut anchors = vec![
        label_anchor(
            "domain",
            AnchorValue::Enum(row.domain.clone()),
            &source,
            now,
        ),
        label_anchor(
            "source_dataset",
            AnchorValue::Enum(source.clone()),
            &source,
            now,
        ),
    ];
    for term in &row.bridge_terms {
        anchors.push(label_anchor(
            &format!("bridge_term:{term}"),
            AnchorValue::Bool(true),
            &source,
            now,
        ));
    }
    if let Some(value) = row.binding_affinity_nm {
        anchors.push(label_anchor(
            "binding_affinity_nm",
            AnchorValue::Number(value),
            &source,
            now,
        ));
    }
    for anchor in &anchors {
        anchor.validate_schema()?;
    }
    Ok(anchors)
}

fn label_anchor(kind: &str, value: AnchorValue, source: &str, now: u64) -> Anchor {
    Anchor {
        kind: AnchorKind::Label(kind.to_string()),
        value,
        source: source.to_string(),
        observed_at: now,
        confidence: 1.0,
    }
}

fn node_props(row: &PreparedRow) -> AsterAssocNodeProps {
    AsterAssocNodeProps {
        anchors: vec![AnchorKind::Label(GRAPH_ANCHOR.to_string())],
        metadata: row_metadata(row),
        ..Default::default()
    }
}

fn readback(
    vault: &AsterVault,
    vault_dir: &Path,
    slots: &[calyx_core::Slot],
    expected: &[calyx_core::Constellation],
) -> CliResult<MolecularVaultReadback> {
    let snapshot = vault.snapshot();
    for cx in expected {
        let stored = vault.get(cx.cx_id, snapshot)?;
        if stored.metadata != cx.metadata || stored.anchors != cx.anchors {
            return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
                "molecular vault readback mismatch for cx {}",
                cx.cx_id
            ))
            .into());
        }
    }
    let mut measured_slot_rows = BTreeMap::new();
    for slot in slots {
        let mut present = 0usize;
        for cx in expected {
            let Some(vector) = vault.read_slot_vector_at(snapshot, cx.cx_id, slot.slot_id)? else {
                continue;
            };
            if !vector.is_absent() {
                present += 1;
            }
        }
        if present > 0 {
            measured_slot_rows.insert(slot.slot_key.key().to_string(), present);
        }
    }
    let graph = PhysicalAsterAssocSnapshot::latest(vault_dir, DEFAULT_ASTER_ASSOC_COLLECTION)?
        .full_graph()?;
    Ok(MolecularVaultReadback {
        base_rows: vault.scan_cf_at(snapshot, ColumnFamily::Base)?.len(),
        anchor_rows: vault.scan_cf_at(snapshot, ColumnFamily::Anchors)?.len(),
        measured_slot_rows,
        graph_nodes: graph.node_count(),
        graph_edges: graph.edge_count(),
        all_rows_read_back: true,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
