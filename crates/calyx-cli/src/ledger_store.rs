use std::collections::BTreeMap;
use std::path::Path;

use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_aster::sst::SstEntry;
use calyx_aster::vault::encode::decode_write_batch;
use calyx_aster::wal::replay_dir;
use calyx_core::{CalyxError, Result as CalyxResult};
use calyx_ledger::{LedgerCfStore, LedgerRow};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AsterLedgerCfStore {
    rows: Vec<LedgerRow>,
}

impl AsterLedgerCfStore {
    pub(crate) fn open(vault: &Path) -> CalyxResult<Self> {
        let layout = AsterVaultLayout::read(vault)?;
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
            rows: rows
                .into_iter()
                .map(|(seq, bytes)| LedgerRow { seq, bytes })
                .collect(),
        })
    }
}

impl LedgerCfStore for AsterLedgerCfStore {
    fn scan(&self) -> CalyxResult<Vec<LedgerRow>> {
        Ok(self.rows.clone())
    }

    fn put_new(&mut self, seq: u64, _bytes: &[u8]) -> CalyxResult<()> {
        Err(CalyxError::ledger_append_only_violation(format!(
            "calyx CLI opened Aster ledger seq {seq} read-only"
        )))
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
                "--vault path {} is not an Aster vault directory",
                vault.display()
            )));
        }

        let layout = Self {
            has_ledger_cf: vault.join("cf").join(ColumnFamily::Ledger.name()).is_dir(),
            has_wal: vault.join("wal").is_dir(),
        };
        if !layout.has_ledger_cf && !layout.has_wal {
            return Err(CalyxError::ledger_corrupt(format!(
                "--vault requires real Aster ledger state under {}/cf/ledger or {}/wal",
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

pub(crate) fn parse_aster_ledger_seq(key: &[u8]) -> CalyxResult<u64> {
    let key: [u8; 8] = key.try_into().map_err(|_| {
        CalyxError::ledger_corrupt(format!(
            "Aster ledger CF key has {} bytes, expected 8",
            key.len()
        ))
    })?;
    Ok(u64::from_be_bytes(key))
}
