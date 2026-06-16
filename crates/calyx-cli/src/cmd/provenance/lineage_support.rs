use calyx_core::{CalyxError, CxId};
use calyx_ledger::{EntryKind, LedgerEntry, SubjectId};
use serde_json::Value;

use crate::error::CliResult;

pub(super) fn primary_ingest_entry<'a>(
    cx_id: CxId,
    current: &'a LedgerEntry,
    entries: &'a [LedgerEntry],
) -> CliResult<&'a LedgerEntry> {
    if !is_cli_anchor(current) {
        return Ok(current);
    }
    entries
        .iter()
        .find(|entry| {
            entry.seq < current.seq
                && entry.kind == EntryKind::Ingest
                && matches!(entry.subject, SubjectId::Cx(id) if id == cx_id)
                && !is_cli_anchor(entry)
        })
        .ok_or_else(|| {
            CalyxError::ledger_corrupt(format!(
                "missing primary ingest ledger row before anchor seq {} for {cx_id}",
                current.seq
            ))
            .into()
        })
}

fn is_cli_anchor(entry: &LedgerEntry) -> bool {
    match serde_json::from_slice::<Value>(&entry.payload) {
        Ok(payload) => payload.get("mode").and_then(Value::as_str) == Some("cli-anchor"),
        Err(_) => false,
    }
}
