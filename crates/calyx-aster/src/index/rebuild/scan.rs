use calyx_core::{Clock, Result, Seq};

use crate::cf::{ColumnFamily, prefix_range};
use crate::collection::Collection;
use crate::index::{IndexSpec, inverted::InvertedStats};
use crate::mvcc::tombstone_value;
use crate::vault::AsterVault;

use super::data::collect_record_rows_page;
use super::expected::{expected_index_rows_page, index_key_is_stale};
use super::support::{index_cf, index_prefix};
use super::types::IndexRow;

pub(super) fn scan_data_rows<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    col: &Collection,
    spec: &IndexSpec,
    batch_size: usize,
    write_missing: bool,
) -> Result<(u64, u64, bool)> {
    let mut cursor = None;
    let mut missing_count = 0_u64;
    let mut rows_scanned = 0_u64;
    let mut saw_data = false;
    let mut inverted_stats = InvertedStats::default();
    let mut latest_stats_row = None;
    loop {
        let rows = collect_record_rows_page(vault, snapshot, col, cursor.as_deref(), batch_size)?;
        if rows.is_empty() {
            break;
        }
        cursor = rows.last().map(|row| row.data_key.clone());
        rows_scanned += rows.len() as u64;
        saw_data = true;
        let expected = expected_index_rows_page(
            vault,
            col,
            spec,
            &rows,
            &mut inverted_stats,
            &mut latest_stats_row,
        )?;
        missing_count += write_missing_rows(vault, snapshot, &expected, batch_size, write_missing)?;
    }
    if let Some(stats_row) = latest_stats_row {
        missing_count += write_missing_rows(
            vault,
            snapshot,
            std::slice::from_ref(&stats_row),
            batch_size,
            write_missing,
        )?;
    }
    Ok((missing_count, rows_scanned, saw_data))
}

pub(super) fn scan_stale_index_rows<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    col: &Collection,
    spec: &IndexSpec,
    batch_size: usize,
    saw_data: bool,
    write_stale: bool,
) -> Result<u64> {
    let cf = index_cf(spec)?;
    let prefix = index_prefix(col, spec)?;
    let range = prefix_range(&prefix);
    let mut cursor = None;
    let mut stale_count = 0_u64;
    loop {
        let page =
            vault.scan_cf_range_page_at(snapshot, cf, &range, cursor.as_deref(), batch_size)?;
        if page.is_empty() {
            break;
        }
        cursor = page.last().map(|(key, _)| key.clone());
        let mut stale_rows = Vec::new();
        for (key, _value) in page {
            if index_key_is_stale(vault, snapshot, col, spec, &prefix, &key, saw_data)? {
                stale_rows.push((cf, key, tombstone_value()));
            }
        }
        stale_count += stale_rows.len() as u64;
        if write_stale && !stale_rows.is_empty() {
            vault.write_cf_batch(stale_rows)?;
        }
    }
    Ok(stale_count)
}

fn rows_needing_write<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    expected: &[IndexRow],
) -> Result<Vec<IndexRow>> {
    let mut missing = Vec::new();
    for (cf, key, expected_value) in expected {
        match vault.read_cf_at(snapshot, *cf, key)? {
            Some(actual_value) if actual_value == *expected_value => {}
            _ => missing.push((*cf, key.clone(), expected_value.clone())),
        }
    }
    Ok(missing)
}

fn write_missing_rows<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    expected: &[IndexRow],
    batch_size: usize,
    write_missing: bool,
) -> Result<u64> {
    let missing = rows_needing_write(vault, snapshot, expected)?;
    let count = missing.len() as u64;
    if write_missing {
        write_rows_in_batches(vault, missing, batch_size)?;
    }
    Ok(count)
}

fn write_rows_in_batches<C: Clock>(
    vault: &AsterVault<C>,
    rows: impl IntoIterator<Item = (ColumnFamily, Vec<u8>, Vec<u8>)>,
    batch_size: usize,
) -> Result<()> {
    let mut batch = Vec::with_capacity(batch_size);
    for row in rows {
        batch.push(row);
        if batch.len() == batch_size {
            vault.write_cf_batch(batch.drain(..))?;
        }
    }
    if !batch.is_empty() {
        vault.write_cf_batch(batch)?;
    }
    Ok(())
}
