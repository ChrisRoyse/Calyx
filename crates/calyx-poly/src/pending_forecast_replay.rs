//! Rebuilds the pending-forecast register from Aster Ledger CF rows (issue #240).

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::Clock;
use calyx_ledger::{EntryKind, decode as decode_ledger};
use serde_json::Value;

use crate::error::{PolyError, Result};
use crate::model::Resolution;
use crate::pending_forecast_register::{
    PENDING_FORECAST_REGISTERED_EVENT, PENDING_FORECAST_RESOLUTION_JOIN_EVENT,
    PENDING_FORECAST_SCHEMA_VERSION, PendingForecastEntry, PendingForecastRegister,
    PendingForecastStatus, PendingForecastWorkItem,
};
use crate::score::ForecastSource;

pub const ERR_PENDING_FORECAST_REPLAY_READ: &str = "CALYX_POLY_PENDING_FORECAST_REPLAY_READ";
pub const ERR_PENDING_FORECAST_REPLAY_PAYLOAD: &str = "CALYX_POLY_PENDING_FORECAST_REPLAY_PAYLOAD";
pub const ERR_PENDING_FORECAST_REPLAY_REF: &str = "CALYX_POLY_PENDING_FORECAST_REPLAY_REF";
pub const ERR_PENDING_FORECAST_REPLAY_DUPLICATE: &str =
    "CALYX_POLY_PENDING_FORECAST_REPLAY_DUPLICATE";
pub const ERR_PENDING_FORECAST_REPLAY_INCONSISTENT: &str =
    "CALYX_POLY_PENDING_FORECAST_REPLAY_INCONSISTENT";

pub fn replay_pending_forecast_register_from_vault<C>(
    vault: &AsterVault<C>,
) -> Result<PendingForecastRegister>
where
    C: Clock,
{
    let snapshot = vault.latest_seq();
    let mut rows = vault
        .scan_cf_at(snapshot, ColumnFamily::Ledger)
        .map_err(|err| read_err(format!("scan Ledger CF at snapshot {snapshot}: {err}")))?
        .into_iter()
        .map(|(key, bytes)| Ok((parse_ledger_seq(&key)?, bytes)))
        .collect::<Result<Vec<_>>>()?;
    rows.sort_by_key(|(seq, _)| *seq);

    let mut register = PendingForecastRegister::default();
    for (seq, bytes) in rows {
        let ledger = decode_ledger(&bytes)
            .map_err(|err| read_err(format!("decode Ledger CF row {seq}: {err}")))?;
        if ledger.seq != seq {
            return Err(read_err(format!(
                "Ledger CF key seq {seq} does not match decoded ledger seq {}",
                ledger.seq
            )));
        }
        if ledger.kind != EntryKind::Measure {
            continue;
        }
        apply_ledger_payload(&mut register, seq, &ledger.payload)?;
    }
    Ok(register)
}

pub fn decode_pending_safe_ref_for_replay(value: &Value) -> Result<String> {
    decode_safe_ref(value, "safe_ref")
}

fn apply_ledger_payload(
    register: &mut PendingForecastRegister,
    seq: u64,
    payload: &[u8],
) -> Result<()> {
    let value: Value = serde_json::from_slice(payload).map_err(|err| {
        payload_err(format!(
            "decode pending-forecast payload at seq {seq}: {err}"
        ))
    })?;
    match value.get("schema_version").and_then(Value::as_str) {
        Some(PENDING_FORECAST_SCHEMA_VERSION) => {}
        _ => return Ok(()),
    }
    match required_str(&value, "event")? {
        PENDING_FORECAST_REGISTERED_EVENT => apply_registered(register, seq, &value),
        PENDING_FORECAST_RESOLUTION_JOIN_EVENT => apply_resolution_join(register, seq, &value),
        event => payload_fail(format!(
            "unknown pending-forecast ledger event {event:?} at seq {seq}"
        )),
    }
}

fn apply_registered(register: &mut PendingForecastRegister, seq: u64, value: &Value) -> Result<()> {
    let forecast = required(value, "forecast")?;
    let status = parse_status(required(forecast, "status")?)?;
    if status != PendingForecastStatus::Pending {
        return inconsistent(format!(
            "registered forecast at seq {seq} has non-pending status {status:?}"
        ));
    }
    let entry = PendingForecastEntry {
        forecast_id: decode_ref(forecast, "forecast_ref")?,
        source: parse_source(required(forecast, "source")?)?,
        condition_id: decode_ref(forecast, "condition_ref")?,
        token_id: decode_ref(forecast, "outcome_ref")?,
        outcome_index: required_u32(forecast, "outcome_index")?,
        domain: required_str(forecast, "domain")?.to_string(),
        horizon_bucket: required_str(forecast, "horizon_bucket")?.to_string(),
        forecast_version: required_u32(forecast, "forecast_version")?,
        p_model: required_f64(forecast, "p_model")?,
        confidence: required_f64(forecast, "confidence")?,
        forecast_ts: required_u64(forecast, "forecast_ts")?,
        provenance_hash: required_str(forecast, "provenance_hash")?.to_string(),
        status,
        registered_ledger_seq: Some(seq),
        terminal_ledger_seq: None,
        terminal_resolution_id: None,
        terminal_actual_win: None,
    };
    validate_entry_for_replay(&entry)?;
    if find_entry(register, &entry.forecast_id).is_some() {
        return duplicate(format!(
            "duplicate pending forecast id {} at seq {seq}",
            entry.forecast_id
        ));
    }
    register.entries.push(entry);
    Ok(())
}

fn apply_resolution_join(
    register: &mut PendingForecastRegister,
    seq: u64,
    value: &Value,
) -> Result<()> {
    let resolution_id = decode_ref(value, "resolution_ref")?;
    let resolution = parse_resolution(required(value, "resolution")?)?;
    let voided = required_bool(value, "voided")?;
    let work_items = required_array(value, "work_items")?
        .iter()
        .map(parse_work_item)
        .collect::<Result<Vec<_>>>()?;
    verify_ref_array_matches_items(value, "selected_forecast_refs", &work_items)?;
    verify_ref_array_matches_items(value, "transitioned_forecast_refs", &work_items)?;
    verify_blocked_refs(register, value, &resolution)?;

    for item in work_items {
        apply_work_item(register, seq, &resolution_id, voided, item)?;
    }
    Ok(())
}

fn apply_work_item(
    register: &mut PendingForecastRegister,
    seq: u64,
    resolution_id: &str,
    voided: bool,
    item: PendingForecastWorkItem,
) -> Result<()> {
    let entry = find_entry_mut(register, &item.forecast_id).ok_or_else(|| {
        PolyError::diagnostics(
            ERR_PENDING_FORECAST_REPLAY_INCONSISTENT,
            format!(
                "resolution join seq {seq} references unknown forecast {}",
                item.forecast_id
            ),
        )
    })?;
    verify_item_matches_entry(entry, &item, seq)?;
    if entry.status != PendingForecastStatus::Pending {
        return inconsistent(format!(
            "resolution join seq {seq} would transition already-terminal forecast {}",
            item.forecast_id
        ));
    }
    match (voided, item.status, item.actual_win) {
        (true, PendingForecastStatus::Void, None) => {}
        (false, PendingForecastStatus::Scored, Some(_)) => {}
        _ => {
            return inconsistent(format!(
                "resolution join seq {seq} has invalid status/actual_win for forecast {}",
                item.forecast_id
            ));
        }
    }
    entry.status = item.status;
    entry.terminal_ledger_seq = Some(seq);
    entry.terminal_resolution_id = Some(resolution_id.to_string());
    entry.terminal_actual_win = item.actual_win;
    Ok(())
}

fn parse_work_item(value: &Value) -> Result<PendingForecastWorkItem> {
    Ok(PendingForecastWorkItem {
        forecast_id: decode_ref(value, "forecast_ref")?,
        forecast_version: required_u32(value, "forecast_version")?,
        condition_id: decode_ref(value, "condition_ref")?,
        token_id: decode_ref(value, "outcome_ref")?,
        outcome_index: required_u32(value, "outcome_index")?,
        p_model: required_f64(value, "p_model")?,
        confidence: required_f64(value, "confidence")?,
        actual_win: optional_bool(value, "actual_win")?,
        status: parse_status(required(value, "status")?)?,
    })
}

fn parse_resolution(value: &Value) -> Result<Resolution> {
    Ok(Resolution {
        condition_id: decode_ref(value, "condition_ref")?,
        winning_outcome_index: required_u32(value, "winning_outcome_index")?,
        winning_label: required_str(value, "winning_label")?.to_string(),
        resolved_ts: required_u64(value, "resolved_ts")?,
        source: required_str(value, "resolution_source")?.to_string(),
        disputed: required_bool(value, "disputed")?,
    })
}

fn verify_ref_array_matches_items(
    value: &Value,
    field: &str,
    items: &[PendingForecastWorkItem],
) -> Result<()> {
    let refs = required_array(value, field)?
        .iter()
        .map(|item| decode_safe_ref(item, field))
        .collect::<Result<Vec<_>>>()?;
    let item_ids = items
        .iter()
        .map(|item| item.forecast_id.clone())
        .collect::<Vec<_>>();
    if refs == item_ids {
        return Ok(());
    }
    inconsistent(format!("{field} does not match work_items forecast refs"))
}

fn verify_blocked_refs(
    register: &PendingForecastRegister,
    value: &Value,
    resolution: &Resolution,
) -> Result<()> {
    for blocked in required_array(value, "lookahead_blocked_forecast_refs")? {
        let forecast_id = decode_safe_ref(blocked, "lookahead_blocked_forecast_refs")?;
        let entry = find_entry(register, &forecast_id).ok_or_else(|| {
            PolyError::diagnostics(
                ERR_PENDING_FORECAST_REPLAY_INCONSISTENT,
                format!("blocked forecast {forecast_id} is not registered"),
            )
        })?;
        if entry.condition_id != resolution.condition_id
            || entry.status != PendingForecastStatus::Pending
            || entry.forecast_ts < resolution.resolved_ts
        {
            return inconsistent(format!(
                "blocked forecast {forecast_id} is not a pending look-ahead forecast"
            ));
        }
    }
    Ok(())
}

fn verify_item_matches_entry(
    entry: &PendingForecastEntry,
    item: &PendingForecastWorkItem,
    seq: u64,
) -> Result<()> {
    if entry.forecast_version == item.forecast_version
        && entry.condition_id == item.condition_id
        && entry.token_id == item.token_id
        && entry.outcome_index == item.outcome_index
        && entry.p_model.to_bits() == item.p_model.to_bits()
        && entry.confidence.to_bits() == item.confidence.to_bits()
    {
        return Ok(());
    }
    inconsistent(format!(
        "resolution join seq {seq} work item no longer matches registered forecast {}",
        item.forecast_id
    ))
}

fn decode_ref(value: &Value, field: &str) -> Result<String> {
    decode_safe_ref(required(value, field)?, field)
}

fn decode_safe_ref(value: &Value, label: &str) -> Result<String> {
    let byte_len = required_u64(value, "byte_len")? as usize;
    let expected_hash = required_str(value, "ref_hash")?;
    let chunks = required_array(value, "chunks")?;
    let mut out = String::new();
    for chunk in chunks {
        out.push_str(chunk.as_str().ok_or_else(|| {
            PolyError::diagnostics(
                ERR_PENDING_FORECAST_REPLAY_REF,
                format!("{label} contains a non-string chunk"),
            )
        })?);
    }
    if out.len() != byte_len {
        return ref_fail(format!(
            "{label} byte_len mismatch: stored {byte_len}, reconstructed {}",
            out.len()
        ));
    }
    let actual_hash = blake3::hash(out.as_bytes()).to_hex().to_string();
    if actual_hash != expected_hash {
        return ref_fail(format!(
            "{label} hash mismatch: stored {expected_hash}, reconstructed {actual_hash}"
        ));
    }
    Ok(out)
}

fn validate_entry_for_replay(entry: &PendingForecastEntry) -> Result<()> {
    let labels = [
        &entry.forecast_id,
        &entry.condition_id,
        &entry.token_id,
        &entry.domain,
        &entry.horizon_bucket,
    ];
    if labels.iter().any(|value| value.trim().is_empty()) || entry.forecast_version == 0 {
        return inconsistent("replayed pending forecast has missing identity fields");
    }
    if !unit(entry.p_model)
        || !entry.confidence.is_finite()
        || !(0.0..1.0).contains(&entry.confidence)
    {
        return inconsistent("replayed pending forecast has invalid p_model or confidence");
    }
    if entry.provenance_hash.len() != 64
        || !entry
            .provenance_hash
            .chars()
            .all(|ch| ch.is_ascii_hexdigit())
    {
        return inconsistent("replayed pending forecast has invalid provenance_hash");
    }
    Ok(())
}

fn parse_source(value: &Value) -> Result<ForecastSource> {
    serde_json::from_value(value.clone())
        .map_err(|err| payload_err(format!("decode forecast source: {err}")))
}

fn parse_status(value: &Value) -> Result<PendingForecastStatus> {
    serde_json::from_value(value.clone())
        .map_err(|err| payload_err(format!("decode pending forecast status: {err}")))
}

fn find_entry<'a>(
    register: &'a PendingForecastRegister,
    forecast_id: &str,
) -> Option<&'a PendingForecastEntry> {
    register
        .entries
        .iter()
        .find(|entry| entry.forecast_id == forecast_id)
}

fn find_entry_mut<'a>(
    register: &'a mut PendingForecastRegister,
    forecast_id: &str,
) -> Option<&'a mut PendingForecastEntry> {
    register
        .entries
        .iter_mut()
        .find(|entry| entry.forecast_id == forecast_id)
}

fn parse_ledger_seq(key: &[u8]) -> Result<u64> {
    let bytes: [u8; 8] = key
        .try_into()
        .map_err(|_| read_err(format!("Ledger CF key has {} bytes, expected 8", key.len())))?;
    Ok(u64::from_be_bytes(bytes))
}

fn required<'a>(value: &'a Value, field: &str) -> Result<&'a Value> {
    value.get(field).ok_or_else(|| {
        PolyError::diagnostics(
            ERR_PENDING_FORECAST_REPLAY_PAYLOAD,
            format!("missing {field}"),
        )
    })
}

fn required_array<'a>(value: &'a Value, field: &str) -> Result<&'a [Value]> {
    required(value, field)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| {
            PolyError::diagnostics(
                ERR_PENDING_FORECAST_REPLAY_PAYLOAD,
                format!("{field} must be an array"),
            )
        })
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    required(value, field)?.as_str().ok_or_else(|| {
        PolyError::diagnostics(
            ERR_PENDING_FORECAST_REPLAY_PAYLOAD,
            format!("{field} must be a string"),
        )
    })
}

fn required_u64(value: &Value, field: &str) -> Result<u64> {
    required(value, field)?.as_u64().ok_or_else(|| {
        PolyError::diagnostics(
            ERR_PENDING_FORECAST_REPLAY_PAYLOAD,
            format!("{field} must be a u64"),
        )
    })
}

fn required_u32(value: &Value, field: &str) -> Result<u32> {
    let raw = required_u64(value, field)?;
    u32::try_from(raw).map_err(|_| {
        PolyError::diagnostics(
            ERR_PENDING_FORECAST_REPLAY_PAYLOAD,
            format!("{field} value {raw} exceeds u32"),
        )
    })
}

fn required_f64(value: &Value, field: &str) -> Result<f64> {
    required(value, field)?.as_f64().ok_or_else(|| {
        PolyError::diagnostics(
            ERR_PENDING_FORECAST_REPLAY_PAYLOAD,
            format!("{field} must be an f64"),
        )
    })
}

fn required_bool(value: &Value, field: &str) -> Result<bool> {
    required(value, field)?.as_bool().ok_or_else(|| {
        PolyError::diagnostics(
            ERR_PENDING_FORECAST_REPLAY_PAYLOAD,
            format!("{field} must be a bool"),
        )
    })
}

fn optional_bool(value: &Value, field: &str) -> Result<Option<bool>> {
    match required(value, field)? {
        Value::Null => Ok(None),
        other => other.as_bool().map(Some).ok_or_else(|| {
            PolyError::diagnostics(
                ERR_PENDING_FORECAST_REPLAY_PAYLOAD,
                format!("{field} must be a bool or null"),
            )
        }),
    }
}

fn unit(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn read_err(message: impl Into<String>) -> PolyError {
    PolyError::diagnostics(ERR_PENDING_FORECAST_REPLAY_READ, message.into())
}

fn payload_err(message: impl Into<String>) -> PolyError {
    PolyError::diagnostics(ERR_PENDING_FORECAST_REPLAY_PAYLOAD, message.into())
}

fn payload_fail<T>(message: impl Into<String>) -> Result<T> {
    Err(payload_err(message))
}

fn ref_fail<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_PENDING_FORECAST_REPLAY_REF,
        message.into(),
    ))
}

fn duplicate<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_PENDING_FORECAST_REPLAY_DUPLICATE,
        message.into(),
    ))
}

fn inconsistent<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_PENDING_FORECAST_REPLAY_INCONSISTENT,
        message.into(),
    ))
}
