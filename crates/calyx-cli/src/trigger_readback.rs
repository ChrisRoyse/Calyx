use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_loom::{
    ReactiveRowKind, decode_audit_entry, decode_trigger_fired, reactive_audit_prefix,
    reactive_row_key,
};
use serde_json::json;

use crate::cf_read::{hex_bytes, latest_cf_rows};
use crate::output::print_json;

pub fn readback_trigger_audit(vault: &Path, sub_id: &str) -> crate::error::CliResult {
    let trigger_id: calyx_loom::TriggerId = sub_id
        .parse()
        .map_err(|error| format!("invalid trigger id: {error}"))?;
    let prefix = reactive_audit_prefix(trigger_id);
    let mut rows = Vec::new();
    for (key, value) in latest_cf_rows(vault, ColumnFamily::Reactive)? {
        if !key.starts_with(&prefix) {
            continue;
        }
        let parts = reactive_row_key(&key).map_err(|error| error.to_string())?;
        let entry = decode_audit_entry(&value).map_err(|error| error.to_string())?;
        rows.push(json!({
            "row_kind": kind_name(parts.kind),
            "key_hex": hex_bytes(&key),
            "value_hex": hex_bytes(&value),
            "trigger_id": parts.trigger_id,
            "ledger_seq": parts.ledger_seq,
            "entry": entry,
        }));
    }
    print_json(&json!({
        "vault": vault.display().to_string(),
        "trigger_id": trigger_id,
        "rows": rows,
    }))
}

pub fn readback_trigger_fired(vault: &Path) -> crate::error::CliResult {
    let mut rows = Vec::new();
    for (key, value) in latest_cf_rows(vault, ColumnFamily::Reactive)? {
        let parts = reactive_row_key(&key).map_err(|error| error.to_string())?;
        if parts.kind != ReactiveRowKind::Fired {
            continue;
        }
        let event = decode_trigger_fired(&value).map_err(|error| error.to_string())?;
        rows.push(json!({
            "row_kind": kind_name(parts.kind),
            "key_hex": hex_bytes(&key),
            "value_hex": hex_bytes(&value),
            "trigger_id": parts.trigger_id,
            "ledger_seq": parts.ledger_seq,
            "event": event,
        }));
    }
    print_json(&json!({
        "vault": vault.display().to_string(),
        "rows": rows,
    }))
}

fn kind_name(kind: ReactiveRowKind) -> &'static str {
    match kind {
        ReactiveRowKind::Audit => "audit",
        ReactiveRowKind::Fired => "fired",
    }
}
