use std::collections::BTreeMap;

use calyx_core::{
    Anchor, CalyxError, Clock, Constellation, CxFlags, CxId, GuardTauProfile, InputRef, LedgerRef,
    Modality, Result, SlotId, SlotVector, VaultStore,
};
use serde::{Deserialize, Serialize};

use super::engine::check_dedup_without_conflict_write;
use super::ingest_ledger::{LedgerPayload, action_name, action_name_for_action, ledger_payload};
use super::{
    AnchorConflictResult, ContestedWith, DedupAction, DedupDecision, DedupPolicy, DedupResult,
    OccurrenceId, check_anchor_conflict, contested_with_key, dedup_error, encode_contested_with,
};
use crate::cf::ColumnFamily;
use crate::recurrence::{OccurrenceContext, RetentionPolicy, build_append};
use crate::vault::AsterVault;

pub const CALYX_DEDUP_INVALID_EVENT_TIME: &str = "CALYX_DEDUP_INVALID_EVENT_TIME";

const EVENT_TIME_SCALAR: &str = "event_time_secs";
const OCCURRENCE_PREFIX: &[u8] = b"dedup:occurrence:";
const COLLAPSE_PREFIX: &[u8] = b"dedup:collapse:";
const LINK_PREFIX: &[u8] = b"dedup:link:";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EpochSecs(pub i64);

impl EpochSecs {
    pub fn to_u64(self) -> Result<u64> {
        u64::try_from(self.0).map_err(|_| {
            dedup_error(
                CALYX_DEDUP_INVALID_EVENT_TIME,
                format!("event time {} is before Unix epoch", self.0),
            )
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IngestInput {
    pub raw_bytes: Vec<u8>,
    pub panel_version: u32,
    pub modality: Modality,
    pub slots: BTreeMap<SlotId, SlotVector>,
    pub scalars: BTreeMap<String, f64>,
    pub anchors: Vec<Anchor>,
    pub input_pointer: Option<String>,
    pub redacted: bool,
}

impl IngestInput {
    pub fn new(raw_bytes: impl Into<Vec<u8>>, panel_version: u32, modality: Modality) -> Self {
        Self {
            raw_bytes: raw_bytes.into(),
            panel_version,
            modality,
            slots: BTreeMap::new(),
            scalars: BTreeMap::new(),
            anchors: Vec::new(),
            input_pointer: None,
            redacted: true,
        }
    }

    pub fn with_slot(mut self, slot: SlotId, vector: SlotVector) -> Self {
        self.slots.insert(slot, vector);
        self
    }

    pub fn with_anchor(mut self, anchor: Anchor) -> Self {
        self.anchors.push(anchor);
        self
    }

    fn to_constellation<C>(&self, vault: &AsterVault<C>, at: EpochSecs) -> Result<Constellation>
    where
        C: Clock,
    {
        let event_time = at.to_u64()?;
        let input_hash = *blake3::hash(&self.raw_bytes).as_bytes();
        let mut scalars = self.scalars.clone();
        scalars.insert(EVENT_TIME_SCALAR.to_string(), at.0 as f64);
        Ok(Constellation {
            cx_id: vault.cx_id_for_input(&self.raw_bytes, self.panel_version),
            vault_id: vault.vault_id(),
            panel_version: self.panel_version,
            created_at: event_time,
            input_ref: InputRef {
                hash: input_hash,
                pointer: self.input_pointer.clone(),
                redacted: self.redacted,
            },
            modality: self.modality,
            slots: self.slots.clone(),
            scalars,
            anchors: self.anchors.clone(),
            provenance: LedgerRef {
                seq: 0,
                hash: [0; 32],
            },
            flags: CxFlags {
                ungrounded: self.anchors.is_empty(),
                redacted_input: self.redacted,
                ..CxFlags::default()
            },
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DedupOnlineKind {
    Occurrence,
    Collapse,
    Link,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DedupOnlineEvent {
    pub kind: DedupOnlineKind,
    pub into: CxId,
    pub source: CxId,
    pub occurrence: OccurrenceId,
    pub at: EpochSecs,
    pub action: DedupAction,
    pub per_slot_cos: Vec<(SlotId, f32)>,
}

pub fn ingest_at<C>(
    vault: &AsterVault<C>,
    input: &IngestInput,
    at: EpochSecs,
    guard_profile: Option<&dyn GuardTauProfile>,
) -> Result<DedupResult>
where
    C: Clock,
{
    let new_cx = input.to_constellation(vault, at)?;
    let policy = vault.dedup_policy().clone();
    let decision = check_dedup_without_conflict_write(&new_cx, vault, &policy, guard_profile)?;
    match decision {
        DedupDecision::NoMatch => store_new(vault, new_cx, at, &policy, "NoMatch", Vec::new()),
        DedupDecision::AnchorConflict { existing } => {
            let existing_cx = vault.get(existing, vault.snapshot())?;
            let online_rows = contested_rows(&new_cx, &existing_cx)?;
            store_new(vault, new_cx, at, &policy, "AnchorConflict", online_rows)
        }
        DedupDecision::Match {
            existing,
            per_slot_cos,
        } => match &policy {
            DedupPolicy::Exact => exact_duplicate(vault, &new_cx, at, existing, per_slot_cos),
            DedupPolicy::TctCosine(config) => {
                if same_event_exact(vault, new_cx.cx_id, existing, at)? {
                    exact_duplicate(vault, &new_cx, at, existing, per_slot_cos)
                } else {
                    merge_match(
                        vault,
                        new_cx,
                        at,
                        existing,
                        per_slot_cos,
                        config.action.clone(),
                    )
                }
            }
            DedupPolicy::Off => store_new(vault, new_cx, at, &policy, "NoMatch", Vec::new()),
        },
    }
}

pub fn ingest<C>(
    vault: &AsterVault<C>,
    input: &IngestInput,
    clock: &dyn Clock,
    guard_profile: Option<&dyn GuardTauProfile>,
) -> Result<DedupResult>
where
    C: Clock,
{
    let now_secs = i64::try_from(clock.now() / 1_000).map_err(|_| {
        dedup_error(
            CALYX_DEDUP_INVALID_EVENT_TIME,
            "clock timestamp does not fit EpochSecs",
        )
    })?;
    ingest_at(vault, input, EpochSecs(now_secs), guard_profile)
}

pub fn dedup_online_key(kind: DedupOnlineKind, into: CxId, occurrence: OccurrenceId) -> Vec<u8> {
    let prefix = match kind {
        DedupOnlineKind::Occurrence => OCCURRENCE_PREFIX,
        DedupOnlineKind::Collapse => COLLAPSE_PREFIX,
        DedupOnlineKind::Link => LINK_PREFIX,
    };
    event_key(prefix, into, occurrence)
}

pub fn decode_dedup_online_event(bytes: &[u8]) -> Result<DedupOnlineEvent> {
    serde_json::from_slice(bytes).map_err(|error| {
        CalyxError::aster_corrupt_shard(format!("decode dedup online event: {error}"))
    })
}

fn store_new<C>(
    vault: &AsterVault<C>,
    mut new_cx: Constellation,
    at: EpochSecs,
    policy: &DedupPolicy,
    decision: &'static str,
    mut online_rows: Vec<(Vec<u8>, Vec<u8>)>,
) -> Result<DedupResult>
where
    C: Clock,
{
    let is_recurrence_series = matches!(
        policy,
        DedupPolicy::TctCosine(config) if config.action == DedupAction::RecurrenceSeries
    );
    let mut recurrence_rows = Vec::new();
    if is_recurrence_series {
        let append = build_append(
            vault,
            new_cx,
            at,
            OccurrenceContext::new(Vec::new())?,
            at,
            RetentionPolicy::default(),
        )?;
        let occurrence = append.occurrence_id;
        new_cx = append.updated_base;
        recurrence_rows = append.recurrence_rows;
        online_rows.push(online_event_row(
            DedupOnlineKind::Occurrence,
            new_cx.cx_id,
            new_cx.cx_id,
            occurrence,
            at,
            DedupAction::RecurrenceSeries,
            Vec::new(),
        )?);
    }
    let payload = ledger_payload(LedgerPayload {
        cx: &new_cx,
        at,
        result: "New",
        decision,
        action: action_name(policy),
        into: None,
        occurrence: None,
        per_slot_cos: &[],
    })?;
    let id = new_cx.cx_id;
    vault.commit_dedup_ingest(
        Some(new_cx),
        None,
        online_rows,
        recurrence_rows,
        id,
        payload,
    )?;
    Ok(DedupResult::New(id))
}

fn exact_duplicate<C>(
    vault: &AsterVault<C>,
    new_cx: &Constellation,
    at: EpochSecs,
    existing: CxId,
    per_slot_cos: Vec<(SlotId, f32)>,
) -> Result<DedupResult>
where
    C: Clock,
{
    let payload = ledger_payload(LedgerPayload {
        cx: new_cx,
        at,
        result: "ExactDuplicate",
        decision: "Match",
        action: Some("Exact"),
        into: Some(existing),
        occurrence: None,
        per_slot_cos: &per_slot_cos,
    })?;
    vault.commit_dedup_ingest(None, None, Vec::new(), Vec::new(), existing, payload)?;
    Ok(DedupResult::ExactDuplicate(existing))
}

fn merge_match<C>(
    vault: &AsterVault<C>,
    new_cx: Constellation,
    at: EpochSecs,
    existing: CxId,
    per_slot_cos: Vec<(SlotId, f32)>,
    action: DedupAction,
) -> Result<DedupResult>
where
    C: Clock,
{
    let kind = online_kind(&action);
    let mut updated_base = None;
    let mut recurrence_rows = Vec::new();
    let occurrence = if action == DedupAction::RecurrenceSeries {
        let base = vault.get(existing, vault.snapshot())?;
        let append = build_append(
            vault,
            base,
            at,
            OccurrenceContext::new(Vec::new())?,
            at,
            RetentionPolicy::default(),
        )?;
        updated_base = Some(append.updated_base);
        recurrence_rows = append.recurrence_rows;
        append.occurrence_id
    } else {
        next_occurrence_id(vault, kind, existing)?
    };
    let online_rows = vec![online_event_row(
        kind,
        existing,
        new_cx.cx_id,
        occurrence,
        at,
        action.clone(),
        per_slot_cos.clone(),
    )?];
    let payload = ledger_payload(LedgerPayload {
        cx: &new_cx,
        at,
        result: "DedupMerge",
        decision: "Match",
        action: Some(action_name_for_action(&action)),
        into: Some(existing),
        occurrence: Some(occurrence),
        per_slot_cos: &per_slot_cos,
    })?;
    let candidate = (action == DedupAction::Link).then_some(new_cx);
    let subject = candidate.as_ref().map_or(existing, |cx| cx.cx_id);
    vault.commit_dedup_ingest(
        candidate,
        updated_base,
        online_rows,
        recurrence_rows,
        subject,
        payload,
    )?;
    Ok(DedupResult::DedupMerge {
        into: existing,
        occurrence,
    })
}

fn same_event_exact<C>(
    vault: &AsterVault<C>,
    new_id: CxId,
    existing: CxId,
    at: EpochSecs,
) -> Result<bool>
where
    C: Clock,
{
    if new_id != existing {
        return Ok(false);
    }
    let existing_cx = vault.get(existing, vault.snapshot())?;
    Ok(existing_cx.created_at == at.to_u64()?)
}

fn contested_rows(
    new_cx: &Constellation,
    existing_cx: &Constellation,
) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let AnchorConflictResult::Conflicting {
        anchor_type,
        reason,
    } = check_anchor_conflict(new_cx, existing_cx)
    else {
        return Err(CalyxError::aster_corrupt_shard(
            "dedup decision reported anchor conflict but anchors are compatible",
        ));
    };
    let new_value = ContestedWith {
        contested_with: existing_cx.cx_id,
        anchor_type: anchor_type.clone(),
        reason: reason.clone(),
    };
    let existing_value = ContestedWith {
        contested_with: new_cx.cx_id,
        anchor_type,
        reason,
    };
    Ok(vec![
        (
            contested_with_key(new_cx.cx_id),
            encode_contested_with(&new_value)?,
        ),
        (
            contested_with_key(existing_cx.cx_id),
            encode_contested_with(&existing_value)?,
        ),
    ])
}

fn next_occurrence_id<C>(
    vault: &AsterVault<C>,
    kind: DedupOnlineKind,
    into: CxId,
) -> Result<OccurrenceId>
where
    C: Clock,
{
    let prefix = event_prefix(kind, into);
    let count = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Online)?
        .into_iter()
        .filter(|(key, _)| key.starts_with(&prefix))
        .count();
    Ok(OccurrenceId(count as u64))
}

fn online_event_row(
    kind: DedupOnlineKind,
    into: CxId,
    source: CxId,
    occurrence: OccurrenceId,
    at: EpochSecs,
    action: DedupAction,
    per_slot_cos: Vec<(SlotId, f32)>,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let event = DedupOnlineEvent {
        kind,
        into,
        source,
        occurrence,
        at,
        action,
        per_slot_cos,
    };
    let key = dedup_online_key(kind, into, occurrence);
    let value = serde_json::to_vec(&event).map_err(|error| {
        CalyxError::aster_corrupt_shard(format!("encode dedup online event: {error}"))
    })?;
    Ok((key, value))
}

fn online_kind(action: &DedupAction) -> DedupOnlineKind {
    match action {
        DedupAction::Collapse => DedupOnlineKind::Collapse,
        DedupAction::Link => DedupOnlineKind::Link,
        DedupAction::RecurrenceSeries => DedupOnlineKind::Occurrence,
    }
}

fn event_prefix(kind: DedupOnlineKind, into: CxId) -> Vec<u8> {
    let prefix = match kind {
        DedupOnlineKind::Occurrence => OCCURRENCE_PREFIX,
        DedupOnlineKind::Collapse => COLLAPSE_PREFIX,
        DedupOnlineKind::Link => LINK_PREFIX,
    };
    let mut key = Vec::with_capacity(prefix.len() + 16);
    key.extend_from_slice(prefix);
    key.extend_from_slice(into.as_bytes());
    key
}

fn event_key(prefix: &[u8], into: CxId, occurrence: OccurrenceId) -> Vec<u8> {
    let mut key = Vec::with_capacity(prefix.len() + 24);
    key.extend_from_slice(prefix);
    key.extend_from_slice(into.as_bytes());
    key.extend_from_slice(&occurrence.0.to_be_bytes());
    key
}

#[cfg(test)]
#[path = "ingest_at_tests.rs"]
mod tests;
