//! Shape-aware Loom weave for heterogeneous Poly panel slots (issue #231).
//!
//! Dense same-dimension groups still use the real `calyx_loom::LoomStore`
//! path. Sparse same-dimension pairs use sparse cosine without densifying.
//! Every remaining pair is persisted in the report with an explicit unsupported
//! reason so heterogeneous panels do not silently skip cross-terms.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, XTermKind, xterm_key};
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Clock, CxId, SlotId, SlotVector, SparseEntry, VaultStore};
use calyx_loom::agreement_graph::XtermRow;
use calyx_loom::{
    AgreementEdge, CrossTermKey, CrossTermKind, CrossTermValue, LoomStore, SignalProvenanceTag,
    agreement_weight,
};

use crate::error::{PolyError, Result};
pub use crate::loom_shape_weave_types::{
    DenseGroupReport, ShapeAwareConstellationReport, ShapeAwareLoomWeaveReport,
    ShapeAwareLoomWeaveRun, UnsupportedShapePair,
};
use crate::loom_weave::{
    ERR_LOOM_WEAVE_DUPLICATE_CX, ERR_LOOM_WEAVE_EMPTY_INPUT, ERR_LOOM_WEAVE_PANEL_VERSION_MISMATCH,
    ERR_LOOM_WEAVE_READBACK_MISMATCH, readback_xterm_rows,
};

pub const SHAPE_AWARE_LOOM_WEAVE_SCHEMA_VERSION: &str = "poly.shape_aware_loom_weave.v1";
pub const SHAPE_AWARE_LOOM_WEAVE_ARTIFACT_KIND: &str = "poly_shape_aware_loom_weave";
pub const ERR_SHAPE_LOOM_INVALID_VECTOR: &str = "CALYX_POLY_SHAPE_LOOM_INVALID_VECTOR";

#[derive(Clone, Debug)]
struct ShapeSlot {
    slot: SlotId,
    shape: SlotShapeSummary,
}

#[derive(Clone, Debug)]
enum SlotShapeSummary {
    Dense {
        dim: u32,
        data: Vec<f32>,
        zero_norm: bool,
    },
    Sparse {
        dim: u32,
        entries: Vec<SparseEntry>,
        zero_norm: bool,
    },
    Multi {
        token_dim: u32,
        token_count: usize,
    },
    Absent {
        reason: String,
    },
}

pub fn run_shape_aware_loom_weave_for_cx_ids<C: Clock>(
    vault: &AsterVault<C>,
    domain: &str,
    panel_version: u32,
    cx_ids: &[CxId],
    output_dir: &Path,
    cache_capacity: usize,
) -> Result<ShapeAwareLoomWeaveRun> {
    validate_request(cx_ids)?;
    let snapshot = vault.snapshot();
    let mut xterms = BTreeMap::<CrossTermKey, XtermRow>::new();
    let mut unsupported_pairs = Vec::new();
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
        let slots = shape_slots(&constellation.slots)?;
        let before_xterms = xterms.len();
        let before_unsupported = unsupported_pairs.len();
        let dense_groups = weave_dense_groups(*cx_id, &slots, cache_capacity, &mut xterms)?;
        let sparse_count =
            weave_or_report_pairs(*cx_id, &slots, &mut xterms, &mut unsupported_pairs)?;
        constellations.push(ShapeAwareConstellationReport {
            cx_id: cx_id.to_string(),
            panel_version: constellation.panel_version,
            slot_count: slots.len(),
            dense_groups,
            sparse_agreement_count: sparse_count,
            unsupported_pair_count: unsupported_pairs.len() - before_unsupported,
            xterm_count: xterms.len() - before_xterms,
        });
    }

    let xterm_rows: Vec<XtermRow> = xterms.values().cloned().collect();
    let kv_rows = xterm_rows
        .iter()
        .map(|row| {
            let value = serde_json::to_vec(row).map_err(|error| {
                PolyError::from(CalyxError::disk_pressure(format!(
                    "encode shape-aware xterm row: {error}"
                )))
            })?;
            Ok((ColumnFamily::XTerm, aster_xterm_key(row), value))
        })
        .collect::<Result<Vec<_>>>()?;
    let persisted_seq = vault.write_cf_batch(kv_rows)?;
    vault.flush()?;
    let readback_rows = readback_xterm_rows(vault, persisted_seq, cx_ids)?;
    if readback_rows != xterm_rows {
        return Err(PolyError::diagnostics(
            ERR_LOOM_WEAVE_READBACK_MISMATCH,
            format!(
                "computed {} shape-aware XTerm rows but read back {} rows",
                xterm_rows.len(),
                readback_rows.len()
            ),
        ));
    }

    let agreement_graph = agreement_graph_from_rows(&readback_rows)?;
    let report = ShapeAwareLoomWeaveReport {
        schema_version: SHAPE_AWARE_LOOM_WEAVE_SCHEMA_VERSION.to_string(),
        artifact_kind: SHAPE_AWARE_LOOM_WEAVE_ARTIFACT_KIND.to_string(),
        domain: domain.to_string(),
        panel_version,
        cache_capacity,
        source_cx_ids: cx_ids.iter().map(ToString::to_string).collect(),
        constellation_count: cx_ids.len(),
        xterm_count: readback_rows.len(),
        unsupported_pair_count: unsupported_pairs.len(),
        persisted_seq,
        constellations,
        unsupported_pairs,
        xterm_order: readback_rows.iter().map(xterm_row_id).collect(),
        xterm_rows: readback_rows,
        agreement_graph_order: agreement_graph.iter().map(agreement_edge_id).collect(),
        agreement_graph,
        graph_source:
            "shape-aware dense LoomStore rows + sparse cosine rows after XTerm CF readback equality"
                .to_string(),
    };
    let report_path = write_shape_aware_loom_weave_report(output_dir, &report)?;
    Ok(ShapeAwareLoomWeaveRun {
        report_path,
        report,
        persisted_seq,
    })
}

pub fn write_shape_aware_loom_weave_report(
    dir: &Path,
    report: &ShapeAwareLoomWeaveReport,
) -> Result<PathBuf> {
    let file_name = format!(
        "shape_aware_loom_weave_{}_v{}.json",
        sanitize(&report.domain),
        report.panel_version
    );
    crate::diagnostics_store::write_json(dir, &file_name, report)
}

pub fn read_shape_aware_loom_weave_report(path: &Path) -> Result<ShapeAwareLoomWeaveReport> {
    crate::diagnostics_store::read_json(path)
}

fn validate_request(cx_ids: &[CxId]) -> Result<()> {
    if cx_ids.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_LOOM_WEAVE_EMPTY_INPUT,
            "shape-aware Loom weave requires at least one stored constellation CxId",
        ));
    }
    let mut seen = BTreeSet::new();
    for cx_id in cx_ids {
        if !seen.insert(*cx_id) {
            return Err(PolyError::diagnostics(
                ERR_LOOM_WEAVE_DUPLICATE_CX,
                format!("duplicate CxId {cx_id} in shape-aware Loom weave request"),
            ));
        }
    }
    Ok(())
}

fn shape_slots(slots: &BTreeMap<SlotId, SlotVector>) -> Result<Vec<ShapeSlot>> {
    let mut out = Vec::with_capacity(slots.len());
    for (slot, vector) in slots {
        vector.validate_schema().map_err(|err| {
            PolyError::diagnostics(
                ERR_SHAPE_LOOM_INVALID_VECTOR,
                format!("slot {} failed schema validation: {err}", slot.get()),
            )
        })?;
        let shape = match vector {
            SlotVector::Dense { dim, data } => SlotShapeSummary::Dense {
                dim: *dim,
                data: data.clone(),
                zero_norm: dense_zero_norm(data),
            },
            SlotVector::Sparse { dim, entries } => SlotShapeSummary::Sparse {
                dim: *dim,
                entries: entries.clone(),
                zero_norm: sparse_norm(entries) <= f32::EPSILON,
            },
            SlotVector::Multi { token_dim, tokens } => SlotShapeSummary::Multi {
                token_dim: *token_dim,
                token_count: tokens.len(),
            },
            SlotVector::Absent { reason } => SlotShapeSummary::Absent {
                reason: format!("{reason:?}"),
            },
        };
        out.push(ShapeSlot { slot: *slot, shape });
    }
    Ok(out)
}

fn weave_dense_groups(
    cx_id: CxId,
    slots: &[ShapeSlot],
    cache_capacity: usize,
    xterms: &mut BTreeMap<CrossTermKey, XtermRow>,
) -> Result<Vec<DenseGroupReport>> {
    let mut groups = BTreeMap::<u32, BTreeMap<SlotId, Vec<f32>>>::new();
    for slot in slots {
        if let SlotShapeSummary::Dense {
            dim,
            data,
            zero_norm: false,
        } = &slot.shape
        {
            groups
                .entry(*dim)
                .or_default()
                .insert(slot.slot, data.clone());
        }
    }

    let mut reports = Vec::new();
    for (dim, group) in groups.into_iter().filter(|(_, group)| group.len() >= 2) {
        let mut loom = LoomStore::new(cache_capacity);
        loom.weave(cx_id, &group)?;
        let rows = loom.xterm_rows();
        for row in &rows {
            xterms.insert(row.key, row.clone());
        }
        reports.push(DenseGroupReport {
            dim,
            slots: group.keys().map(|slot| slot.get()).collect(),
            xterm_count: rows.len(),
        });
    }
    Ok(reports)
}

fn weave_or_report_pairs(
    cx_id: CxId,
    slots: &[ShapeSlot],
    xterms: &mut BTreeMap<CrossTermKey, XtermRow>,
    unsupported: &mut Vec<UnsupportedShapePair>,
) -> Result<usize> {
    let mut sparse_count = 0usize;
    for i in 0..slots.len() {
        for j in (i + 1)..slots.len() {
            let left = &slots[i];
            let right = &slots[j];
            let key = agreement_key(cx_id, left.slot, right.slot);
            if xterms.contains_key(&key) {
                continue;
            }
            if let Some(row) = sparse_agreement_row(cx_id, left, right)? {
                xterms.insert(row.key, row);
                sparse_count += 1;
            } else {
                let (code, reason) = unsupported_reason(left, right);
                unsupported.push(UnsupportedShapePair {
                    cx_id: cx_id.to_string(),
                    slot_a: left.slot.get(),
                    slot_b: right.slot.get(),
                    shape_a: shape_label(&left.shape),
                    shape_b: shape_label(&right.shape),
                    reason_code: code.to_string(),
                    reason,
                });
            }
        }
    }
    Ok(sparse_count)
}

fn sparse_agreement_row(
    cx_id: CxId,
    left: &ShapeSlot,
    right: &ShapeSlot,
) -> Result<Option<XtermRow>> {
    let (
        SlotShapeSummary::Sparse {
            dim: left_dim,
            entries: left_entries,
            zero_norm: false,
        },
        SlotShapeSummary::Sparse {
            dim: right_dim,
            entries: right_entries,
            zero_norm: false,
        },
    ) = (&left.shape, &right.shape)
    else {
        return Ok(None);
    };
    if left_dim != right_dim {
        return Ok(None);
    }
    let value = sparse_cosine(left_entries, right_entries);
    Ok(Some(XtermRow {
        key: agreement_key(cx_id, left.slot, right.slot),
        value: CrossTermValue::Scalar(value),
        tag: SignalProvenanceTag::Derived,
    }))
}

fn sparse_cosine(left: &[SparseEntry], right: &[SparseEntry]) -> f32 {
    let mut left_map = BTreeMap::new();
    for entry in left {
        left_map.insert(entry.idx, entry.val);
    }
    let dot = right
        .iter()
        .filter_map(|entry| left_map.get(&entry.idx).map(|left| left * entry.val))
        .sum::<f32>();
    dot / (sparse_norm(left).sqrt() * sparse_norm(right).sqrt())
}

fn unsupported_reason(left: &ShapeSlot, right: &ShapeSlot) -> (&'static str, String) {
    match (&left.shape, &right.shape) {
        (
            SlotShapeSummary::Dense {
                zero_norm: true, ..
            },
            _,
        )
        | (
            _,
            SlotShapeSummary::Dense {
                zero_norm: true, ..
            },
        )
        | (
            SlotShapeSummary::Sparse {
                zero_norm: true, ..
            },
            _,
        )
        | (
            _,
            SlotShapeSummary::Sparse {
                zero_norm: true, ..
            },
        ) => (
            "zero_norm",
            "agreement requires non-zero vectors for both slots".to_string(),
        ),
        (SlotShapeSummary::Dense { dim: a, .. }, SlotShapeSummary::Dense { dim: b, .. }) => (
            "dense_dimension_mismatch",
            format!("dense agreement requires equal dimensions, got {a} and {b}"),
        ),
        (SlotShapeSummary::Sparse { dim: a, .. }, SlotShapeSummary::Sparse { dim: b, .. }) => (
            "sparse_dimension_mismatch",
            format!("sparse agreement requires equal dimensions, got {a} and {b}"),
        ),
        (SlotShapeSummary::Multi { .. }, _) | (_, SlotShapeSummary::Multi { .. }) => (
            "multi_vector_unsupported",
            "multi-vector slots require a token-aware Loom kernel".to_string(),
        ),
        (SlotShapeSummary::Absent { reason }, _) => (
            "slot_absent",
            format!("left slot is explicitly absent: {reason}"),
        ),
        (_, SlotShapeSummary::Absent { reason }) => (
            "slot_absent",
            format!("right slot is explicitly absent: {reason}"),
        ),
        _ => (
            "heterogeneous_shape_pair",
            "no mathematically valid cross-term kernel for this shape pair".to_string(),
        ),
    }
}

fn agreement_graph_from_rows(rows: &[XtermRow]) -> Result<Vec<AgreementEdge>> {
    let mut edges = BTreeMap::<(SlotId, SlotId), (f32, usize)>::new();
    for row in rows {
        if let CrossTermValue::Scalar(value) = row.value {
            let entry = edges.entry((row.key.a, row.key.b)).or_default();
            entry.0 += value;
            entry.1 += 1;
        }
    }
    edges
        .into_iter()
        .map(|((a, b), (sum, n))| {
            let raw = sum / n.max(1) as f32;
            Ok(AgreementEdge {
                a,
                b,
                raw_mean_agreement: raw,
                mean_agreement: raw,
                agreement_weight: agreement_weight(raw)?,
                n,
            })
        })
        .collect()
}

fn agreement_key(cx_id: CxId, a: SlotId, b: SlotId) -> CrossTermKey {
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    CrossTermKey {
        cx_id,
        a,
        b,
        kind: CrossTermKind::Agreement,
    }
}

fn aster_xterm_key(row: &XtermRow) -> Vec<u8> {
    xterm_key(row.key.cx_id, row.key.a, row.key.b, XTermKind::Agreement)
}

fn xterm_row_id(row: &XtermRow) -> String {
    format!("{}:{}:{}", row.key.cx_id, row.key.a.get(), row.key.b.get())
}

fn agreement_edge_id(edge: &AgreementEdge) -> String {
    format!("{}:{}", edge.a.get(), edge.b.get())
}

fn shape_label(shape: &SlotShapeSummary) -> String {
    match shape {
        SlotShapeSummary::Dense { dim, .. } => format!("dense:{dim}"),
        SlotShapeSummary::Sparse { dim, entries, .. } => {
            format!("sparse:{dim}:nnz={}", entries.len())
        }
        SlotShapeSummary::Multi {
            token_dim,
            token_count,
        } => format!("multi:{token_dim}:tokens={token_count}"),
        SlotShapeSummary::Absent { reason } => format!("absent:{reason}"),
    }
}

fn dense_zero_norm(values: &[f32]) -> bool {
    values.iter().map(|value| value * value).sum::<f32>() <= f32::EPSILON
}

fn sparse_norm(entries: &[SparseEntry]) -> f32 {
    entries.iter().map(|entry| entry.val * entry.val).sum()
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
