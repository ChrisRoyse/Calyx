use calyx_aster::vault::AsterVault;
use calyx_core::{Anchor, AnchorKind, CxId};
use calyx_ledger::{ActorId, EntryKind, RedactionPolicy, SubjectId};

use super::anchor::anchor_kind_key;
use crate::error::CliResult;

pub(super) fn append_cli_ledger(
    vault: &AsterVault,
    kind: EntryKind,
    cx_id: CxId,
    mode: &'static str,
) -> CliResult<u64> {
    let bytes = serde_json::to_vec(&serde_json::json!({ "mode": mode }))?;
    RedactionPolicy::check_payload(&bytes)?;
    append_ledger_payload(vault, kind, cx_id, bytes)
}

pub(super) fn append_cli_batch_ledger(
    vault: &AsterVault,
    kind: EntryKind,
    cx_ids: &[CxId],
    mode: &'static str,
) -> CliResult<u64> {
    let first = *cx_ids
        .first()
        .ok_or_else(|| crate::error::CliError::usage("batch ledger requires at least one cx_id"))?;
    let cx_ids = cx_ids.iter().map(CxId::to_string).collect::<Vec<_>>();
    let bytes = serde_json::to_vec(&serde_json::json!({
        "mode": mode,
        "count": cx_ids.len(),
        "cx_id": cx_ids,
        "first_cx_id": cx_ids.first(),
        "last_cx_id": cx_ids.last(),
    }))?;
    RedactionPolicy::check_payload(&bytes)?;
    append_ledger_payload(vault, kind, first, bytes)
}

pub(super) fn append_anchor_ledger(
    vault: &AsterVault,
    cx_id: CxId,
    kind: &AnchorKind,
    anchor: Anchor,
) -> CliResult<u64> {
    let bytes = anchor_payload(kind)?;
    Ok(vault
        .anchor_with_ledger_entry(
            cx_id,
            anchor,
            EntryKind::Ingest,
            SubjectId::Cx(cx_id),
            bytes,
            ActorId::Service("calyx-cli".to_string()),
        )?
        .seq)
}

pub(super) fn append_anchor_marker_ledger(
    vault: &AsterVault,
    cx_id: CxId,
    kind: &AnchorKind,
) -> CliResult<u64> {
    let bytes = anchor_payload(kind)?;
    append_ledger_payload(vault, EntryKind::Ingest, cx_id, bytes)
}

fn append_ledger_payload(
    vault: &AsterVault,
    kind: EntryKind,
    cx_id: CxId,
    bytes: Vec<u8>,
) -> CliResult<u64> {
    Ok(vault
        .append_ledger_entry(
            kind,
            SubjectId::Cx(cx_id),
            bytes,
            ActorId::Service("calyx-cli".to_string()),
        )?
        .seq)
}

fn anchor_payload(kind: &AnchorKind) -> CliResult<Vec<u8>> {
    let bytes = serde_json::to_vec(&serde_json::json!({
        "mode": "cli-anchor",
        "anchor_kind": anchor_kind_key(kind),
    }))?;
    RedactionPolicy::check_payload(&bytes)?;
    Ok(bytes)
}
