//! Read-only Ledger column-family view over an Aster vault directory.
//!
//! Merges the on-disk `cf/ledger` SSTs with any unflushed WAL records into a
//! [`LedgerCfStore`] suitable for `calyx_ledger::verify_chain`. The view takes
//! the durable commit lock while copying rows and the head anchor so concurrent
//! writers cannot expose a mixed-time snapshot. It remains ledger-read-only:
//! any append attempt is a `CALYX_LEDGER_APPEND_ONLY_VIOLATION`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Result as CalyxResult};
use calyx_ledger::{LedgerCfStore, LedgerHeadAnchor, LedgerRow};

use crate::cf::{CfRouter, ColumnFamily, ledger_key};
use crate::sst::SstEntry;
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
    let layout = AsterVaultLayout::read(vault)?;
    let _commit_guard = crate::file_lock::FileLockGuard::acquire(&durable_commit_lock_path(vault))?;
    let key = ledger_key(seq);
    let mut row = None;

    if layout.has_ledger_cf {
        let router = CfRouter::open_selected_cfs(vault, 0, [ColumnFamily::Ledger])?;
        if let Some(bytes) = router.get(ColumnFamily::Ledger, &key)? {
            row = Some(bytes);
        }
    }

    if layout.has_wal {
        let replay = replay_dir(vault.join("wal"))?;
        if let Some(torn) = replay.torn_tail {
            return Err(torn.error());
        }
        for record in replay.records {
            for write in decode_write_batch(&record.payload)? {
                if write.cf == ColumnFamily::Ledger && write.key == key {
                    match &row {
                        Some(existing) if existing != &write.value => {
                            return Err(CalyxError::ledger_corrupt(format!(
                                "divergent Aster ledger bytes for seq {seq}"
                            )));
                        }
                        Some(_) => {}
                        None => row = Some(write.value),
                    }
                }
            }
        }
    }

    Ok(row.map(|bytes| LedgerRow { seq, bytes }))
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
mod tests {
    use std::fs;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn open_waits_for_durable_commit_lock_before_reading_rows_and_anchor() {
        let root = test_vault_dir("issue973-open-lock");
        fs::create_dir_all(root.join("cf").join(ColumnFamily::Ledger.name())).unwrap();

        let guard = crate::file_lock::FileLockGuard::acquire(&durable_commit_lock_path(&root))
            .expect("acquire writer commit lock");
        let (sender, receiver) = mpsc::channel();
        let thread_root = root.clone();
        let handle = thread::spawn(move || {
            let result = AsterLedgerCfStore::open(&thread_root)
                .and_then(|store| store.scan().map(|rows| rows.len()));
            sender.send(result).expect("send open result");
        });

        assert!(
            receiver.recv_timeout(Duration::from_millis(100)).is_err(),
            "ledger view opened while a writer-owned durable commit lock was held"
        );

        drop(guard);
        let row_count = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("ledger view should open after commit lock release")
            .expect("open ledger view");
        assert_eq!(row_count, 0);
        handle.join().expect("open thread");
        fs::remove_dir_all(root).ok();
    }

    fn test_vault_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "calyx-aster-{name}-{}-{unique}",
            std::process::id()
        ))
    }
}
