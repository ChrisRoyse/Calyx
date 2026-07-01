use super::{
    BASE_PAGE_INDEX_DIR, BasePageIndexEntry, BasePageIndexPage, BasePageIndexPageRef,
    BasePageIndexSource, corrupt, hex_bytes, sha256_hex, stale,
};
use crate::cf::ColumnFamily;
use crate::mvcc::is_tombstone_value;
use crate::sst::SstReader;
use crate::vault::encode::decode_write_batch;
use crate::wal::read_record_at;
use calyx_core::{CalyxError, Result};
use std::fs;
use std::path::Path;

pub(super) fn read_page(
    vault: &Path,
    page_ref: &BasePageIndexPageRef,
) -> Result<BasePageIndexPage> {
    let path = vault.join(BASE_PAGE_INDEX_DIR).join(&page_ref.path);
    let bytes = fs::read(&path).map_err(|error| {
        CalyxError::disk_pressure(format!("read Base page index page: {error}"))
    })?;
    let actual = sha256_hex(&bytes);
    if actual != page_ref.sha256_hex {
        return Err(corrupt(format!(
            "Base page index page {} sha256 mismatch: expected {}, got {}",
            path.display(),
            page_ref.sha256_hex,
            actual
        )));
    }
    let page: BasePageIndexPage = serde_json::from_slice(&bytes)
        .map_err(|error| corrupt(format!("decode Base page index page: {error}")))?;
    if page.entries.len() != page_ref.entry_count {
        return Err(corrupt(format!(
            "Base page index page {} expected {} entries, got {}",
            path.display(),
            page_ref.entry_count,
            page.entries.len()
        )));
    }
    Ok(page)
}

pub(super) fn read_source_value(
    vault: &Path,
    key: &[u8],
    source: &BasePageIndexSource,
) -> Result<Vec<u8>> {
    match source {
        BasePageIndexSource::Sst { path, .. } => {
            let source_path = vault.join(path);
            if !source_path.exists() {
                return Err(stale(format!(
                    "Base page index source SST {} no longer exists",
                    source_path.display()
                )));
            }
            SstReader::open(&source_path)?.get(key)?.ok_or_else(|| {
                stale(format!(
                    "Base page index source SST {} no longer contains key {}",
                    source_path.display(),
                    hex_bytes(key)
                ))
            })
        }
        BasePageIndexSource::Wal {
            path,
            seq,
            start_offset,
            end_offset,
        } => read_wal_source_value(vault, key, path, *seq, *start_offset, *end_offset),
    }
}

fn read_wal_source_value(
    vault: &Path,
    key: &[u8],
    path: &str,
    seq: u64,
    start_offset: u64,
    end_offset: u64,
) -> Result<Vec<u8>> {
    let source_path = vault.join(path);
    if !source_path.exists() {
        return Err(stale(format!(
            "Base page index source WAL {} no longer exists",
            source_path.display()
        )));
    }
    let record = read_record_at(&source_path, seq, start_offset, end_offset)?;
    for row in decode_write_batch(&record.payload)? {
        if row.cf == ColumnFamily::Base && row.key == key {
            return Ok(row.value);
        }
    }
    Err(stale(format!(
        "Base page index source WAL record {seq} no longer contains key {}",
        hex_bytes(key)
    )))
}

pub(super) fn validate_entry_value(entry: &BasePageIndexEntry, value: &[u8]) -> Result<()> {
    let hash = sha256_hex(value);
    if hash != entry.value_sha256_hex {
        return Err(corrupt(format!(
            "Base page index key {} source value sha256 mismatch: expected {}, got {}",
            entry.key_hex, entry.value_sha256_hex, hash
        )));
    }
    let tombstoned = is_tombstone_value(value);
    if tombstoned != entry.tombstoned {
        return Err(corrupt(format!(
            "Base page index key {} tombstone state mismatch: manifest {}, source {}",
            entry.key_hex, entry.tombstoned, tombstoned
        )));
    }
    Ok(())
}
