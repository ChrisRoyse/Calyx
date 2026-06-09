use std::collections::BTreeMap;
use std::env;
use std::ops::Range;
use std::path::Path;

use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_aster::sst::SstEntry;
use calyx_aster::vault::encode::decode_write_batch;
use calyx_aster::wal::replay_dir;
use calyx_core::{CalyxError, Result as CalyxResult};
use calyx_ledger::{DirectoryLedgerStore, LedgerCfStore, LedgerRow, merkle_root};

pub fn print_root(ledger_dir: &Path, range: Range<u64>) -> Result<(), String> {
    let store = DirectoryLedgerStore::open(ledger_dir).map_err(|error| error.to_string())?;
    let root = merkle_root(&store, range).map_err(|error| error.to_string())?;
    println!("{}", hex_bytes(&root));
    Ok(())
}

pub fn print_root_from_env(range: Range<u64>) -> Result<(), String> {
    let ledger_dir = env::var("CALYX_LEDGER_DIR")
        .map_err(|_| "CALYX_LEDGER_DIR is required when --ledger is omitted".to_string())?;
    print_root(Path::new(&ledger_dir), range)
}

pub fn print_root_from_vault(vault: &Path, range: Range<u64>) -> Result<(), String> {
    let store = AsterLedgerCfStore::open(vault).map_err(|error| error.to_string())?;
    let root = merkle_root(&store, range).map_err(|error| error.to_string())?;
    println!("{}", hex_bytes(&root));
    Ok(())
}

pub fn parse_range(value: &str) -> Result<Range<u64>, String> {
    let (start, end) = value
        .split_once("..")
        .ok_or_else(|| "range must use a..b syntax".to_string())?;
    let start = start
        .parse::<u64>()
        .map_err(|error| format!("invalid range start: {error}"))?;
    let end = end
        .parse::<u64>()
        .map_err(|error| format!("invalid range end: {error}"))?;
    if start > end {
        return Err(format!("range start {start} > end {end}"));
    }
    Ok(start..end)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AsterLedgerCfStore {
    rows: Vec<LedgerRow>,
}

impl AsterLedgerCfStore {
    fn open(vault: &Path) -> CalyxResult<Self> {
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
            "calyx merkle-root --vault opened Aster ledger seq {seq} read-only"
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

fn parse_aster_ledger_seq(key: &[u8]) -> CalyxResult<u64> {
    let key: [u8; 8] = key.try_into().map_err(|_| {
        CalyxError::ledger_corrupt(format!(
            "Aster ledger CF key has {} bytes, expected 8",
            key.len()
        ))
    })?;
    Ok(u64::from_be_bytes(key))
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_range_accepts_half_open_range() {
        assert_eq!(parse_range("0..4").unwrap(), 0..4);
    }

    #[test]
    fn parse_range_rejects_reverse_range() {
        let error = parse_range("5..4").unwrap_err();
        assert!(error.contains("start 5 > end 4"));
    }

    #[test]
    fn aster_ledger_keys_are_big_endian_u64() {
        assert_eq!(parse_aster_ledger_seq(&9_u64.to_be_bytes()).unwrap(), 9);
    }

    #[test]
    fn aster_ledger_keys_reject_wrong_width() {
        let error = parse_aster_ledger_seq(&[1, 2, 3]).unwrap_err();
        assert_eq!(error.code, "CALYX_LEDGER_CORRUPT");
        assert!(error.to_string().contains("expected 8"));
    }
}
