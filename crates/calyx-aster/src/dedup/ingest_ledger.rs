use calyx_core::{CalyxError, Constellation, CxId, Result, SlotId};
use serde_json::json;

use super::{DedupAction, DedupPolicy, DedupRestoreSnapshot, EpochSecs, OccurrenceId};

pub(super) struct LedgerPayload<'a> {
    pub cx: &'a Constellation,
    pub at: EpochSecs,
    pub result: &'static str,
    pub decision: &'static str,
    pub action: Option<&'static str>,
    pub into: Option<CxId>,
    pub occurrence: Option<OccurrenceId>,
    pub per_slot_cos: &'a [(SlotId, f32)],
    pub recurrence_signature: Option<RecurrenceSignatureLedger>,
    pub restore: Option<&'a DedupRestoreSnapshot>,
}

#[derive(Clone, Copy)]
pub(super) struct RecurrenceSignatureLedger {
    pub same_action: CxId,
    pub new_time: EpochSecs,
}

pub(super) fn ledger_payload(payload: LedgerPayload<'_>) -> Result<Vec<u8>> {
    let value = json!({
        "cx_id": payload.cx.cx_id.to_string(),
        "input_hash": hex(&payload.cx.input_ref.hash),
        "event_time_secs": payload.at.0,
        "dedup_result": payload.result,
        "dedup_decision": payload.decision,
        "dedup_action": payload.action,
        "dedup_into_id": payload.into.map(|id| id.to_string()),
        "merged_from": payload.restore.map(|restore| restore.merged_from.to_string()),
        "occurrence_id": payload.occurrence.map(|id| id.0),
        "recurrence_signature": payload.recurrence_signature.is_some(),
        "same_action": payload.recurrence_signature.map(|signature| {
            signature.same_action.to_string()
        }),
        "new_time": payload.recurrence_signature.map(|signature| signature.new_time.0),
        "per_slot_cos": payload.per_slot_cos.iter().map(|(slot, cos)| {
            json!({"slot": slot.get(), "cos": cos})
        }).collect::<Vec<_>>(),
        "restore": payload.restore,
    });
    serde_json::to_vec(&value)
        .map_err(|error| CalyxError::aster_corrupt_shard(format!("encode ledger payload: {error}")))
}

pub(super) fn action_name(policy: &DedupPolicy) -> Option<&'static str> {
    match policy {
        DedupPolicy::Off => Some("Off"),
        DedupPolicy::Exact => Some("Exact"),
        DedupPolicy::TctCosine(config) => Some(action_name_for_action(&config.action)),
    }
}

pub(super) fn action_name_for_action(action: &DedupAction) -> &'static str {
    match action {
        DedupAction::Collapse => "Collapse",
        DedupAction::Link => "Link",
        DedupAction::RecurrenceSeries => "RecurrenceSeries",
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
