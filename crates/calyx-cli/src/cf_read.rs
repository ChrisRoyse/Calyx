use calyx_aster::cf::ColumnFamily;
use calyx_aster::sst::SstReader;
use calyx_aster::storage_names::{SstName, classify_sst};
use calyx_aster::vault::encode::{decode_constellation_base, decode_write_batch};
use calyx_aster::wal::replay_dir;
use calyx_core::VaultId;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Lists canonical Aster SST files in deterministic readback order.
pub(crate) fn list_sst_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    if !dir.exists() {
        return Ok(files);
    }
    for entry in fs::read_dir(dir).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if classify_sst(&path)
            .map_err(|error| error.to_string())?
            .is_some()
        {
            files.push(path);
        }
    }
    files.sort_by(|left, right| sst_order(left).cmp(&sst_order(right)).then(left.cmp(right)));
    Ok(files)
}

pub(crate) fn sst_order(path: &Path) -> (u64, usize) {
    match classify_sst(path).ok().flatten() {
        Some(SstName::Router { seq }) => (seq, 0),
        Some(SstName::DurableBatch { seq, index }) => (seq, index),
        Some(SstName::Compacted { seq }) => (seq, usize::MAX),
        None => (0, 0),
    }
}

pub(crate) fn latest_cf_rows(
    vault: &Path,
    cf: ColumnFamily,
) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, String> {
    let mut rows = BTreeMap::new();
    for file in list_sst_files(&vault.join("cf").join(cf.name()))? {
        let reader = SstReader::open(&file).map_err(|error| error.to_string())?;
        for row in reader.iter().map_err(|error| error.to_string())? {
            rows.insert(row.key, row.value);
        }
    }
    let replay = replay_dir(vault.join("wal")).map_err(|error| error.to_string())?;
    for record in replay.records {
        for row in decode_write_batch(&record.payload).map_err(|error| error.to_string())? {
            if row.cf == cf {
                rows.insert(row.key, row.value);
            }
        }
    }
    Ok(rows)
}

pub(crate) fn vault_id_from_base(vault: &Path) -> Result<VaultId, String> {
    latest_cf_rows(vault, ColumnFamily::Base)?
        .into_values()
        .next()
        .map(|bytes| {
            decode_constellation_base(&bytes)
                .map(|cx| cx.vault_id)
                .map_err(|error| error.to_string())
        })
        .transpose()?
        .ok_or_else(|| "cannot infer vault id: base CF has no rows".to_string())
}

pub(crate) fn hex_bytes(bytes: &[u8]) -> String {
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
    fn hex_bytes_matches_lowercase_plain_hex() {
        assert_eq!(hex_bytes(b"k1"), "6b31");
    }

    #[test]
    fn sst_order_places_compacted_last_for_same_seq() {
        assert!(
            sst_order(Path::new("00000000000000000007-0001.sst"))
                < sst_order(Path::new("compacted-00000000000000000007.sst"))
        );
    }
}
