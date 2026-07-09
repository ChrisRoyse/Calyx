//! Poly integration for Loom weave cross-terms and agreement graph exposure.
//!
//! The source of truth is the durable Aster `xterm` column family. This module
//! reads stored constellations from the vault, runs the real `calyx-loom`
//! engine over their dense slots, persists the exact Loom XTerm encoding, then
//! reads the XTerm rows back before writing the agreement-graph report.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, XTermKind, xterm_key, xterm_prefix_range};
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, Constellation, CxId, Seq, SlotId, SlotVector, VaultStore};
use calyx_loom::agreement_graph::XtermRow;
use calyx_loom::{AgreementEdge, CrossTermKind, LoomStore};
use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};

pub const LOOM_WEAVE_SCHEMA_VERSION: &str = "poly.loom_weave.v1";
pub const LOOM_WEAVE_ARTIFACT_KIND: &str = "poly_loom_weave";
pub const ERR_LOOM_WEAVE_EMPTY_INPUT: &str = "CALYX_POLY_LOOM_WEAVE_EMPTY_INPUT";
pub const ERR_LOOM_WEAVE_DUPLICATE_CX: &str = "CALYX_POLY_LOOM_WEAVE_DUPLICATE_CX";
pub const ERR_LOOM_WEAVE_PANEL_VERSION_MISMATCH: &str =
    "CALYX_POLY_LOOM_WEAVE_PANEL_VERSION_MISMATCH";
pub const ERR_LOOM_WEAVE_NON_DENSE_SLOT: &str = "CALYX_POLY_LOOM_WEAVE_NON_DENSE_SLOT";
pub const ERR_LOOM_WEAVE_TOO_FEW_DENSE_SLOTS: &str = "CALYX_POLY_LOOM_WEAVE_TOO_FEW_DENSE_SLOTS";
pub const ERR_LOOM_WEAVE_NO_XTERMS: &str = "CALYX_POLY_LOOM_WEAVE_NO_XTERMS";
pub const ERR_LOOM_WEAVE_XTERM_DECODE: &str = "CALYX_POLY_LOOM_WEAVE_XTERM_DECODE";
pub const ERR_LOOM_WEAVE_XTERM_KEY_MISMATCH: &str = "CALYX_POLY_LOOM_WEAVE_XTERM_KEY_MISMATCH";
pub const ERR_LOOM_WEAVE_READBACK_MISMATCH: &str = "CALYX_POLY_LOOM_WEAVE_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoomWeaveConstellationReport {
    pub cx_id: String,
    pub panel_version: u32,
    pub dense_slot_ids: Vec<u16>,
    pub inserted_xterms: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoomWeaveReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub domain: String,
    pub panel_version: u32,
    pub cache_capacity: usize,
    pub source_cx_ids: Vec<String>,
    pub constellation_count: usize,
    pub measured_slot_count: usize,
    pub xterm_count: usize,
    pub persisted_seq: Seq,
    pub constellations: Vec<LoomWeaveConstellationReport>,
    pub xterm_rows: Vec<XtermRow>,
    pub xterm_order: Vec<String>,
    pub agreement_graph: Vec<AgreementEdge>,
    pub agreement_graph_order: Vec<String>,
    pub graph_source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LoomWeaveRun {
    pub report_path: PathBuf,
    pub report: LoomWeaveReport,
    pub persisted_seq: Seq,
}

pub fn run_loom_weave_for_cx_ids<C: Clock>(
    vault: &AsterVault<C>,
    domain: &str,
    panel_version: u32,
    cx_ids: &[CxId],
    output_dir: &Path,
    cache_capacity: usize,
) -> Result<LoomWeaveRun> {
    validate_request(cx_ids)?;
    let snapshot = vault.snapshot();
    let mut loom = LoomStore::new(cache_capacity);
    let mut constellations = Vec::with_capacity(cx_ids.len());

    for cx_id in cx_ids {
        let constellation = vault.get(*cx_id, snapshot)?;
        if constellation.panel_version != panel_version {
            return Err(PolyError::diagnostics(
                ERR_LOOM_WEAVE_PANEL_VERSION_MISMATCH,
                format!(
                    "constellation {cx_id} panel_version {} does not match request {panel_version}",
                    constellation.panel_version
                ),
            ));
        }
        let dense_slots = dense_slots(&constellation)?;
        let inserted = loom.weave(*cx_id, &dense_slots)?;
        constellations.push(LoomWeaveConstellationReport {
            cx_id: cx_id.to_string(),
            panel_version: constellation.panel_version,
            dense_slot_ids: dense_slots.keys().map(|slot| slot.get()).collect(),
            inserted_xterms: inserted,
        });
    }

    let xterm_rows = loom.xterm_rows();
    if xterm_rows.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_LOOM_WEAVE_NO_XTERMS,
            "Loom weave produced no XTerm rows",
        ));
    }
    let kv_rows = loom.xterm_kv_rows()?;
    let persisted_seq = vault.write_cf_batch(
        kv_rows
            .into_iter()
            .map(|(key, value)| (ColumnFamily::XTerm, key, value)),
    )?;
    vault.flush()?;

    let readback_rows = readback_xterm_rows(vault, persisted_seq, cx_ids)?;
    if readback_rows != xterm_rows {
        return Err(PolyError::diagnostics(
            ERR_LOOM_WEAVE_READBACK_MISMATCH,
            format!(
                "computed {} XTerm rows but read back {} rows",
                xterm_rows.len(),
                readback_rows.len()
            ),
        ));
    }

    let agreement_graph = loom.agreement_graph()?;
    let report = LoomWeaveReport {
        schema_version: LOOM_WEAVE_SCHEMA_VERSION.to_string(),
        artifact_kind: LOOM_WEAVE_ARTIFACT_KIND.to_string(),
        domain: domain.to_string(),
        panel_version,
        cache_capacity,
        source_cx_ids: cx_ids.iter().map(ToString::to_string).collect(),
        constellation_count: cx_ids.len(),
        measured_slot_count: loom.measured_count(),
        xterm_count: xterm_rows.len(),
        persisted_seq,
        constellations,
        xterm_order: xterm_rows.iter().map(xterm_row_id).collect(),
        xterm_rows,
        agreement_graph_order: agreement_graph.iter().map(agreement_edge_id).collect(),
        agreement_graph,
        graph_source: "calyx_loom::LoomStore::agreement_graph after XTerm CF readback equality"
            .to_string(),
    };
    let report_path = write_loom_weave_report(output_dir, &report)?;
    Ok(LoomWeaveRun {
        report_path,
        report,
        persisted_seq,
    })
}

pub fn write_loom_weave_report(dir: &Path, report: &LoomWeaveReport) -> Result<PathBuf> {
    let file_name = format!(
        "loom_weave_{}_v{}.json",
        sanitize(&report.domain),
        report.panel_version
    );
    crate::diagnostics_store::write_json(dir, &file_name, report)
}

pub fn read_loom_weave_report(path: &Path) -> Result<LoomWeaveReport> {
    crate::diagnostics_store::read_json(path)
}

pub fn readback_xterm_rows<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    cx_ids: &[CxId],
) -> Result<Vec<XtermRow>> {
    let mut rows = Vec::new();
    let mut seen_keys = BTreeSet::new();
    for cx_id in cx_ids {
        let range = xterm_prefix_range(*cx_id);
        for (key, value) in vault.scan_cf_range_at(snapshot, ColumnFamily::XTerm, &range)? {
            if !seen_keys.insert(key.clone()) {
                continue;
            }
            let row: XtermRow = serde_json::from_slice(&value).map_err(|err| {
                PolyError::diagnostics(
                    ERR_LOOM_WEAVE_XTERM_DECODE,
                    format!("decode XTerm row for {cx_id}: {err}"),
                )
            })?;
            let expected_key = aster_xterm_key(&row);
            if key != expected_key {
                return Err(PolyError::diagnostics(
                    ERR_LOOM_WEAVE_XTERM_KEY_MISMATCH,
                    format!(
                        "XTerm CF key {} does not match row key {}",
                        hex(&key),
                        hex(&expected_key)
                    ),
                ));
            }
            rows.push(row);
        }
    }
    rows.sort_by_key(|row| row.key);
    Ok(rows)
}

fn validate_request(cx_ids: &[CxId]) -> Result<()> {
    if cx_ids.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_LOOM_WEAVE_EMPTY_INPUT,
            "Loom weave requires at least one stored constellation CxId",
        ));
    }
    let mut seen = BTreeSet::new();
    for cx_id in cx_ids {
        if !seen.insert(*cx_id) {
            return Err(PolyError::diagnostics(
                ERR_LOOM_WEAVE_DUPLICATE_CX,
                format!("duplicate CxId {cx_id} in Loom weave request"),
            ));
        }
    }
    Ok(())
}

fn dense_slots(constellation: &Constellation) -> Result<BTreeMap<SlotId, Vec<f32>>> {
    let mut dense = BTreeMap::new();
    for (slot, vector) in &constellation.slots {
        match vector {
            SlotVector::Dense { dim, data } => {
                if *dim == 0 || data.len() != *dim as usize {
                    return Err(PolyError::diagnostics(
                        ERR_LOOM_WEAVE_NON_DENSE_SLOT,
                        format!(
                            "constellation {} slot {} has invalid dense dim {dim} with {} values",
                            constellation.cx_id,
                            slot.get(),
                            data.len()
                        ),
                    ));
                }
                dense.insert(*slot, data.clone());
            }
            SlotVector::Sparse { .. } => {
                return Err(non_dense_slot_error(
                    constellation.cx_id,
                    *slot,
                    "sparse",
                    "Loom weave requires dense vector slots; sparse text slots need a dedicated sparse Loom path",
                ));
            }
            SlotVector::Multi { .. } => {
                return Err(non_dense_slot_error(
                    constellation.cx_id,
                    *slot,
                    "multi",
                    "Loom weave requires dense vector slots; multi-vector slots need a token-aware Loom path",
                ));
            }
            SlotVector::Absent { reason } => {
                return Err(non_dense_slot_error(
                    constellation.cx_id,
                    *slot,
                    "absent",
                    format!("slot is explicitly absent: {reason:?}"),
                ));
            }
        }
    }
    if dense.len() < 2 {
        return Err(PolyError::diagnostics(
            ERR_LOOM_WEAVE_TOO_FEW_DENSE_SLOTS,
            format!(
                "constellation {} has {} dense slot(s); Loom weave needs at least two",
                constellation.cx_id,
                dense.len()
            ),
        ));
    }
    Ok(dense)
}

fn non_dense_slot_error(
    cx_id: CxId,
    slot: SlotId,
    kind: &str,
    detail: impl Into<String>,
) -> PolyError {
    PolyError::diagnostics(
        ERR_LOOM_WEAVE_NON_DENSE_SLOT,
        format!(
            "constellation {cx_id} slot {} is {kind}; {}",
            slot.get(),
            detail.into()
        ),
    )
}

fn aster_xterm_key(row: &XtermRow) -> Vec<u8> {
    xterm_key(
        row.key.cx_id,
        row.key.a,
        row.key.b,
        match row.key.kind {
            CrossTermKind::Concat => XTermKind::Concat,
            CrossTermKind::Interaction => XTermKind::Interaction,
            CrossTermKind::Agreement => XTermKind::Agreement,
            CrossTermKind::Delta => XTermKind::Delta,
        },
    )
}

fn xterm_row_id(row: &XtermRow) -> String {
    format!(
        "{}:{}:{}:{:?}",
        row.key.cx_id,
        row.key.a.get(),
        row.key.b.get(),
        row.key.kind
    )
}

fn agreement_edge_id(edge: &AgreementEdge) -> String {
    format!("{}:{}", edge.a.get(), edge.b.get())
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
