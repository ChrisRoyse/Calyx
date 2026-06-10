use calyx_core::{CalyxError, CxId, Result, SlotId};
use serde::{Deserialize, Serialize};

use super::{DedupAction, EpochSecs, OccurrenceId};

const OCCURRENCE_PREFIX: &[u8] = b"dedup:occurrence:";
const COLLAPSE_PREFIX: &[u8] = b"dedup:collapse:";
const LINK_PREFIX: &[u8] = b"dedup:link:";

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

pub(super) fn next_online_prefix(kind: DedupOnlineKind, into: CxId) -> Vec<u8> {
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

pub(super) fn online_event_row(
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

pub(super) fn online_kind(action: &DedupAction) -> DedupOnlineKind {
    match action {
        DedupAction::Collapse => DedupOnlineKind::Collapse,
        DedupAction::Link => DedupOnlineKind::Link,
        DedupAction::RecurrenceSeries => DedupOnlineKind::Occurrence,
    }
}

fn event_key(prefix: &[u8], into: CxId, occurrence: OccurrenceId) -> Vec<u8> {
    let mut key = Vec::with_capacity(prefix.len() + 24);
    key.extend_from_slice(prefix);
    key.extend_from_slice(into.as_bytes());
    key.extend_from_slice(&occurrence.0.to_be_bytes());
    key
}
