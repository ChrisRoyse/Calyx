use calyx_core::{Result, Seq};

use crate::cf::{ColumnFamily, prefix_range};
use crate::collection::Collection;
use crate::index::rebuild::support::{corrupt, hex};
use crate::layers::relational::{collection_id, decode_record_value, record_key};
use crate::layers::{RecordKey, Row};
use crate::vault::AsterVault;
use calyx_core::Clock;

use super::types::{RECORD_DISC, RecordDataRow};

pub(super) fn collect_record_rows_page<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    col: &Collection,
    after_key: Option<&[u8]>,
    limit: usize,
) -> Result<Vec<RecordDataRow>> {
    let prefix = record_collection_prefix(col);
    let range = prefix_range(&prefix);
    vault
        .scan_cf_range_page_at(snapshot, ColumnFamily::Relational, &range, after_key, limit)?
        .into_iter()
        .map(|(key, value)| {
            let pk = parse_record_pk(&prefix, &key, snapshot)?;
            let row = decode_record_value(&value).map_err(|error| {
                corrupt(format!(
                    "corrupt relational row at snapshot_seq={snapshot} key={}: {error}",
                    hex(&key)
                ))
            })?;
            Ok(RecordDataRow {
                data_key: key,
                pk,
                row,
            })
        })
        .collect()
}

pub(super) fn read_record_row<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    col: &Collection,
    pk: &RecordKey,
) -> Result<Option<Row>> {
    let key = record_key(col, pk)?;
    vault
        .read_cf_at(snapshot, ColumnFamily::Relational, &key)?
        .map(|value| {
            decode_record_value(&value).map_err(|error| {
                corrupt(format!(
                    "corrupt relational row at snapshot_seq={snapshot} key={}: {error}",
                    hex(&key)
                ))
            })
        })
        .transpose()
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
