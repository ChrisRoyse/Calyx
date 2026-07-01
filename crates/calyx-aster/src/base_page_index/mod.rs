//! Persisted Base CF page index used by bounded readback commands.
//!
//! The index is not a cache fallback. Bounded readers either verify this
//! physical source of truth against the current ledger head and referenced SST
//! or WAL bytes, or they fail closed with a `CALYX_BASE_PAGE_INDEX_*` error.

#[cfg(test)]
mod tests;

mod format;
mod readback;
mod sst_scan;
mod types;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Result};

use crate::cf::ColumnFamily;
use crate::ledger_head::read_head_anchor;
use crate::mvcc::is_tombstone_value;
use crate::sst::SstReader;
use crate::storage_names::sst_order_key;
use crate::vault::encode::decode_write_batch;
use crate::wal::stream_records;

use format::{
    corrupt, decode_hex, hex_bytes, missing, now_ms, relative_path, remove_path, sha256_hex, stale,
    sync_parent, write_bytes_file, write_json_file,
};
use readback::{read_page, read_source_value, validate_entry_value};
use sst_scan::list_base_sst_files;
pub use types::{
    BASE_PAGE_INDEX_DIR, BASE_PAGE_INDEX_MANIFEST, BasePageIndexBuildProgress, BasePageIndexEntry,
    BasePageIndexManifest, BasePageIndexPage, BasePageIndexPageRef, BasePageIndexSource,
    DEFAULT_BASE_PAGE_INDEX_PAGE_SIZE,
};
#[cfg(test)]
use types::{CORRUPT_CODE, MISSING_CODE, STALE_CODE};
use types::{INDEX_MAGIC, INDEX_VERSION};

#[derive(Clone, Debug)]
struct IndexedValue {
    value_sha256_hex: String,
    tombstoned: bool,
    source: BasePageIndexSource,
}

struct BuildSnapshot {
    ledger_head_height: u64,
    ledger_head_tip_hash_hex: String,
    base_sst_files: usize,
    wal_records: usize,
}

pub fn build_base_page_index(
    vault: &Path,
    page_size: usize,
    mut progress: impl FnMut(BasePageIndexBuildProgress) -> Result<()>,
) -> Result<BasePageIndexManifest> {
    if page_size == 0 {
        return Err(corrupt("Base page index page size must be at least 1"));
    }
    let _guard = crate::file_lock::FileLockGuard::acquire(&durable_commit_lock_path(vault))?;
    let (ledger_head_height, ledger_head_tip_hash_hex) = current_head(vault)?;
    let sst_files = list_base_sst_files(vault)?;
    progress(BasePageIndexBuildProgress::ScanStarted {
        sst_files: sst_files.len(),
        ledger_head_height,
    })?;

    let mut rows = BTreeMap::<Vec<u8>, IndexedValue>::new();
    for (index, file) in sst_files.iter().enumerate() {
        let order = sst_order_key(file)?.ok_or_else(|| {
            corrupt(format!(
                "Base SST {} has no canonical order key",
                file.display()
            ))
        })?;
        let source = BasePageIndexSource::Sst {
            path: relative_path(vault, file),
            order_seq: order.seq,
            order_class_rank: order.class_rank,
            order_index: order.index,
        };
        for entry in SstReader::open(file)?.iter()? {
            let tombstoned = is_tombstone_value(&entry.value);
            rows.insert(
                entry.key,
                IndexedValue {
                    value_sha256_hex: sha256_hex(&entry.value),
                    tombstoned,
                    source: source.clone(),
                },
            );
        }
        let scanned = index + 1;
        if scanned == 1 || scanned == sst_files.len() || scanned % 1000 == 0 {
            progress(BasePageIndexBuildProgress::SstScanned {
                scanned_sst_files: scanned,
                total_sst_files: sst_files.len(),
                current_rows: rows.len(),
            })?;
        }
    }

    let wal_records = stream_records(vault.join("wal"), |record| {
        for row in decode_write_batch(&record.payload)? {
            if row.cf != ColumnFamily::Base {
                continue;
            }
            let tombstoned = is_tombstone_value(&row.value);
            rows.insert(
                row.key,
                IndexedValue {
                    value_sha256_hex: sha256_hex(&row.value),
                    tombstoned,
                    source: BasePageIndexSource::Wal {
                        path: relative_path(vault, &record.segment_path),
                        seq: record.seq,
                        start_offset: record.start_offset,
                        end_offset: record.end_offset,
                    },
                },
            );
        }
        Ok(())
    })?;
    progress(BasePageIndexBuildProgress::WalScanned {
        wal_records,
        current_rows: rows.len(),
    })?;

    let manifest = write_index(
        vault,
        page_size,
        rows,
        BuildSnapshot {
            ledger_head_height,
            ledger_head_tip_hash_hex,
            base_sst_files: sst_files.len(),
            wal_records,
        },
        progress,
    )?;
    Ok(manifest)
}

pub fn read_base_page_index_manifest(vault: &Path) -> Result<BasePageIndexManifest> {
    read_manifest_file(&manifest_path(vault))
}

pub fn read_indexed_base_rows(vault: &Path, limit: usize) -> Result<BTreeMap<Vec<u8>, Vec<u8>>> {
    if limit == 0 {
        return Ok(BTreeMap::new());
    }
    let _guard = crate::file_lock::FileLockGuard::acquire(&durable_commit_lock_path(vault))?;
    let manifest = read_manifest_file(&manifest_path(vault))?;
    validate_current_head(vault, &manifest)?;
    let mut rows = BTreeMap::new();
    for page_ref in &manifest.pages {
        let page = read_page(vault, page_ref)?;
        for entry in &page.entries {
            let key = decode_hex(&entry.key_hex, "Base page index key")?;
            let value = read_source_value(vault, &key, &entry.source)?;
            validate_entry_value(entry, &value)?;
            if !entry.tombstoned {
                if rows.insert(key, value).is_some() {
                    return Err(corrupt("Base page index contains a duplicate live key"));
                }
                if rows.len() == limit {
                    return Ok(rows);
                }
            }
        }
    }
    Ok(rows)
}

pub fn read_indexed_base_rows_for_keys(
    vault: &Path,
    keys: &[Vec<u8>],
) -> Result<BTreeMap<Vec<u8>, Option<Vec<u8>>>> {
    let _guard = crate::file_lock::FileLockGuard::acquire(&durable_commit_lock_path(vault))?;
    let manifest = read_manifest_file(&manifest_path(vault))?;
    validate_current_head(vault, &manifest)?;
    let mut rows = keys
        .iter()
        .map(|key| (key.clone(), None))
        .collect::<BTreeMap<_, _>>();
    for key in keys {
        let key_hex = hex_bytes(key);
        let Some(page_ref) = manifest.pages.iter().find(|page| {
            page.first_key_hex.as_str() <= key_hex.as_str()
                && key_hex.as_str() <= page.last_key_hex.as_str()
        }) else {
            continue;
        };
        let page = read_page(vault, page_ref)?;
        let Some(entry) = page.entries.iter().find(|entry| entry.key_hex == key_hex) else {
            continue;
        };
        let value = read_source_value(vault, key, &entry.source)?;
        validate_entry_value(entry, &value)?;
        rows.insert(key.clone(), Some(value));
    }
    Ok(rows)
}

pub fn advance_base_page_index_head_if_base_unchanged(vault: &Path) -> Result<bool> {
    let path = manifest_path(vault);
    if !path.exists() {
        return Ok(false);
    }
    let _guard = crate::file_lock::FileLockGuard::acquire(&durable_commit_lock_path(vault))?;
    let mut manifest = read_manifest_file(&path)?;
    let current_base_sst_files = list_base_sst_files(vault)?.len();
    if current_base_sst_files != manifest.base_sst_files {
        return Err(stale(format!(
            "Base page index covers {} Base SST files but current vault has {}; refusing to advance index head without rebuild",
            manifest.base_sst_files, current_base_sst_files
        )));
    }
    let (height, tip_hash_hex) = current_head(vault)?;
    if height == manifest.ledger_head_height && tip_hash_hex == manifest.ledger_head_tip_hash_hex {
        return Ok(false);
    }
    if height < manifest.ledger_head_height {
        return Err(corrupt(format!(
            "Base page index head would regress from {} to {height}",
            manifest.ledger_head_height
        )));
    }
    manifest.ledger_head_height = height;
    manifest.ledger_head_tip_hash_hex = tip_hash_hex;
    write_json_file(&path, &manifest)?;
    sync_parent(&path)?;
    Ok(true)
}

fn write_index(
    vault: &Path,
    page_size: usize,
    rows: BTreeMap<Vec<u8>, IndexedValue>,
    snapshot: BuildSnapshot,
    mut progress: impl FnMut(BasePageIndexBuildProgress) -> Result<()>,
) -> Result<BasePageIndexManifest> {
    let tmp = vault.join(format!(".{BASE_PAGE_INDEX_DIR}.{}.tmp", std::process::id()));
    if tmp.exists() {
        remove_path(&tmp)?;
    }
    fs::create_dir_all(&tmp)
        .map_err(|error| CalyxError::disk_pressure(format!("create Base page index: {error}")))?;
    let mut pages = Vec::new();
    let mut chunk = Vec::with_capacity(page_size);
    let mut page_index = 0;
    let mut live_entries = 0;
    let total_entries = rows.len();
    for (key, indexed) in rows {
        let tombstoned = indexed.tombstoned;
        if !tombstoned {
            live_entries += 1;
        }
        chunk.push(BasePageIndexEntry {
            key_hex: hex_bytes(&key),
            value_sha256_hex: indexed.value_sha256_hex,
            tombstoned,
            source: indexed.source,
        });
        if chunk.len() == page_size {
            write_page(&tmp, page_index, std::mem::take(&mut chunk), &mut pages)?;
            emit_page_progress(
                &mut progress,
                page_index,
                pages.last().expect("page written"),
            )?;
            page_index += 1;
        }
    }
    if !chunk.is_empty() {
        write_page(&tmp, page_index, chunk, &mut pages)?;
        emit_page_progress(
            &mut progress,
            page_index,
            pages.last().expect("page written"),
        )?;
    }
    let manifest = BasePageIndexManifest {
        magic: INDEX_MAGIC.to_string(),
        version: INDEX_VERSION,
        ledger_head_height: snapshot.ledger_head_height,
        ledger_head_tip_hash_hex: snapshot.ledger_head_tip_hash_hex,
        page_size,
        total_entries,
        live_entries,
        tombstone_entries: total_entries.saturating_sub(live_entries),
        base_sst_files: snapshot.base_sst_files,
        wal_records: snapshot.wal_records,
        built_at_unix_ms: now_ms()?,
        pages,
    };
    write_json_file(&tmp.join(BASE_PAGE_INDEX_MANIFEST), &manifest)?;
    let final_dir = vault.join(BASE_PAGE_INDEX_DIR);
    if final_dir.exists() {
        remove_path(&final_dir)?;
    }
    fs::rename(&tmp, &final_dir).map_err(|error| {
        CalyxError::disk_pressure(format!(
            "replace Base page index {}: {error}",
            final_dir.display()
        ))
    })?;
    sync_parent(&final_dir)?;
    progress(BasePageIndexBuildProgress::Complete {
        total_entries: manifest.total_entries,
        live_entries: manifest.live_entries,
        pages: manifest.pages.len(),
    })?;
    Ok(manifest)
}

fn write_page(
    dir: &Path,
    page_index: usize,
    entries: Vec<BasePageIndexEntry>,
    pages: &mut Vec<BasePageIndexPageRef>,
) -> Result<()> {
    let first_key_hex = entries
        .first()
        .map(|entry| entry.key_hex.clone())
        .unwrap_or_default();
    let last_key_hex = entries
        .last()
        .map(|entry| entry.key_hex.clone())
        .unwrap_or_default();
    let live_entry_count = entries.iter().filter(|entry| !entry.tombstoned).count();
    let page = BasePageIndexPage { entries };
    let bytes = serde_json::to_vec_pretty(&page)
        .map_err(|error| corrupt(format!("encode Base page index page: {error}")))?;
    let file_name = format!("page-{page_index:08}.json");
    write_bytes_file(&dir.join(&file_name), &bytes)?;
    pages.push(BasePageIndexPageRef {
        path: file_name,
        first_key_hex,
        last_key_hex,
        entry_count: page.entries.len(),
        live_entry_count,
        sha256_hex: sha256_hex(&bytes),
    });
    Ok(())
}

fn emit_page_progress(
    progress: &mut impl FnMut(BasePageIndexBuildProgress) -> Result<()>,
    page_index: usize,
    page: &BasePageIndexPageRef,
) -> Result<()> {
    progress(BasePageIndexBuildProgress::PageWritten {
        page_index,
        entry_count: page.entry_count,
        live_entry_count: page.live_entry_count,
    })
}

fn read_manifest_file(path: &Path) -> Result<BasePageIndexManifest> {
    if !path.exists() {
        return Err(missing(format!(
            "Base page index manifest is missing at {}",
            path.display()
        )));
    }
    let bytes = fs::read(path).map_err(|error| {
        CalyxError::disk_pressure(format!("read Base page index manifest: {error}"))
    })?;
    let manifest: BasePageIndexManifest = serde_json::from_slice(&bytes)
        .map_err(|error| corrupt(format!("decode Base page index manifest: {error}")))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &BasePageIndexManifest) -> Result<()> {
    if manifest.magic != INDEX_MAGIC {
        return Err(corrupt(format!(
            "Base page index manifest magic {} is not {INDEX_MAGIC}",
            manifest.magic
        )));
    }
    if manifest.version != INDEX_VERSION {
        return Err(corrupt(format!(
            "Base page index version {} is not {INDEX_VERSION}",
            manifest.version
        )));
    }
    if manifest.page_size == 0 {
        return Err(corrupt("Base page index manifest page_size is zero"));
    }
    if manifest.live_entries + manifest.tombstone_entries != manifest.total_entries {
        return Err(corrupt("Base page index manifest row counts do not add up"));
    }
    if manifest
        .pages
        .iter()
        .map(|page| page.entry_count)
        .sum::<usize>()
        != manifest.total_entries
    {
        return Err(corrupt("Base page index page counts do not add up"));
    }
    Ok(())
}

fn validate_current_head(vault: &Path, manifest: &BasePageIndexManifest) -> Result<()> {
    let (height, tip_hash_hex) = current_head(vault)?;
    if height != manifest.ledger_head_height || tip_hash_hex != manifest.ledger_head_tip_hash_hex {
        return Err(stale(format!(
            "Base page index was built at ledger head {}:{} but current head is {}:{}",
            manifest.ledger_head_height, manifest.ledger_head_tip_hash_hex, height, tip_hash_hex
        )));
    }
    Ok(())
}

fn manifest_path(vault: &Path) -> PathBuf {
    vault
        .join(BASE_PAGE_INDEX_DIR)
        .join(BASE_PAGE_INDEX_MANIFEST)
}

fn durable_commit_lock_path(vault: &Path) -> PathBuf {
    vault.join("locks").join("durable.commit.lock")
}

fn current_head(vault: &Path) -> Result<(u64, String)> {
    let Some(anchor) = read_head_anchor(vault)? else {
        return Ok((0, hex_bytes(&[0_u8; 32])));
    };
    Ok((anchor.height, hex_bytes(&anchor.tip_hash)))
}
