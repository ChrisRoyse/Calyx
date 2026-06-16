use calyx_core::{Constellation, SlotVector};
use calyx_sextant::TraverseDirection;
use serde_json::{Value, json};

pub(super) fn definition(cx: Constellation) -> Value {
    json!({
        "cx_id": cx.cx_id.to_string(),
        "slots": cx.slots.into_iter().map(|(slot, vector)| json!({
            "slot": slot.get(),
            "vector": vector_json(vector),
        })).collect::<Vec<_>>(),
    })
}

pub(super) fn empty_definition(lens: u16, index: usize) -> Value {
    json!({
        "lens": lens,
        "index": index,
        "cx_id": Value::Null,
        "slots": [],
    })
}

pub(super) fn direction_key(direction: TraverseDirection) -> &'static str {
    match direction {
        TraverseDirection::Forward => "forward",
        TraverseDirection::Backward => "backward",
        TraverseDirection::Both => "both",
    }
}

fn vector_json(vector: SlotVector) -> Value {
    match vector {
        SlotVector::Dense { dim, data } => json!({ "kind": "dense", "dim": dim, "values": data }),
        SlotVector::Sparse { dim, entries } => {
            json!({ "kind": "sparse", "dim": dim, "entries": entries })
        }
        SlotVector::Multi { token_dim, tokens } => {
            json!({ "kind": "multi", "token_dim": token_dim, "tokens": tokens })
        }
        SlotVector::Absent { reason } => json!({ "kind": "absent", "reason": reason }),
    }
}
