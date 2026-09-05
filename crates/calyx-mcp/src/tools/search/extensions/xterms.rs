use std::collections::BTreeMap;

use calyx_aster::cf::{ColumnFamily, XTermKind, xterm_key};
use calyx_aster::plain_graph::GraphCollectionLifecycle;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Constellation, CxId, VaultStore};
use calyx_ledger::{ActorId, EntryKind, RedactionPolicy, SubjectId};
use calyx_lodestar::PANEL_ASTER_ASSOC_COLLECTION;
use calyx_loom::agreement_graph::XtermRow;
use calyx_loom::{
    CrossTermKey, CrossTermKind, CrossTermValue, SignalProvenanceTag, agreement_scalar,
};
use serde_json::json;

use crate::server::ToolResult;

pub(super) fn materialize_agreement_xterms(
    vault: &AsterVault,
    docs: &BTreeMap<CxId, Constellation>,
) -> ToolResult<usize> {
    let mut rows = Vec::new();
    let snapshot = vault.snapshot();
    let lifecycle = GraphCollectionLifecycle::new(vault)?;
    for cx in docs.values() {
        let dense_slots = cx
            .slots
            .iter()
            .filter_map(|(slot, vector)| vector.as_dense().map(|values| (*slot, values)))
            .collect::<Vec<_>>();
        for left in 0..dense_slots.len() {
            for right in (left + 1)..dense_slots.len() {
                let (a, av) = dense_slots[left];
                let (b, bv) = dense_slots[right];
                let key = xterm_key(cx.cx_id, a, b, XTermKind::Agreement);
                let stored_key = lifecycle.resolve_weave_xterm_key(
                    snapshot,
                    PANEL_ASTER_ASSOC_COLLECTION,
                    &key,
                )?;
                if vault
                    .read_cf_at(snapshot, ColumnFamily::XTerm, &stored_key)?
                    .is_some()
                {
                    continue;
                }
                if stored_key != key {
                    return Err(CalyxError::aster_corrupt_shard(format!(
                        "accepted weave generation is missing XTerm cx={} slots=({}, {}) kind=agreement",
                        cx.cx_id, a, b
                    ))
                    .into());
                }
                let row = XtermRow {
                    key: CrossTermKey {
                        cx_id: cx.cx_id,
                        a,
                        b,
                        kind: CrossTermKind::Agreement,
                    },
                    value: CrossTermValue::Scalar(agreement_scalar(av, bv)?),
                    tag: SignalProvenanceTag::Derived,
                };
                let value = serde_json::to_vec(&row).map_err(|err| {
                    CalyxError::aster_corrupt_shard(format!("encode xterm row: {err}"))
                })?;
                rows.push((ColumnFamily::XTerm, key, value));
            }
        }
    }
    if rows.is_empty() {
        return Ok(0);
    }
    let count = rows.len();
    let payload = serde_json::to_vec(&json!({
        "mode": "mcp-agreement-xterms",
        "rows": count,
    }))
    .map_err(|err| CalyxError::aster_corrupt_shard(format!("encode xterm ledger: {err}")))?;
    RedactionPolicy::check_payload(&payload)?;
    vault.write_cf_batch_with_ledger_entry(
        rows,
        EntryKind::Measure,
        SubjectId::Query(b"mcp-nav-agreement".to_vec()),
        payload,
        ActorId::Service("calyx-mcp".to_string()),
    )?;
    vault.flush()?;
    Ok(count)
}
