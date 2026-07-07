//! Ingest a snapshot into a Calyx vault and ground it on resolution.
//!
//! This is the seam onto the engine. [`ingest_snapshot`] builds a constellation and persists it via
//! [`VaultStore::put`] (the real source of truth). [`ground_market`] attaches resolution anchors to
//! every stored snapshot of a resolved market.
//!
//! The bounded **association fan-out** path runs the cheap-screen selection report during ingest and
//! ledgers the selected expensive-confirm set against the ingested [`CxId`]. Heavier confirmers
//! (Loom cross-terms, temporal lead/lag, Assay bits, kernel build) consume that persisted boundary.

use std::path::{Path, PathBuf};

use calyx_aster::vault::AsterVault;
use calyx_core::{Anchor, AnchorKind, AnchorValue, Clock, CxId, LedgerRef, VaultId, VaultStore};
use calyx_ledger::{ActorId, EntryKind, SubjectId};

use crate::constellation::{build_constellation, resolution_anchor, resolution_label_anchor};
use crate::error::{PolyError, Result};
use crate::fanout_selection::{
    FanoutSelectionReport, FanoutSelectionRequest, FanoutSelectionRun,
    compute_fanout_selection_report, run_fanout_selection_report,
};
use crate::lenses::PolyPanel;
use crate::model::{MarketSnapshot, Resolution};
use crate::no_lookahead::{
    NoLookaheadTiming, validate_no_lookahead_timing, validate_resolution_anchor_timing,
};

pub const ASSOCIATION_FANOUT_INGEST_SCHEMA_VERSION: &str = "poly.association_fanout_ingest.v1";
pub const ERR_ASSOCIATION_FANOUT_INVALID_REQUEST: &str =
    "CALYX_POLY_ASSOCIATION_FANOUT_INVALID_REQUEST";
pub const ERR_ASSOCIATION_FANOUT_NO_SELECTED: &str = "CALYX_POLY_ASSOCIATION_FANOUT_NO_SELECTED";
pub const ERR_ASSOCIATION_FANOUT_REPORT_READ: &str = "CALYX_POLY_ASSOCIATION_FANOUT_REPORT_READ";
pub const ERR_ASSOCIATION_FANOUT_PAYLOAD_ENCODE: &str =
    "CALYX_POLY_ASSOCIATION_FANOUT_PAYLOAD_ENCODE";
pub const ERR_ASSOCIATION_FANOUT_INGEST_MISMATCH: &str =
    "CALYX_POLY_ASSOCIATION_FANOUT_INGEST_MISMATCH";

#[derive(Clone, Debug, PartialEq)]
pub struct AssociationFanoutIngestRun {
    pub cx_id: CxId,
    pub fanout_report_path: PathBuf,
    pub fanout_report: FanoutSelectionReport,
    pub ledger_ref: LedgerRef,
}

/// Ingests one market snapshot into the given domain vault, returning its content-addressed id.
///
/// `vault_id` identifies the domain vault (one per domain); the caller holds the matching store
/// handle. Fail-closed: any store error propagates; nothing is silently dropped.
pub fn ingest_snapshot<S: VaultStore>(
    store: &S,
    panel: &PolyPanel,
    snapshot: &MarketSnapshot,
    vault_id: VaultId,
    vault_salt: &[u8],
) -> Result<CxId> {
    let constellation = build_constellation(snapshot, panel, vault_id, vault_salt)?;
    let cx = store.put(constellation)?;
    Ok(cx)
}

/// Store capability required to ledger the association fan-out decision produced during ingest.
pub trait AssociationFanoutLedgerStore: VaultStore {
    fn append_association_fanout_ledger(
        &self,
        cx_id: CxId,
        payload: Vec<u8>,
    ) -> calyx_core::Result<LedgerRef>;
}

impl<C> AssociationFanoutLedgerStore for AsterVault<C>
where
    C: Clock,
{
    fn append_association_fanout_ledger(
        &self,
        cx_id: CxId,
        payload: Vec<u8>,
    ) -> calyx_core::Result<LedgerRef> {
        self.append_ledger_entry(
            EntryKind::Measure,
            SubjectId::Cx(cx_id),
            payload,
            ActorId::Service("calyx-poly-association-fanout".to_string()),
        )
    }
}

/// Ingests one snapshot and, as part of the same domain operation, persists and ledgers the bounded
/// association fan-out report for the supplied cheap-screen candidates.
///
/// Fan-out preflight runs before `put`, so malformed fan-out requests fail without creating a vault
/// row. The persisted report remains the source of truth for selected/dropped candidate pairs; the
/// ledger payload stores only a content hash and compact summary that point back to that report.
pub fn ingest_snapshot_with_association_fanout<S: AssociationFanoutLedgerStore>(
    store: &S,
    panel: &PolyPanel,
    snapshot: &MarketSnapshot,
    vault_id: VaultId,
    vault_salt: &[u8],
    fanout_output_root: &Path,
    fanout_request: &FanoutSelectionRequest,
) -> Result<AssociationFanoutIngestRun> {
    if fanout_request.panel_version != panel.version {
        return Err(PolyError::diagnostics(
            ERR_ASSOCIATION_FANOUT_INVALID_REQUEST,
            format!(
                "fan-out panel_version {} must match ingest panel version {}",
                fanout_request.panel_version, panel.version
            ),
        ));
    }
    let preflight = compute_fanout_selection_report(fanout_request)?;
    if preflight.selected_count == 0 {
        return Err(PolyError::diagnostics(
            ERR_ASSOCIATION_FANOUT_NO_SELECTED,
            "association fan-out produced no expensive-confirm candidates",
        ));
    }

    let constellation = build_constellation(snapshot, panel, vault_id, vault_salt)?;
    let expected_cx = constellation.cx_id;
    let fanout_run = run_fanout_selection_report(fanout_output_root, fanout_request)?;
    debug_assert_eq!(fanout_run.report, preflight);

    let stored_cx = store.put(constellation)?;
    if stored_cx != expected_cx {
        return Err(PolyError::diagnostics(
            ERR_ASSOCIATION_FANOUT_INGEST_MISMATCH,
            format!("store returned {stored_cx} for precomputed CxId {expected_cx}"),
        ));
    }
    let payload = association_fanout_payload(stored_cx, &fanout_run)?;
    let ledger_ref = store.append_association_fanout_ledger(stored_cx, payload)?;
    Ok(AssociationFanoutIngestRun {
        cx_id: stored_cx,
        fanout_report_path: fanout_run.report_path,
        fanout_report: fanout_run.report,
        ledger_ref,
    })
}

fn association_fanout_payload(cx_id: CxId, run: &FanoutSelectionRun) -> Result<Vec<u8>> {
    let report_bytes = std::fs::read(&run.report_path).map_err(|err| {
        PolyError::diagnostics(
            ERR_ASSOCIATION_FANOUT_REPORT_READ,
            format!("read fan-out report {}: {err}", run.report_path.display()),
        )
    })?;
    let report_blake3 = blake3::hash(&report_bytes).to_hex().to_string();
    let report_file = run
        .report_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("fanout_selection_report")
        .to_string();
    serde_json::to_vec(&serde_json::json!({
        "schema_version": ASSOCIATION_FANOUT_INGEST_SCHEMA_VERSION,
        "event": "poly.association_fanout_ingest",
        "cx_id": cx_id.to_string(),
        "domain": &run.report.domain,
        "panel_version": run.report.panel_version,
        "fanout_report_file": report_file,
        "fanout_report_blake3_chunks": hex_chunks(&report_blake3),
        "input_count": run.report.input_count,
        "selected_count": run.report.selected_count,
        "dropped_count": run.report.dropped_count,
        "expensive_estimators": &run.report.expensive_estimators,
        "selected_pairs": run.report.selected.iter().map(|decision| {
            serde_json::json!({
                "pair_id": &decision.pair_id,
                "left_axis": &decision.left_key,
                "right_axis": &decision.right_key,
                "cheap_score": decision.cheap_score,
                "expensive_rank": decision.expensive_rank
            })
        }).collect::<Vec<_>>(),
        "dropped_pairs": run.report.dropped.iter().map(|decision| {
            serde_json::json!({
                "pair_id": &decision.pair_id,
                "drop_reason": &decision.drop_reason
            })
        }).collect::<Vec<_>>()
    }))
    .map_err(|err| {
        PolyError::diagnostics(
            ERR_ASSOCIATION_FANOUT_PAYLOAD_ENCODE,
            format!("encode association fan-out ledger payload: {err}"),
        )
    })
}

fn hex_chunks(hex: &str) -> Vec<String> {
    hex.as_bytes()
        .chunks(16)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default().to_string())
        .collect()
}

/// Store capability required for ledger-stamped outcome grounding.
pub trait GroundingLedgerStore: VaultStore {
    /// Writes outcome anchors and a same-commit grounding ledger entry.
    fn anchors_with_grounding_ledger(
        &self,
        id: CxId,
        anchors: Vec<Anchor>,
        payload: Vec<u8>,
    ) -> calyx_core::Result<LedgerRef>;
}

impl<C> GroundingLedgerStore for AsterVault<C>
where
    C: Clock,
{
    fn anchors_with_grounding_ledger(
        &self,
        id: CxId,
        anchors: Vec<Anchor>,
        payload: Vec<u8>,
    ) -> calyx_core::Result<LedgerRef> {
        self.anchors_with_ledger_entry(
            id,
            anchors,
            EntryKind::Grounding,
            SubjectId::Cx(id),
            payload,
            ActorId::Service("calyx-poly-grounding".to_string()),
        )
    }
}

/// Grounds a resolved market: writes the outcome anchor onto every stored snapshot `cx_id`.
///
/// Fail-closed: every anchor must be written with a real grounding ledger entry.
pub fn ground_market<S: GroundingLedgerStore>(
    store: &S,
    snapshot_cx_ids: &[CxId],
    resolution: &Resolution,
    outcome_index: u32,
) -> Result<Vec<LedgerRef>> {
    if snapshot_cx_ids.is_empty() {
        return Err(PolyError::grounding(
            "CALYX_POLY_GROUNDING_EMPTY_SCOPE",
            "ground_market requires at least one snapshot CxId",
        ));
    }
    let anchors = vec![
        resolution_anchor(resolution, outcome_index),
        resolution_label_anchor(resolution),
    ];
    let mut refs = Vec::with_capacity(snapshot_cx_ids.len());
    for cx in snapshot_cx_ids {
        let stored = store.get(*cx, store.snapshot())?;
        let resolution_observed_at = resolution.resolved_ts.saturating_mul(1000);
        let timing = NoLookaheadTiming {
            feature_max_observed_at: stored.created_at,
            snapshot_observed_at: stored.created_at,
            resolution_observed_at,
            backfill_observed_at: resolution_observed_at,
        };
        validate_no_lookahead_timing(&timing)?;
        validate_resolution_anchor_timing(&timing, &anchors)?;
        let payload = grounding_payload(*cx, &anchors, resolution, outcome_index)?;
        refs.push(store.anchors_with_grounding_ledger(*cx, anchors.clone(), payload)?);
    }
    Ok(refs)
}

fn grounding_payload(
    cx_id: CxId,
    anchors: &[Anchor],
    resolution: &Resolution,
    outcome_index: u32,
) -> Result<Vec<u8>> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "event": "poly.outcome_grounding",
        "cx_id": cx_id.to_string(),
        "condition_id": resolution.condition_id,
        "winning_outcome_index": resolution.winning_outcome_index,
        "winning_label": resolution.winning_label,
        "resolved_ts": resolution.resolved_ts,
        "resolution_source": resolution.source,
        "disputed": resolution.disputed,
        "grounded_outcome_index": outcome_index,
        "anchors": anchors.iter().map(anchor_payload).collect::<Vec<_>>()
    }))
    .map_err(|err| {
        PolyError::grounding(
            "CALYX_POLY_GROUNDING_PAYLOAD_ENCODE_FAILED",
            format!("encode grounding ledger payload: {err}"),
        )
    })
}

fn anchor_payload(anchor: &Anchor) -> serde_json::Value {
    let (kind, label_axis) = match &anchor.kind {
        AnchorKind::TestPass => ("TestPass", None),
        AnchorKind::TieFormed => ("TieFormed", None),
        AnchorKind::Thumbs => ("Thumbs", None),
        AnchorKind::Label(axis) => ("Label", Some(axis.as_str())),
        AnchorKind::Reward => ("Reward", None),
        AnchorKind::SpeakerMatch => ("SpeakerMatch", None),
        AnchorKind::StyleHold => ("StyleHold", None),
        AnchorKind::Recurrence => ("Recurrence", None),
    };
    serde_json::json!({
        "kind": kind,
        "label_axis": label_axis,
        "source": anchor.source,
        "observed_at": anchor.observed_at,
        "confidence": anchor.confidence,
        "value": anchor_value_payload(&anchor.value)
    })
}

fn anchor_value_payload(value: &AnchorValue) -> serde_json::Value {
    match value {
        AnchorValue::Bool(value) => serde_json::json!({"type": "bool", "value": value}),
        AnchorValue::Enum(value) => serde_json::json!({"type": "enum", "value": value}),
        AnchorValue::Number(value) => serde_json::json!({"type": "number", "value": value}),
        AnchorValue::OneHot(value) => serde_json::json!({"type": "one_hot", "value": value}),
        AnchorValue::Text(value) => serde_json::json!({"type": "text", "value": value}),
        AnchorValue::Vector(value) => serde_json::json!({"type": "vector", "value": value}),
    }
}

#[cfg(test)]
mod tests {
    include!("pipeline_tests.rs");
}
