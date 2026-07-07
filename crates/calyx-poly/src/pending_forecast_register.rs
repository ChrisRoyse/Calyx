//! Durable pending-forecast register and resolution join (issue #234).
//!
//! The register is append-only at the source of truth: every pending forecast registration and every
//! resolution join is written to the vault ledger CF. The in-memory state is only the reconstructed
//! view callers use to avoid duplicate transitions and to hand exact work items to the feedback
//! controller.

use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, LedgerRef};
use calyx_ledger::{ActorId, EntryKind, SubjectId};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{PolyError, Result};
use crate::model::Resolution;
use crate::pending_forecast_payload::{
    pending_entry_payload, pending_work_item_payload, resolution_payload, safe_ref,
};
use crate::score::ForecastSource;

pub const PENDING_FORECAST_SCHEMA_VERSION: &str = "poly.pending_forecast_register.v1";
pub const PENDING_FORECAST_REGISTERED_EVENT: &str = "poly.pending_forecast_registered";
pub const PENDING_FORECAST_RESOLUTION_JOIN_EVENT: &str = "poly.pending_forecast_resolution_join";
pub const ERR_PENDING_FORECAST_INVALID: &str = "CALYX_POLY_PENDING_FORECAST_INVALID";
pub const ERR_PENDING_FORECAST_LEDGER_APPEND: &str = "CALYX_POLY_PENDING_FORECAST_LEDGER_APPEND";
pub const ERR_PENDING_FORECAST_PAYLOAD: &str = "CALYX_POLY_PENDING_FORECAST_PAYLOAD";

const ACTOR: &str = "calyx-poly-pending-forecast-register";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingForecastStatus {
    Pending,
    Scored,
    Void,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingForecastEntry {
    pub forecast_id: String,
    pub source: ForecastSource,
    pub condition_id: String,
    pub token_id: String,
    pub outcome_index: u32,
    pub domain: String,
    pub horizon_bucket: String,
    pub forecast_version: u32,
    pub p_model: f64,
    pub confidence: f64,
    pub forecast_ts: u64,
    pub provenance_hash: String,
    pub status: PendingForecastStatus,
    pub registered_ledger_seq: Option<u64>,
    pub terminal_ledger_seq: Option<u64>,
    pub terminal_resolution_id: Option<String>,
    pub terminal_actual_win: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PendingForecastRegister {
    pub entries: Vec<PendingForecastEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingForecastWorkItem {
    pub forecast_id: String,
    pub forecast_version: u32,
    pub condition_id: String,
    pub token_id: String,
    pub outcome_index: u32,
    pub p_model: f64,
    pub confidence: f64,
    pub actual_win: Option<bool>,
    pub status: PendingForecastStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolutionJoinResult {
    pub resolution_id: String,
    pub condition_id: String,
    pub voided: bool,
    pub idempotent_replay: bool,
    pub selected_forecast_ids: Vec<String>,
    pub transitioned_forecast_ids: Vec<String>,
    pub lookahead_blocked_forecast_ids: Vec<String>,
    pub work_items: Vec<PendingForecastWorkItem>,
    pub pending_after: usize,
    pub ledger_seq: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingForecastObservability {
    pub pending_count: usize,
    pub stale_pending_count: usize,
    pub pending_forecast_ids: Vec<String>,
    pub stale_pending_forecast_ids: Vec<String>,
}

pub trait PendingForecastLedgerStore {
    fn append_pending_forecast_ledger(
        &self,
        subject: SubjectId,
        payload: Vec<u8>,
    ) -> calyx_core::Result<LedgerRef>;
}

impl<C> PendingForecastLedgerStore for AsterVault<C>
where
    C: Clock,
{
    fn append_pending_forecast_ledger(
        &self,
        subject: SubjectId,
        payload: Vec<u8>,
    ) -> calyx_core::Result<LedgerRef> {
        self.append_ledger_entry(
            EntryKind::Measure,
            subject,
            payload,
            ActorId::Service(ACTOR.to_string()),
        )
    }
}

pub fn record_pending_forecast<S: PendingForecastLedgerStore>(
    store: &S,
    register: &mut PendingForecastRegister,
    mut entry: PendingForecastEntry,
) -> Result<LedgerRef> {
    validate_entry(&entry)?;
    entry.status = PendingForecastStatus::Pending;
    entry.registered_ledger_seq = None;
    entry.terminal_ledger_seq = None;
    entry.terminal_resolution_id = None;
    entry.terminal_actual_win = None;
    let payload = encode_payload(&json!({
        "schema_version": PENDING_FORECAST_SCHEMA_VERSION,
        "event": PENDING_FORECAST_REGISTERED_EVENT,
        "forecast": pending_entry_payload(&entry)
    }))?;
    let ledger_ref = append(store, subject_for(&entry.forecast_id), payload)?;
    entry.registered_ledger_seq = Some(ledger_ref.seq);
    register.entries.push(entry);
    Ok(ledger_ref)
}

pub fn join_resolution_to_pending_forecasts<S: PendingForecastLedgerStore>(
    store: &S,
    register: &mut PendingForecastRegister,
    resolution: &Resolution,
    voided: bool,
) -> Result<ResolutionJoinResult> {
    let resolution_id = resolution_join_id(resolution, voided);
    let blocked = register
        .entries
        .iter()
        .filter(|entry| {
            entry.condition_id == resolution.condition_id
                && entry.status == PendingForecastStatus::Pending
                && entry.forecast_ts >= resolution.resolved_ts
        })
        .map(|entry| entry.forecast_id.clone())
        .collect::<Vec<_>>();
    let pending_indexes = register
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            entry.condition_id == resolution.condition_id
                && entry.status == PendingForecastStatus::Pending
                && entry.forecast_ts < resolution.resolved_ts
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if pending_indexes.is_empty() {
        let replay = replay_items(register, resolution, &resolution_id, voided);
        if !replay.is_empty() {
            return Ok(join_result(
                register,
                resolution,
                JoinResultParts {
                    voided,
                    idempotent_replay: true,
                    work_items: replay,
                    transitioned: Vec::new(),
                    blocked,
                    ledger_seq: None,
                },
            ));
        }
        let payload = encode_payload(&join_payload(JoinPayloadParts {
            resolution,
            voided,
            resolution_id: &resolution_id,
            work_items: &[],
            transitioned: &[],
            blocked: &blocked,
            pending_after: pending_count(register),
            idempotent_replay: false,
        }))?;
        let ledger_ref = append(store, subject_for(&resolution_id), payload)?;
        return Ok(join_result(
            register,
            resolution,
            JoinResultParts {
                voided,
                idempotent_replay: false,
                work_items: Vec::new(),
                transitioned: Vec::new(),
                blocked,
                ledger_seq: Some(ledger_ref.seq),
            },
        ));
    }

    let status = if voided {
        PendingForecastStatus::Void
    } else {
        PendingForecastStatus::Scored
    };
    let ledger_payload_items = pending_indexes
        .iter()
        .map(|index| work_item(&register.entries[*index], resolution, voided, status))
        .collect::<Vec<_>>();
    let transitioned = ledger_payload_items
        .iter()
        .map(|item| item.forecast_id.clone())
        .collect::<Vec<_>>();
    let payload = encode_payload(&join_payload(JoinPayloadParts {
        resolution,
        voided,
        resolution_id: &resolution_id,
        work_items: &ledger_payload_items,
        transitioned: &transitioned,
        blocked: &blocked,
        pending_after: pending_count(register) - pending_indexes.len(),
        idempotent_replay: false,
    }))?;
    let ledger_ref = append(store, subject_for(&resolution_id), payload)?;
    for index in pending_indexes {
        let entry = &mut register.entries[index];
        entry.status = status;
        entry.terminal_ledger_seq = Some(ledger_ref.seq);
        entry.terminal_resolution_id = Some(resolution_id.clone());
        entry.terminal_actual_win =
            (!voided).then_some(entry.outcome_index == resolution.winning_outcome_index);
    }
    Ok(join_result(
        register,
        resolution,
        JoinResultParts {
            voided,
            idempotent_replay: false,
            work_items: ledger_payload_items,
            transitioned,
            blocked,
            ledger_seq: Some(ledger_ref.seq),
        },
    ))
}

pub fn observe_pending_forecasts(
    register: &PendingForecastRegister,
    now_ts: u64,
    stale_after_secs: u64,
) -> PendingForecastObservability {
    let mut pending = Vec::new();
    let mut stale = Vec::new();
    for entry in register
        .entries
        .iter()
        .filter(|entry| entry.status == PendingForecastStatus::Pending)
    {
        pending.push(entry.forecast_id.clone());
        if now_ts.saturating_sub(entry.forecast_ts) >= stale_after_secs {
            stale.push(entry.forecast_id.clone());
        }
    }
    PendingForecastObservability {
        pending_count: pending.len(),
        stale_pending_count: stale.len(),
        pending_forecast_ids: pending,
        stale_pending_forecast_ids: stale,
    }
}

fn replay_items(
    register: &PendingForecastRegister,
    resolution: &Resolution,
    resolution_id: &str,
    voided: bool,
) -> Vec<PendingForecastWorkItem> {
    register
        .entries
        .iter()
        .filter(|entry| {
            entry.condition_id == resolution.condition_id
                && entry.terminal_resolution_id.as_deref() == Some(resolution_id)
        })
        .map(|entry| work_item(entry, resolution, voided, entry.status))
        .collect()
}

fn join_result(
    register: &PendingForecastRegister,
    resolution: &Resolution,
    parts: JoinResultParts,
) -> ResolutionJoinResult {
    ResolutionJoinResult {
        resolution_id: resolution_join_id(resolution, parts.voided),
        condition_id: resolution.condition_id.clone(),
        voided: parts.voided,
        idempotent_replay: parts.idempotent_replay,
        selected_forecast_ids: parts
            .work_items
            .iter()
            .map(|item| item.forecast_id.clone())
            .collect(),
        transitioned_forecast_ids: parts.transitioned,
        lookahead_blocked_forecast_ids: parts.blocked,
        work_items: parts.work_items,
        pending_after: pending_count(register),
        ledger_seq: parts.ledger_seq,
    }
}

fn work_item(
    entry: &PendingForecastEntry,
    resolution: &Resolution,
    voided: bool,
    status: PendingForecastStatus,
) -> PendingForecastWorkItem {
    PendingForecastWorkItem {
        forecast_id: entry.forecast_id.clone(),
        forecast_version: entry.forecast_version,
        condition_id: entry.condition_id.clone(),
        token_id: entry.token_id.clone(),
        outcome_index: entry.outcome_index,
        p_model: entry.p_model,
        confidence: entry.confidence,
        actual_win: (!voided).then_some(entry.outcome_index == resolution.winning_outcome_index),
        status,
    }
}

struct JoinResultParts {
    voided: bool,
    idempotent_replay: bool,
    work_items: Vec<PendingForecastWorkItem>,
    transitioned: Vec<String>,
    blocked: Vec<String>,
    ledger_seq: Option<u64>,
}

struct JoinPayloadParts<'a> {
    resolution: &'a Resolution,
    voided: bool,
    resolution_id: &'a str,
    work_items: &'a [PendingForecastWorkItem],
    transitioned: &'a [String],
    blocked: &'a [String],
    pending_after: usize,
    idempotent_replay: bool,
}

fn join_payload(parts: JoinPayloadParts<'_>) -> serde_json::Value {
    let resolution = parts.resolution;
    json!({
        "schema_version": PENDING_FORECAST_SCHEMA_VERSION,
        "event": PENDING_FORECAST_RESOLUTION_JOIN_EVENT,
        "resolution_ref": safe_ref(parts.resolution_id),
        "resolution": resolution_payload(resolution),
        "voided": parts.voided,
        "idempotent_replay": parts.idempotent_replay,
        "selected_forecast_refs": parts.work_items.iter().map(|item| safe_ref(&item.forecast_id)).collect::<Vec<_>>(),
        "transitioned_forecast_refs": parts.transitioned.iter().map(|id| safe_ref(id)).collect::<Vec<_>>(),
        "lookahead_blocked_forecast_refs": parts.blocked.iter().map(|id| safe_ref(id)).collect::<Vec<_>>(),
        "pending_after": parts.pending_after,
        "work_items": parts.work_items.iter().map(pending_work_item_payload).collect::<Vec<_>>()
    })
}

fn append<S: PendingForecastLedgerStore>(
    store: &S,
    subject: SubjectId,
    payload: Vec<u8>,
) -> Result<LedgerRef> {
    store
        .append_pending_forecast_ledger(subject, payload)
        .map_err(|err| {
            PolyError::diagnostics(
                ERR_PENDING_FORECAST_LEDGER_APPEND,
                format!("append pending-forecast ledger row: {err}"),
            )
        })
}

fn encode_payload(value: &serde_json::Value) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|err| {
        PolyError::diagnostics(
            ERR_PENDING_FORECAST_PAYLOAD,
            format!("encode pending-forecast ledger payload: {err}"),
        )
    })
}

fn validate_entry(entry: &PendingForecastEntry) -> Result<()> {
    let labels = [
        &entry.forecast_id,
        &entry.condition_id,
        &entry.token_id,
        &entry.domain,
        &entry.horizon_bucket,
    ];
    if labels.iter().any(|value| value.trim().is_empty()) || entry.forecast_version == 0 {
        return invalid("forecast id, condition, token, domain, horizon, and version are required");
    }
    if !unit(entry.p_model)
        || !entry.confidence.is_finite()
        || !(0.0..1.0).contains(&entry.confidence)
    {
        return invalid("p_model must be in [0,1] and confidence must be in [0,1)");
    }
    if entry.provenance_hash.len() != 64
        || !entry
            .provenance_hash
            .chars()
            .all(|ch| ch.is_ascii_hexdigit())
    {
        return invalid("provenance_hash must be a 64-character hex digest");
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_PENDING_FORECAST_INVALID,
        message.into(),
    ))
}

fn unit(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn pending_count(register: &PendingForecastRegister) -> usize {
    register
        .entries
        .iter()
        .filter(|entry| entry.status == PendingForecastStatus::Pending)
        .count()
}

fn resolution_join_id(resolution: &Resolution, voided: bool) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        resolution.condition_id,
        resolution.resolved_ts,
        resolution.winning_outcome_index,
        resolution.source,
        voided
    )
}

fn subject_for(value: &str) -> SubjectId {
    SubjectId::Query(blake3::hash(value.as_bytes()).as_bytes().to_vec())
}
