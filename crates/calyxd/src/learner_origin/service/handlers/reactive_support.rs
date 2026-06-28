use calyx_core::LedgerRef;
use calyx_loom::{ReactiveEngine, SubscriptionId, TriggerFired};
use calyx_ward::NoveltyAction;
use serde::Serialize;
use serde_json::{Value, json};

use super::super::{OriginError, STATUS_UNPROCESSABLE, hex};
use super::reactive_plan::ReactiveMmdReadback;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReactiveIntervention {
    kind: &'static str,
    pub(super) reason: &'static str,
    confidence: f64,
}

pub(super) fn reactive_interventions(
    new_region_events: &[TriggerFired],
    drift_events: &[TriggerFired],
    mmd: &ReactiveMmdReadback,
    novelty_action: Option<&NoveltyAction>,
) -> Vec<ReactiveIntervention> {
    let mut out = Vec::new();
    if !new_region_events.is_empty() || novelty_action == Some(&NoveltyAction::NewRegion) {
        out.push(ReactiveIntervention {
            kind: "hint",
            reason: "new_region",
            confidence: 0.9,
        });
    }
    if !drift_events.is_empty() {
        out.push(ReactiveIntervention {
            kind: "review",
            reason: "agreement_drift",
            confidence: 0.9,
        });
    }
    if mmd.drift.significant || mmd.change_point.report.significant {
        out.push(ReactiveIntervention {
            kind: "review",
            reason: "mmd_distribution_shift",
            confidence: (1.0 - mmd.drift.p_value).clamp(0.0, 1.0),
        });
    }
    out
}

pub(super) fn trigger_events_json(events: &[TriggerFired]) -> Vec<Value> {
    events
        .iter()
        .map(|event| {
            json!({
                "triggerId": event.trigger_id.to_string(),
                "cxId": event.cx_id.to_string(),
                "firedAt": event.fired_at,
                "ledgerSeq": event.ledger_ref.seq,
                "ledgerHash": hex(&event.ledger_ref.hash),
                "condition": event.condition_snapshot
            })
        })
        .collect()
}

pub(super) fn trigger_id_for_subscription(
    engine: &ReactiveEngine,
    subscription_id: SubscriptionId,
) -> Result<String, OriginError> {
    engine
        .subscriptions()
        .get(subscription_id)
        .map(|handle| handle.trigger_id.to_string())
        .ok_or_else(|| OriginError::internal("reactive subscription missing after creation"))
}

pub(super) fn stored_ledger_ref(seq: u64, hash_hex: &str) -> Result<LedgerRef, OriginError> {
    Ok(LedgerRef {
        seq,
        hash: parse_hex_32(hash_hex)?,
    })
}

fn parse_hex_32(value: &str) -> Result<[u8; 32], OriginError> {
    if value.len() != 64 {
        return Err(OriginError::internal("stored ledger hash is not 32 bytes"));
    }
    let mut out = [0_u8; 32];
    for index in 0..32 {
        out[index] = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| OriginError::internal("stored ledger hash is not valid hex"))?;
    }
    Ok(out)
}

pub(super) fn vector_bytes(vector: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        out.extend_from_slice(&value.to_bits().to_be_bytes());
    }
    out
}

pub(super) fn reactive_vector(field: &str, values: &[f32]) -> Result<Vec<f32>, OriginError> {
    if values.is_empty() {
        return Err(OriginError::bad_request(
            "CALYX_ORIGIN_FIELD_REQUIRED",
            format!("{field} must contain at least one value"),
        ));
    }
    if values.len() > 4096 {
        return Err(OriginError::bad_request(
            "CALYX_ORIGIN_VECTOR_TOO_LARGE",
            format!("{field} accepts at most 4096 values"),
        ));
    }
    for (index, value) in values.iter().enumerate() {
        if !value.is_finite() {
            return Err(OriginError::bad_request(
                "CALYX_ORIGIN_INVALID_NUMBER",
                format!("{field}[{index}] must be finite"),
            ));
        }
    }
    Ok(values.to_vec())
}

pub(super) fn novelty_action_name(action: NoveltyAction) -> &'static str {
    match action {
        NoveltyAction::NewRegion => "new_region",
        NoveltyAction::Quarantine => "quarantine",
        NoveltyAction::RejectClosed => "reject_closed",
    }
}

pub(super) fn mmd_origin_error(error: calyx_core::CalyxError) -> OriginError {
    OriginError::new(
        STATUS_UNPROCESSABLE,
        "CALYX_ORIGIN_MMD_REJECTED",
        format!("{}: {}", error.code, error.message),
    )
}

pub(super) fn reactive_origin_error(error: calyx_core::CalyxError) -> OriginError {
    OriginError::new(
        STATUS_UNPROCESSABLE,
        "CALYX_ORIGIN_REACTIVE_REJECTED",
        format!("{}: {}", error.code, error.message),
    )
}

pub(super) fn ward_origin_error(error: calyx_ward::WardError) -> OriginError {
    OriginError::new(
        STATUS_UNPROCESSABLE,
        "CALYX_ORIGIN_WARD_REJECTED",
        format!("{}: {}", error.code(), error),
    )
}
