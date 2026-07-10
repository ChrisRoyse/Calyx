//! Ledger-backed authority and orphan recovery for forecast scores.

use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use calyx_core::Clock;
use calyx_ledger::{EntryKind, LedgerAppender, LedgerCfStore};
use serde_json::Value;

use crate::{PolyError, Result};

pub(crate) fn committed_score_ids<S, C>(
    ledger: &LedgerAppender<S, C>,
    schema_version: &str,
) -> Result<BTreeSet<String>>
where
    S: LedgerCfStore,
    C: Clock,
{
    let entries = ledger.scan_entries().map_err(|err| {
        PolyError::score(
            "CALYX_POLY_SCORE_LEDGER_READ_FAILED",
            format!("scan authoritative forecast score ledger: {err}"),
        )
    })?;
    let mut score_ids = BTreeSet::new();
    for entry in entries {
        if entry.kind != EntryKind::Score {
            continue;
        }
        let payload: Value = serde_json::from_slice(&entry.payload).map_err(|err| {
            PolyError::score(
                "CALYX_POLY_SCORE_LEDGER_PAYLOAD_INVALID",
                format!("decode score ledger payload at seq {}: {err}", entry.seq),
            )
        })?;
        let row_schema = required_string(&payload, "schema_version", entry.seq)?;
        if row_schema != schema_version {
            continue;
        }
        let score_id = required_string(&payload, "score_id", entry.seq)?;
        if !score_ids.insert(score_id.to_string()) {
            return Err(PolyError::score(
                "CALYX_POLY_SCORE_LEDGER_DUPLICATE",
                format!("score ledger contains duplicate committed score_id {score_id}"),
            ));
        }
    }
    Ok(score_ids)
}

pub(crate) fn clear_uncommitted_score_diagnostics(
    final_dir: &Path,
    staging_dir: &Path,
) -> Result<()> {
    for path in [final_dir, staging_dir] {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == ErrorKind::NotFound => continue,
            Err(err) => return Err(orphan_cleanup_error(path, err)),
        };
        let result = if metadata.file_type().is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        };
        result.map_err(|err| orphan_cleanup_error(path, err))?;
    }
    Ok(())
}

fn required_string<'a>(payload: &'a Value, field: &str, seq: u64) -> Result<&'a str> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            PolyError::score(
                "CALYX_POLY_SCORE_LEDGER_PAYLOAD_INVALID",
                format!("score ledger payload at seq {seq} lacks string field {field}"),
            )
        })
}

fn orphan_cleanup_error(path: &Path, err: std::io::Error) -> PolyError {
    PolyError::score(
        "CALYX_POLY_SCORE_ORPHAN_CLEANUP_FAILED",
        format!(
            "remove uncommitted score diagnostic {} before ledger retry: {err}",
            path.display()
        ),
    )
}
