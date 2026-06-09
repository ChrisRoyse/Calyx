use calyx_core::CxId;
use serde_json::Value;

use crate::entry::{LedgerEntry, SubjectId};

pub(super) fn entry_mentions_cx(entry: &LedgerEntry, cx_id: CxId) -> bool {
    if entry.subject == SubjectId::Cx(cx_id) {
        return true;
    }
    serde_json::from_slice::<Value>(&entry.payload)
        .ok()
        .is_some_and(|payload| value_mentions_cx(&payload, &cx_id.to_string()))
}

fn value_mentions_cx(value: &Value, needle: &str) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            if is_cx_payload_field(key) {
                value_contains_cx(value, needle)
            } else {
                matches!(value, Value::Object(_) | Value::Array(_))
                    && value_mentions_cx(value, needle)
            }
        }),
        Value::Array(values) => values
            .iter()
            .any(|value| matches!(value, Value::Object(_)) && value_mentions_cx(value, needle)),
        _ => false,
    }
}

fn value_contains_cx(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(value) => value == needle,
        Value::Array(values) => values.iter().any(|value| value_contains_cx(value, needle)),
        Value::Object(_) => value_mentions_cx(value, needle),
        _ => false,
    }
}

fn is_cx_payload_field(key: &str) -> bool {
    matches!(
        key,
        "cx_id"
            | "from_id"
            | "to_id"
            | "source_cx_id"
            | "target_cx_id"
            | "nearest_cx"
            | "matched_cx_id"
            | "query_id"
            | "anchor_kernel_node_id"
    )
}
