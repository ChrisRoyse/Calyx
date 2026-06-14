//! Secondary-index verification and self-heal rebuild (PH54 T05).

mod data;
mod expected;
mod scan;
mod support;
mod types;

use std::time::Instant;

use calyx_core::{Clock, Result};

use super::IndexSpec;
use crate::collection::Collection;
use crate::vault::AsterVault;

use self::scan::{scan_data_rows, scan_stale_index_rows};
use self::support::{effective_batch_size, is_active_spec, require_records_collection};
use self::types::{DEFAULT_BATCH_SIZE, elapsed_ms};

pub use self::types::{IndexHealth, RebuildStats};

pub fn index_verify<C: Clock>(
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
) -> Result<IndexHealth> {
    if !is_active_spec(col, spec)? {
        return Ok(IndexHealth {
            healthy: true,
            ..IndexHealth::default()
        });
    }
    require_records_collection(col)?;
    let snapshot = vault.latest_seq();
    let (missing, _rows_scanned, saw_data) =
        scan_data_rows(vault, snapshot, col, spec, DEFAULT_BATCH_SIZE, false)?;
    let stale = scan_stale_index_rows(
        vault,
        snapshot,
        col,
        spec,
        DEFAULT_BATCH_SIZE,
        saw_data,
        false,
    )?;
    Ok(IndexHealth {
        missing,
        stale,
        healthy: missing == 0 && stale == 0,
    })
}

pub fn index_rebuild<C: Clock>(
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
    batch_size: usize,
) -> Result<RebuildStats> {
    let batch_size = effective_batch_size(batch_size)?;
    if !is_active_spec(col, spec)? {
        return Ok(RebuildStats::default());
    }
    require_records_collection(col)?;
    let started = Instant::now();
    let snapshot = vault.latest_seq();
    let (keys_added, rows_scanned, saw_data) =
        scan_data_rows(vault, snapshot, col, spec, batch_size, true)?;
    let stale_removed =
        scan_stale_index_rows(vault, snapshot, col, spec, batch_size, saw_data, true)?;
    Ok(RebuildStats {
        rows_scanned,
        keys_added,
        stale_removed,
        elapsed_ms: elapsed_ms(started),
    })
}
