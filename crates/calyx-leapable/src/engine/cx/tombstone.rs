use calyx_aster::cf::{ColumnFamily, prefix_range};
use calyx_core::VaultStore;
use calyx_ledger::{ErasureTombstone, decode as decode_ledger, tombstone_from_entry};
use serde_json::{Value, json};

use super::super::{EngineResult, VaultHandle};

const TOMBSTONE_INDEX_MARKER: &[u8] = b"cx_tombstone_index_built_v1";
const TOMBSTONE_INDEX_PREFIX: &[u8] = b"cx_tombstone_v1/";
const LEDGER_SCAN_PAGE_ROWS: usize = 1024;
const TOMBSTONE_SCAN_LIMIT: usize = 1000;

pub(super) struct TombstonePage {
    pub(super) items: Vec<Value>,
    pub(super) truncated: bool,
}

pub(super) fn ensure_tombstone_index(handle: &VaultHandle) -> EngineResult<()> {
    let snapshot = handle.vault.snapshot();
    if handle
        .vault
        .read_cf_at(snapshot, ColumnFamily::Leapable, TOMBSTONE_INDEX_MARKER)?
        .is_some()
    {
        return Ok(());
    }
    let mut rows = Vec::new();
    handle.vault.scan_cf_pages_at(
        snapshot,
        ColumnFamily::Ledger,
        LEDGER_SCAN_PAGE_ROWS,
        |page| -> EngineResult<()> {
            for (_, bytes) in page {
                let entry = decode_ledger(&bytes)?;
                if let Some(tombstone) = tombstone_from_entry(&entry)? {
                    rows.push(tombstone_index_row(&tombstone));
                }
            }
            Ok(())
        },
    )?;
    rows.push((
        ColumnFamily::Leapable,
        TOMBSTONE_INDEX_MARKER.to_vec(),
        snapshot.to_be_bytes().to_vec(),
    ));
    handle.vault.write_cf_batch(rows)?;
    Ok(())
}

pub(super) fn index_erase_tombstone(
    handle: &VaultHandle,
    tombstone: Option<&ErasureTombstone>,
) -> EngineResult<()> {
    if let Some(tombstone) = tombstone {
        handle
            .vault
            .write_cf_batch([tombstone_index_row(tombstone)])?;
    }
    Ok(())
}

pub(super) fn scan_tombstones(handle: &VaultHandle, snapshot: u64) -> EngineResult<TombstonePage> {
    let mut rows = handle.vault.scan_cf_range_page_at(
        snapshot,
        ColumnFamily::Leapable,
        &prefix_range(TOMBSTONE_INDEX_PREFIX),
        None,
        TOMBSTONE_SCAN_LIMIT + 1,
    )?;
    let truncated = rows.len() > TOMBSTONE_SCAN_LIMIT;
    rows.truncate(TOMBSTONE_SCAN_LIMIT);
    let mut items = Vec::with_capacity(rows.len());
    for (_, bytes) in rows {
        let tombstone = ErasureTombstone::from_ledger_payload(&bytes)?;
        items.push(tombstone_value(&tombstone));
    }
    Ok(TombstonePage { items, truncated })
}

fn tombstone_index_row(tombstone: &ErasureTombstone) -> (ColumnFamily, Vec<u8>, Vec<u8>) {
    (
        ColumnFamily::Leapable,
        tombstone_index_key(tombstone.seq),
        tombstone.as_ledger_payload(),
    )
}

fn tombstone_index_key(seq: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(TOMBSTONE_INDEX_PREFIX.len() + 8);
    key.extend_from_slice(TOMBSTONE_INDEX_PREFIX);
    key.extend_from_slice(&seq.to_be_bytes());
    key
}

fn tombstone_value(tombstone: &ErasureTombstone) -> Value {
    json!({
        "seq": tombstone.seq,
        "scope": &tombstone.scope,
        "actor": &tombstone.actor,
        "erased_at": tombstone.erased_at,
        "records_deleted": tombstone.records_deleted,
        "compact": tombstone.as_json_value(),
    })
}
