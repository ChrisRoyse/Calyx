//! Secondary-index verification and self-heal rebuild (PH54 T05).

use std::collections::BTreeSet;
use std::time::Instant;

use calyx_core::{CalyxError, Clock, Result, Seq};
use serde::{Deserialize, Serialize};

use super::inverted::InvertedStats;
use super::{BtreeIndex, IndexKind, IndexMaintenance, IndexSpec, InvertedIndex, SecondaryIndex};
use crate::cf::{ColumnFamily, prefix_range};
use crate::collection::{CALYX_INVALID_ARGUMENT, Collection, CollectionMode};
use crate::layers::relational::{CALYX_SCHEMA_VIOLATION, collection_id, decode_record_value};
use crate::layers::{RecordKey, RecordValue, Row};
use crate::mvcc::tombstone_value;
use crate::vault::AsterVault;

const DEFAULT_BATCH_SIZE: usize = 10_000;
const MAX_BATCH_SIZE: usize = 10_000;
const RECORD_DISC: u8 = 0x01;

type IndexRow = (ColumnFamily, Vec<u8>, Vec<u8>);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebuildStats {
    pub rows_scanned: u64,
    pub keys_added: u64,
    pub stale_removed: u64,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexHealth {
    pub missing: u64,
    pub stale: u64,
    pub healthy: bool,
}

struct RecordDataRow {
    pk: RecordKey,
    row: Row,
}

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
    let rows = collect_record_rows(vault, snapshot, col)?;
    let expected = expected_index_rows(vault, col, spec, &rows)?;
    let missing = rows_needing_write(vault, snapshot, &expected)?.len() as u64;
    let stale = stale_index_keys(vault, snapshot, col, spec, &expected)?.len() as u64;
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
    let rows = collect_record_rows(vault, snapshot, col)?;
    let expected = expected_index_rows(vault, col, spec, &rows)?;
    let missing_rows = rows_needing_write(vault, snapshot, &expected)?;
    let stale_keys = stale_index_keys(vault, snapshot, col, spec, &expected)?;
    let stale_cf = index_cf(spec)?;
    let stale_rows = stale_keys
        .into_iter()
        .map(|key| (stale_cf, key, tombstone_value()))
        .collect::<Vec<_>>();
    write_rows_in_batches(vault, missing_rows.iter().cloned(), batch_size)?;
    write_rows_in_batches(vault, stale_rows.iter().cloned(), batch_size)?;
    Ok(RebuildStats {
        rows_scanned: rows.len() as u64,
        keys_added: missing_rows.len() as u64,
        stale_removed: stale_rows.len() as u64,
        elapsed_ms: elapsed_ms(started),
    })
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

fn stale_index_keys<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    col: &Collection,
    spec: &IndexSpec,
    expected: &[IndexRow],
) -> Result<Vec<Vec<u8>>> {
    let cf = index_cf(spec)?;
    let prefix = index_prefix(col, spec)?;
    let expected_keys = expected
        .iter()
        .filter_map(|(row_cf, key, _)| (*row_cf == cf).then_some(key.clone()))
        .collect::<BTreeSet<_>>();
    let range = prefix_range(&prefix);
    let mut stale = Vec::new();
    for (key, _value) in vault.scan_cf_range_at(snapshot, cf, &range)? {
        if !expected_keys.contains(&key) {
            validate_index_key(col, spec, &prefix, &key)?;
            stale.push(key);
        }
    }
    Ok(stale)
}

fn expected_index_rows<C: Clock>(
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
    rows: &[RecordDataRow],
) -> Result<Vec<IndexRow>> {
    match spec.kind {
        IndexKind::Btree => expected_btree_rows(vault, col, spec, rows),
        IndexKind::Inverted => expected_inverted_rows(col, spec, rows),
        _ => Err(invalid_argument(
            "index rebuild supports btree/inverted specs",
        )),
    }
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
) -> Result<Vec<IndexRow>> {
    let idx = inverted_index(col, spec)?;
    let prefix = idx.index_key_prefix();
    let mut stats = InvertedStats::default();
    let mut expected = Vec::new();
    let mut latest_stats_row = None;
    for data_row in rows {
        let value = indexed_value(&data_row.row, &spec.on_field)?;
        for (key, bytes) in idx.encode_put_entries(value, &data_row.pk, stats)? {
            if is_inverted_stats_key(&prefix, &key) {
                latest_stats_row = Some((ColumnFamily::IndexInverted, key, bytes));
            } else {
                expected.push((ColumnFamily::IndexInverted, key, bytes));
            }
        }
        stats = idx.stats_after_put(value, stats)?;
    }
    if let Some(stats_row) = latest_stats_row {
        expected.push(stats_row);
    }
    Ok(expected)
}

fn collect_record_rows<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    col: &Collection,
) -> Result<Vec<RecordDataRow>> {
    let prefix = record_collection_prefix(col);
    let range = prefix_range(&prefix);
    vault
        .scan_cf_range_at(snapshot, ColumnFamily::Relational, &range)?
        .into_iter()
        .map(|(key, value)| {
            let pk = parse_record_pk(&prefix, &key, snapshot)?;
            let row = decode_record_value(&value).map_err(|error| {
                corrupt(format!(
                    "corrupt relational row at snapshot_seq={snapshot} key={}: {error}",
                    hex(&key)
                ))
            })?;
            Ok(RecordDataRow { pk, row })
        })
        .collect()
}

fn parse_record_pk(prefix: &[u8], key: &[u8], snapshot: Seq) -> Result<RecordKey> {
    if !key.starts_with(prefix) || key.len() < prefix.len() + 2 {
        return Err(corrupt(format!(
            "malformed relational key at snapshot_seq={snapshot}: {}",
            hex(key)
        )));
    }
    let len_at = prefix.len();
    let pk_len = u16::from_be_bytes([key[len_at], key[len_at + 1]]) as usize;
    let pk_start = len_at + 2;
    let pk_end = pk_start.checked_add(pk_len).ok_or_else(|| {
        corrupt(format!(
            "record key length overflow at snapshot_seq={snapshot}"
        ))
    })?;
    if pk_end != key.len() {
        return Err(corrupt(format!(
            "relational key length mismatch at snapshot_seq={snapshot}: {}",
            hex(key)
        )));
    }
    RecordKey::from_bytes(key[pk_start..pk_end].to_vec()).map_err(|error| {
        corrupt(format!(
            "relational key primary key corrupt at snapshot_seq={snapshot}: {error}"
        ))
    })
}

fn record_collection_prefix(col: &Collection) -> Vec<u8> {
    let mut prefix = Vec::with_capacity(9);
    prefix.push(RECORD_DISC);
    prefix.extend_from_slice(&collection_id(col).to_be_bytes());
    prefix
}

fn write_rows_in_batches<C: Clock>(
    vault: &AsterVault<C>,
    rows: impl IntoIterator<Item = (ColumnFamily, Vec<u8>, Vec<u8>)>,
    batch_size: usize,
) -> Result<()> {
    let mut batch = Vec::with_capacity(batch_size.min(MAX_BATCH_SIZE));
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

fn btree_index(col: &Collection, spec: &IndexSpec) -> Result<BtreeIndex> {
    spec.validate()?;
    if spec.kind != IndexKind::Btree {
        return Err(invalid_argument("btree rebuild requires kind Btree"));
    }
    Ok(BtreeIndex::new(collection_id(col), spec.clone()))
}

fn inverted_index(col: &Collection, spec: &IndexSpec) -> Result<InvertedIndex> {
    spec.validate()?;
    if spec.kind != IndexKind::Inverted {
        return Err(invalid_argument("inverted rebuild requires kind Inverted"));
    }
    Ok(InvertedIndex::new(collection_id(col), spec.clone()))
}

fn is_active_spec(col: &Collection, spec: &IndexSpec) -> Result<bool> {
    spec.validate()?;
    if col.indexes.is_empty() {
        return Ok(false);
    }
    if !matches!(spec.kind, IndexKind::Btree | IndexKind::Inverted) {
        return Err(invalid_argument(
            "index rebuild supports btree/inverted specs",
        ));
    }
    let found = col.indexes.iter().any(|declared| {
        declared.name == spec.name
            && declared.kind == spec.kind
            && declared.fields.len() == 1
            && declared.fields[0] == spec.on_field
    });
    if found {
        Ok(true)
    } else {
        Err(invalid_argument(format!(
            "index spec `{}` is not declared on collection `{}`",
            spec.name, col.name
        )))
    }
}

fn index_cf(spec: &IndexSpec) -> Result<ColumnFamily> {
    match spec.kind {
        IndexKind::Btree => Ok(ColumnFamily::IndexBtree),
        IndexKind::Inverted => Ok(ColumnFamily::IndexInverted),
        _ => Err(invalid_argument(
            "index rebuild supports btree/inverted specs",
        )),
    }
}

fn index_prefix(col: &Collection, spec: &IndexSpec) -> Result<Vec<u8>> {
    match spec.kind {
        IndexKind::Btree => Ok(btree_index(col, spec)?.index_key_prefix()),
        IndexKind::Inverted => Ok(inverted_index(col, spec)?.index_key_prefix()),
        _ => Err(invalid_argument(
            "index rebuild supports btree/inverted specs",
        )),
    }
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

fn is_inverted_stats_key(prefix: &[u8], key: &[u8]) -> bool {
    key.starts_with(prefix) && key.len() == prefix.len() + 8
}

fn indexed_value<'a>(row: &'a Row, field: &str) -> Result<&'a RecordValue> {
    row.get(field)
        .ok_or_else(|| index_schema_violation(format!("missing indexed field `{field}`")))
}

fn index_schema_violation(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_SCHEMA_VIOLATION,
        message: message.into(),
        remediation: "submit a row containing every indexed field",
    }
}

fn require_records_collection(col: &Collection) -> Result<()> {
    if col.mode == CollectionMode::Records {
        Ok(())
    } else {
        Err(invalid_argument(format!(
            "index rebuild currently scans Records collections, got {:?}",
            col.mode
        )))
    }
}

fn effective_batch_size(batch_size: usize) -> Result<usize> {
    let effective = if batch_size == 0 {
        DEFAULT_BATCH_SIZE
    } else {
        batch_size
    };
    if effective > MAX_BATCH_SIZE {
        return Err(invalid_argument(format!(
            "index rebuild batch_size {effective} exceeds max {MAX_BATCH_SIZE}"
        )));
    }
    Ok(effective)
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn invalid_argument(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_INVALID_ARGUMENT,
        message: message.into(),
        remediation: "correct the index rebuild input",
    }
}

fn corrupt(message: impl Into<String>) -> CalyxError {
    CalyxError::aster_corrupt_shard(message)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
