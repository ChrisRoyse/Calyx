//! Read-only Ledger column-family view over an Aster vault directory.
//!
//! Merges the on-disk `cf/ledger` SSTs with any unflushed WAL records into a
//! [`LedgerCfStore`] suitable for `calyx_ledger::verify_chain`. The view takes
//! the durable commit lock while copying rows and the head anchor so concurrent
//! writers cannot expose a mixed-time snapshot. It remains ledger-read-only:
//! any append attempt is a `CALYX_LEDGER_APPEND_ONLY_VIOLATION`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Result as CalyxResult};
use calyx_ledger::{LedgerCfStore, LedgerHeadAnchor, LedgerRow};

use crate::cf::{CfRouter, ColumnFamily, ledger_key};
use crate::sst::level::SstLevel;
use crate::sst::{SstEntry, SstLookupMetadata, SstReader};
use crate::storage_names::{SstName, classify_sst, sst_order_key};
use crate::vault::encode::decode_write_batch;
use crate::wal::replay_dir;

/// Read-only snapshot of a vault's Ledger column family (SSTs + WAL).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AsterLedgerCfStore {
    rows: Vec<LedgerRow>,
    anchor: Option<LedgerHeadAnchor>,
}

impl AsterLedgerCfStore {
    /// Opens the Ledger CF of the vault at `vault`, failing closed when the
    /// directory holds no real Aster ledger state.
    pub fn open(vault: &Path) -> CalyxResult<Self> {
        let layout = AsterVaultLayout::read(vault)?;
        let _commit_guard =
            crate::file_lock::FileLockGuard::acquire(&durable_commit_lock_path(vault))?;
        Self::open_with_layout(vault, layout)
    }

    /// Opens the Ledger CF when the caller already owns the durable commit lock.
    pub(crate) fn open_unlocked(vault: &Path) -> CalyxResult<Self> {
        let layout = AsterVaultLayout::read(vault)?;
        Self::open_with_layout(vault, layout)
    }

    fn open_with_layout(vault: &Path, layout: AsterVaultLayout) -> CalyxResult<Self> {
        let mut rows = BTreeMap::new();

        if layout.has_ledger_cf {
            let router = CfRouter::open(vault, 0)?;
            for entry in router.iter_cf(ColumnFamily::Ledger)? {
                insert_sst_entry(&mut rows, entry)?;
            }
        }

        if layout.has_wal {
            let replay = replay_dir(vault.join("wal"))?;
            if let Some(torn) = replay.torn_tail {
                return Err(torn.error());
            }
            for record in replay.records {
                for row in decode_write_batch(&record.payload)? {
                    if row.cf == ColumnFamily::Ledger {
                        let seq = parse_aster_ledger_seq(&row.key)?;
                        insert_ledger_bytes(&mut rows, seq, row.value)?;
                    }
                }
            }
        }

        Ok(Self {
            anchor: crate::ledger_head::read_head_anchor(vault)?,
            rows: rows
                .into_iter()
                .map(|(seq, bytes)| LedgerRow { seq, bytes })
                .collect(),
        })
    }
}

/// Reads one Ledger CF row from a fresh physical view of `vault`.
///
/// This is the point-read counterpart to [`AsterLedgerCfStore::open`]: it takes
/// the same durable commit lock and merges SST plus WAL state, but it only
/// materializes the requested ledger sequence instead of cloning the full
/// ledger into memory.
pub fn read_ledger_seq(vault: &Path, seq: u64) -> CalyxResult<Option<LedgerRow>> {
    let wanted = BTreeSet::from([seq]);
    Ok(read_ledger_seqs(vault, &wanted)?.remove(&seq))
}

/// Reads a targeted set of Ledger CF rows from one stable physical snapshot.
///
/// This is the batch point-read counterpart to [`AsterLedgerCfStore::open`].
/// It takes the durable commit lock, reads physical Ledger SSTs first, and only
/// replays WAL rows for requested sequences not present in immutable SSTs.
/// Every physical duplicate observed on the rows it reads must match byte-for-byte.
pub fn read_ledger_seqs(
    vault: &Path,
    seqs: &BTreeSet<u64>,
) -> CalyxResult<BTreeMap<u64, LedgerRow>> {
    if seqs.is_empty() {
        return Ok(BTreeMap::new());
    }
    let layout = AsterVaultLayout::read(vault)?;
    let _commit_guard = crate::file_lock::FileLockGuard::acquire(&durable_commit_lock_path(vault))?;
    let mut rows = BTreeMap::new();
    if layout.has_ledger_cf {
        read_sst_ledger_rows(vault, seqs, &mut rows)?;
    }
    let unresolved = unresolved_seqs(seqs, &rows);
    if layout.has_wal && !unresolved.is_empty() {
        read_wal_ledger_rows(vault, &unresolved, &mut rows)?;
    }
    Ok(rows
        .into_iter()
        .map(|(seq, bytes)| (seq, LedgerRow { seq, bytes }))
        .collect())
}

fn read_sst_ledger_rows(
    vault: &Path,
    wanted: &BTreeSet<u64>,
    rows: &mut BTreeMap<u64, Vec<u8>>,
) -> CalyxResult<()> {
    read_sst_ledger_rows_from_candidates(vault, wanted, rows, probable_ledger_sst_candidates)?;
    let unresolved = unresolved_seqs(wanted, rows);
    if !unresolved.is_empty() {
        read_sst_ledger_rows_from_candidates(
            vault,
            &unresolved,
            rows,
            key_range_ledger_sst_candidates,
        )?;
    }
    let unresolved = unresolved_seqs(wanted, rows);
    if !unresolved.is_empty() {
        read_sst_ledger_rows_from_candidates(
            vault,
            &unresolved,
            rows,
            named_ledger_sst_candidates,
        )?;
    }
    let unresolved = unresolved_seqs(wanted, rows);
    if !unresolved.is_empty() {
        read_sst_ledger_rows_from_candidates(
            vault,
            &unresolved,
            rows,
            complete_ledger_sst_candidates,
        )?;
    }
    Ok(())
}

fn unresolved_seqs(wanted: &BTreeSet<u64>, rows: &BTreeMap<u64, Vec<u8>>) -> BTreeSet<u64> {
    wanted
        .iter()
        .copied()
        .filter(|seq| !rows.contains_key(seq))
        .collect()
}

fn read_sst_ledger_rows_from_candidates(
    vault: &Path,
    wanted: &BTreeSet<u64>,
    rows: &mut BTreeMap<u64, Vec<u8>>,
    candidates: fn(&Path, &BTreeSet<u64>) -> CalyxResult<Vec<PathBuf>>,
) -> CalyxResult<()> {
    let level = SstLevel::from_oldest_first_with_lookup(candidates(vault, wanted)?)?;
    for seq in wanted {
        let key = ledger_key(*seq);
        for value in level.values_for_key(&key)? {
            insert_ledger_bytes(rows, *seq, value)?;
        }
    }
    Ok(())
}

fn probable_ledger_sst_candidates(
    vault: &Path,
    wanted: &BTreeSet<u64>,
) -> CalyxResult<Vec<PathBuf>> {
    let dir = vault.join("cf").join(ColumnFamily::Ledger.name());
    let mut files = Vec::new();
    for seq in wanted {
        push_ledger_sst_candidate(&dir.join(format!("{seq:020}.sst")), &mut files)?;
        push_ledger_sst_candidate(&dir.join(format!("{seq:020}-0000.sst")), &mut files)?;
    }
    sorted_unique_paths(files)
}

fn named_ledger_sst_candidates(vault: &Path, wanted: &BTreeSet<u64>) -> CalyxResult<Vec<PathBuf>> {
    let dir = vault.join("cf").join(ColumnFamily::Ledger.name());
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .map_err(|error| CalyxError::disk_pressure(format!("read ledger CF dir: {error}")))?
    {
        let path = entry
            .map_err(|error| CalyxError::disk_pressure(format!("read ledger SST entry: {error}")))?
            .path();
        let Some(name) = classify_sst(&path)? else {
            continue;
        };
        let seq = match name {
            SstName::Router { seq } | SstName::DurableBatch { seq, .. } => seq,
            SstName::Compacted { .. } => continue,
        };
        if !wanted.contains(&seq) {
            continue;
        }
        let order = sst_order_key(&path)?.ok_or_else(|| {
            CalyxError::aster_corrupt_shard(format!(
                "classified ledger SST {} has no order key",
                path.display()
            ))
        })?;
        files.push((order, path));
    }
    sorted_unique_paths(files)
}

fn key_range_ledger_sst_candidates(
    vault: &Path,
    wanted: &BTreeSet<u64>,
) -> CalyxResult<Vec<PathBuf>> {
    let dir = vault.join("cf").join(ColumnFamily::Ledger.name());
    let mut files = Vec::new();
    for seq in wanted {
        if let Some(path) = primary_ledger_sst_by_key_range(&dir, *seq)? {
            push_ledger_sst_candidate(&path, &mut files)?;
        }
    }
    sorted_unique_paths(files)
}

fn primary_ledger_sst_by_key_range(dir: &Path, seq: u64) -> CalyxResult<Option<PathBuf>> {
    let key = ledger_key(seq);
    let mut low = 0_u64;
    let mut high = seq;
    let mut candidate = None;
    while low <= high {
        let mid = low + (high - low) / 2;
        let path = dir.join(format!("{mid:020}-0000.sst"));
        if !path.try_exists().map_err(|error| {
            CalyxError::disk_pressure(format!("stat {}: {error}", path.display()))
        })? {
            if mid == 0 {
                break;
            }
            high = mid - 1;
            continue;
        }
        let lookup = ledger_sst_lookup_metadata(&path)?;
        if lookup.first_key.as_slice() <= key.as_slice() {
            candidate = Some((path, lookup));
            low = mid.saturating_add(1);
        } else if mid == 0 {
            break;
        } else {
            high = mid - 1;
        }
    }
    let Some((path, lookup)) = candidate else {
        return Ok(None);
    };
    if key.as_slice() >= lookup.first_key.as_slice() && key.as_slice() <= lookup.last_key.as_slice()
    {
        Ok(Some(path))
    } else {
        Ok(None)
    }
}

fn ledger_sst_lookup_metadata(path: &Path) -> CalyxResult<SstLookupMetadata> {
    SstReader::open(path)?.lookup_metadata().ok_or_else(|| {
        CalyxError::aster_corrupt_shard(format!("ledger SST {} has no keys", path.display()))
    })
}

fn sorted_unique_paths(
    mut files: Vec<(crate::storage_names::SstOrderKey, PathBuf)>,
) -> CalyxResult<Vec<PathBuf>> {
    files.sort_by(|(left_order, left_path), (right_order, right_path)| {
        left_order
            .cmp(right_order)
            .then_with(|| left_path.cmp(right_path))
    });
    let mut paths = files.into_iter().map(|(_, path)| path).collect::<Vec<_>>();
    paths.dedup();
    Ok(paths)
}

fn complete_ledger_sst_candidates(
    vault: &Path,
    _wanted: &BTreeSet<u64>,
) -> CalyxResult<Vec<PathBuf>> {
    let dir = vault.join("cf").join(ColumnFamily::Ledger.name());
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .map_err(|error| CalyxError::disk_pressure(format!("read ledger CF dir: {error}")))?
    {
        let path = entry
            .map_err(|error| CalyxError::disk_pressure(format!("read ledger SST entry: {error}")))?
            .path();
        let Some(_) = classify_sst(&path)? else {
            continue;
        };
        let order = sst_order_key(&path)?.ok_or_else(|| {
            CalyxError::aster_corrupt_shard(format!(
                "classified ledger SST {} has no order key",
                path.display()
            ))
        })?;
        files.push((order, path));
    }
    files.sort_by(|(left_order, left_path), (right_order, right_path)| {
        left_order
            .cmp(right_order)
            .then_with(|| left_path.cmp(right_path))
    });
    let mut paths = files.into_iter().map(|(_, path)| path).collect::<Vec<_>>();
    paths.dedup();
    Ok(paths)
}

fn push_ledger_sst_candidate(
    path: &Path,
    files: &mut Vec<(crate::storage_names::SstOrderKey, PathBuf)>,
) -> CalyxResult<()> {
    if !path
        .try_exists()
        .map_err(|error| CalyxError::disk_pressure(format!("stat {}: {error}", path.display())))?
    {
        return Ok(());
    }
    let Some(name) = classify_sst(path)? else {
        return Ok(());
    };
    match name {
        SstName::Router { .. } | SstName::DurableBatch { .. } => {}
        SstName::Compacted { .. } => {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "targeted ledger point read reached compacted ledger SST {}; add a compacted ledger row index before using point verification on compacted ledger layouts",
                path.display()
            )));
        }
    }
    let order = sst_order_key(path)?.ok_or_else(|| {
        CalyxError::aster_corrupt_shard(format!(
            "classified ledger SST {} has no order key",
            path.display()
        ))
    })?;
    files.push((order, path.to_path_buf()));
    Ok(())
}

fn read_wal_ledger_rows(
    vault: &Path,
    wanted: &BTreeSet<u64>,
    rows: &mut BTreeMap<u64, Vec<u8>>,
) -> CalyxResult<()> {
    let replay = replay_dir(vault.join("wal"))?;
    if let Some(torn) = replay.torn_tail {
        return Err(torn.error());
    }
    for record in replay.records {
        for write in decode_write_batch(&record.payload)? {
            if write.cf != ColumnFamily::Ledger {
                continue;
            }
            let seq = parse_aster_ledger_seq(&write.key)?;
            if wanted.contains(&seq) {
                insert_ledger_bytes(rows, seq, write.value)?;
            }
        }
    }
    Ok(())
}

fn durable_commit_lock_path(vault: &Path) -> PathBuf {
    vault.join("locks").join("durable.commit.lock")
}

impl LedgerCfStore for AsterLedgerCfStore {
    fn scan(&self) -> CalyxResult<Vec<LedgerRow>> {
        Ok(self.rows.clone())
    }

    fn read_seq(&self, seq: u64) -> CalyxResult<Option<LedgerRow>> {
        Ok(self.rows.iter().find(|row| row.seq == seq).cloned())
    }

    fn put_new(&mut self, seq: u64, _bytes: &[u8]) -> CalyxResult<()> {
        Err(CalyxError::ledger_append_only_violation(format!(
            "read-only Aster ledger view rejected append for seq {seq}"
        )))
    }

    fn head_anchor(&self) -> CalyxResult<Option<LedgerHeadAnchor>> {
        Ok(self.anchor.clone())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AsterVaultLayout {
    has_ledger_cf: bool,
    has_wal: bool,
}

impl AsterVaultLayout {
    fn read(vault: &Path) -> CalyxResult<Self> {
        if !vault.is_dir() {
            return Err(CalyxError::ledger_corrupt(format!(
                "vault path {} is not an Aster vault directory",
                vault.display()
            )));
        }

        let layout = Self {
            has_ledger_cf: vault.join("cf").join(ColumnFamily::Ledger.name()).is_dir(),
            has_wal: vault.join("wal").is_dir(),
        };
        if !layout.has_ledger_cf && !layout.has_wal {
            return Err(CalyxError::ledger_corrupt(format!(
                "vault requires real Aster ledger state under {}/cf/ledger or {}/wal",
                vault.display(),
                vault.display()
            )));
        }
        Ok(layout)
    }
}

fn insert_sst_entry(rows: &mut BTreeMap<u64, Vec<u8>>, entry: SstEntry) -> CalyxResult<()> {
    let seq = parse_aster_ledger_seq(&entry.key)?;
    insert_ledger_bytes(rows, seq, entry.value)
}

fn insert_ledger_bytes(
    rows: &mut BTreeMap<u64, Vec<u8>>,
    seq: u64,
    bytes: Vec<u8>,
) -> CalyxResult<()> {
    if let Some(existing) = rows.get(&seq) {
        if existing == &bytes {
            return Ok(());
        }
        return Err(CalyxError::ledger_corrupt(format!(
            "divergent Aster ledger bytes for seq {seq}"
        )));
    }
    rows.insert(seq, bytes);
    Ok(())
}

/// Parses a big-endian u64 Ledger CF key, failing closed on any other width.
pub fn parse_aster_ledger_seq(key: &[u8]) -> CalyxResult<u64> {
    let key: [u8; 8] = key.try_into().map_err(|_| {
        CalyxError::ledger_corrupt(format!(
            "Aster ledger CF key has {} bytes, expected 8",
            key.len()
        ))
    })?;
    Ok(u64::from_be_bytes(key))
}

#[cfg(test)]
mod tests;
