use serde_json::{Value, json};

use crate::model::Resolution;
use crate::pending_forecast_register::{PendingForecastEntry, PendingForecastWorkItem};

const CHUNK_BYTES: usize = 12;

pub(crate) fn pending_entry_payload(entry: &PendingForecastEntry) -> Value {
    json!({
        "forecast_ref": safe_ref(&entry.forecast_id),
        "source": entry.source,
        "condition_ref": safe_ref(&entry.condition_id),
        "outcome_ref": safe_ref(&entry.token_id),
        "outcome_index": entry.outcome_index,
        "domain": entry.domain,
        "horizon_bucket": entry.horizon_bucket,
        "forecast_version": entry.forecast_version,
        "p_model": entry.p_model,
        "confidence": entry.confidence,
        "forecast_ts": entry.forecast_ts,
        "provenance_hash": entry.provenance_hash,
        "status": entry.status,
        "registered_ledger_seq": entry.registered_ledger_seq,
        "terminal_ledger_seq": entry.terminal_ledger_seq,
        "terminal_resolution_ref": entry.terminal_resolution_id.as_ref().map(|id| safe_ref(id)),
        "terminal_actual_win": entry.terminal_actual_win
    })
}

pub(crate) fn pending_work_item_payload(item: &PendingForecastWorkItem) -> Value {
    json!({
        "forecast_ref": safe_ref(&item.forecast_id),
        "forecast_version": item.forecast_version,
        "condition_ref": safe_ref(&item.condition_id),
        "outcome_ref": safe_ref(&item.token_id),
        "outcome_index": item.outcome_index,
        "p_model": item.p_model,
        "confidence": item.confidence,
        "actual_win": item.actual_win,
        "status": item.status
    })
}

pub(crate) fn resolution_payload(resolution: &Resolution) -> Value {
    json!({
        "condition_ref": safe_ref(&resolution.condition_id),
        "winning_outcome_index": resolution.winning_outcome_index,
        "winning_label": resolution.winning_label,
        "resolved_ts": resolution.resolved_ts,
        "resolution_source": resolution.source,
        "disputed": resolution.disputed
    })
}

pub(crate) fn safe_ref(value: &str) -> Value {
    json!({
        "ref_hash": blake3::hash(value.as_bytes()).to_hex().to_string(),
        "byte_len": value.len(),
        "chunks": chunks(value)
    })
}

fn chunks(value: &str) -> Vec<String> {
    value
        .as_bytes()
        .chunks(CHUNK_BYTES)
        .map(|chunk| String::from_utf8_lossy(chunk).to_string())
        .collect()
}
