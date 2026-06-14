use calyx_core::{Clock, Result, Seq};

use crate::cf::ColumnFamily;
use crate::collection::Collection;
use crate::index::{
    IndexKind, IndexMaintenance, IndexSpec, SecondaryIndex, inverted::InvertedStats,
};
use crate::vault::AsterVault;

use super::data::read_record_row;
use super::support::{
    btree_index, indexed_value, invalid_argument, inverted_index, is_inverted_stats_key,
};
use super::types::{IndexRow, RecordDataRow};

pub(super) fn index_key_is_stale<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    col: &Collection,
    spec: &IndexSpec,
    prefix: &[u8],
    key: &[u8],
    saw_data: bool,
) -> Result<bool> {
    validate_index_key(col, spec, prefix, key)?;
    match spec.kind {
        IndexKind::Btree => btree_key_is_stale(vault, snapshot, col, spec, key),
        IndexKind::Inverted => {
            inverted_key_is_stale(vault, snapshot, col, spec, prefix, key, saw_data)
        }
        _ => Err(invalid_argument(
            "index rebuild supports btree/inverted specs",
        )),
    }
}

pub(super) fn expected_index_rows_page<C: Clock>(
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
    rows: &[RecordDataRow],
    inverted_stats: &mut InvertedStats,
    latest_stats_row: &mut Option<IndexRow>,
) -> Result<Vec<IndexRow>> {
    match spec.kind {
        IndexKind::Btree => expected_btree_rows(vault, col, spec, rows),
        IndexKind::Inverted => {
            expected_inverted_rows(col, spec, rows, inverted_stats, latest_stats_row)
        }
        _ => Err(invalid_argument(
            "index rebuild supports btree/inverted specs",
        )),
    }
}

fn btree_key_is_stale<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    col: &Collection,
    spec: &IndexSpec,
    key: &[u8],
) -> Result<bool> {
    let idx = btree_index(col, spec)?;
    let (_field, pk) = idx.decode_index_key(key)?;
    let Some(row) = read_record_row(vault, snapshot, col, &pk)? else {
        return Ok(true);
    };
    let field = indexed_value(&row, &spec.on_field)?;
    Ok(idx.encode_index_key(field, &pk)? != key)
}

fn inverted_key_is_stale<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    col: &Collection,
    spec: &IndexSpec,
    prefix: &[u8],
    key: &[u8],
    saw_data: bool,
) -> Result<bool> {
    if is_inverted_stats_key(prefix, key) {
        return Ok(!saw_data);
    }
    let idx = inverted_index(col, spec)?;
    let (_hash, pk) = idx.decode_posting_key(key)?;
    let Some(row) = read_record_row(vault, snapshot, col, &pk)? else {
        return Ok(true);
    };
    let field = indexed_value(&row, &spec.on_field)?;
    Ok(!idx
        .encode_put_entries(field, &pk, InvertedStats::default())?
        .into_iter()
        .any(|(expected_key, _)| expected_key == key))
}

fn expected_btree_rows<C: Clock>(
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
    rows: &[RecordDataRow],
) -> Result<Vec<IndexRow>> {
    let maintenance = IndexMaintenance {
        indexes: vec![(
            spec.clone(),
            Box::new(btree_index(col, spec)?) as Box<dyn SecondaryIndex>,
        )],
    };
    let mut expected = Vec::new();
    for data_row in rows {
        maintenance.on_put(vault, &mut expected, col, &data_row.pk, &data_row.row)?;
    }
    expected.retain(|(cf, _, _)| *cf == ColumnFamily::IndexBtree);
    Ok(expected)
}

fn expected_inverted_rows(
    col: &Collection,
    spec: &IndexSpec,
    rows: &[RecordDataRow],
    stats: &mut InvertedStats,
    latest_stats_row: &mut Option<IndexRow>,
) -> Result<Vec<IndexRow>> {
    let idx = inverted_index(col, spec)?;
    let prefix = idx.index_key_prefix();
    let mut expected = Vec::new();
    for data_row in rows {
        let value = indexed_value(&data_row.row, &spec.on_field)?;
        for (key, bytes) in idx.encode_put_entries(value, &data_row.pk, *stats)? {
            if is_inverted_stats_key(&prefix, &key) {
                *latest_stats_row = Some((ColumnFamily::IndexInverted, key, bytes));
            } else {
                expected.push((ColumnFamily::IndexInverted, key, bytes));
            }
        }
        *stats = idx.stats_after_put(value, *stats)?;
    }
    Ok(expected)
}

fn validate_index_key(col: &Collection, spec: &IndexSpec, prefix: &[u8], key: &[u8]) -> Result<()> {
    match spec.kind {
        IndexKind::Btree => {
            btree_index(col, spec)?.decode_index_key(key)?;
        }
        IndexKind::Inverted => {
            if !is_inverted_stats_key(prefix, key) {
                inverted_index(col, spec)?.decode_posting_key(key)?;
            }
        }
        _ => {
            return Err(invalid_argument(
                "index rebuild supports btree/inverted specs",
            ));
        }
    }
    Ok(())
}
