use calyx_aster::cf::ColumnFamily;
use calyx_aster::manifest::ManifestStore;
use calyx_aster::sst::SstReader;
use calyx_aster::sst::level::SstLevel;
use calyx_aster::storage_names::{SstName, classify_sst};
use calyx_aster::vault::encode::{decode_constellation_base, decode_write_batch};
use calyx_aster::wal::{ReplayOutcome, replay_dir_after};
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
    let replay = replay_after_manifest(vault)?;
    for record in replay.records {
        for row in decode_write_batch(&record.payload).map_err(|error| error.to_string())? {
            if row.cf == cf {
                rows.insert(row.key, row.value);
            }
        }
    }
    Ok(rows)
}

pub(crate) fn latest_cf_row(
    vault: &Path,
    cf: ColumnFamily,
    key: &[u8],
) -> Result<Option<Vec<u8>>, String> {
    let sst_files = list_sst_files(&vault.join("cf").join(cf.name()))?;
    let level = SstLevel::from_oldest_first(sst_files);
    let mut value = level.get(key).map_err(|error| error.to_string())?;
    let replay = replay_after_manifest(vault)?;
    for record in replay.records {
        for row in decode_write_batch(&record.payload).map_err(|error| error.to_string())? {
            if row.cf == cf && row.key == key {
                value = Some(row.value);
            }
        }
    }
    Ok(value)
}

pub(crate) fn latest_cf_rows_for_keys(
    vault: &Path,
    cf: ColumnFamily,
    keys: &[Vec<u8>],
) -> Result<BTreeMap<Vec<u8>, Option<Vec<u8>>>, String> {
    let sst_files = list_sst_files(&vault.join("cf").join(cf.name()))?;
    let level = SstLevel::from_oldest_first(sst_files);
    let mut rows = BTreeMap::new();
    for key in keys {
        rows.insert(
            key.clone(),
            level.get(key).map_err(|error| error.to_string())?,
        );
    }
    let replay = replay_after_manifest(vault)?;
    for record in replay.records {
        for row in decode_write_batch(&record.payload).map_err(|error| error.to_string())? {
            if row.cf == cf && rows.contains_key(&row.key) {
                rows.insert(row.key, Some(row.value));
            }
        }
    }
    Ok(rows)
}

pub(crate) fn replay_after_manifest(vault: &Path) -> Result<ReplayOutcome, String> {
    let floor = wal_replay_floor(vault)?;
    replay_dir_after(vault.join("wal"), floor).map_err(|error| error.to_string())
}

fn wal_replay_floor(vault: &Path) -> Result<u64, String> {
    if vault.join("CURRENT").exists() || vault.join("MANIFEST").exists() {
        return ManifestStore::open(vault)
            .load_current()
            .map(|manifest| manifest.durable_seq)
            .map_err(|error| error.to_string());
    }
    Ok(0)
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

    #[test]
    fn latest_cf_row_reads_requested_key_from_latest_sst() {
        let root = temp_root("latest-cf-row");
        let base = root.join("cf").join(ColumnFamily::Base.name());
        fs::create_dir_all(&base).unwrap();
        calyx_aster::sst::write_sst(
            base.join("00000000000000000001.sst"),
            [(b"k1".as_slice(), b"old".as_slice()), (b"k2", b"other")],
        )
        .unwrap();
        calyx_aster::sst::write_sst(
            base.join("00000000000000000002.sst"),
            [(b"k1".as_slice(), b"new".as_slice())],
        )
        .unwrap();

        assert_eq!(
            latest_cf_row(&root, ColumnFamily::Base, b"k1").unwrap(),
            Some(b"new".to_vec())
        );
        assert_eq!(
            latest_cf_row(&root, ColumnFamily::Base, b"missing").unwrap(),
            None
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn latest_cf_rows_for_keys_reads_only_requested_latest_rows() {
        let root = temp_root("latest-cf-rows-for-keys");
        let base = root.join("cf").join(ColumnFamily::Base.name());
        fs::create_dir_all(&base).unwrap();
        calyx_aster::sst::write_sst(
            base.join("00000000000000000001.sst"),
            [(b"k1".as_slice(), b"old".as_slice()), (b"k2", b"other")],
        )
        .unwrap();
        calyx_aster::sst::write_sst(
            base.join("00000000000000000002.sst"),
            [(b"k1".as_slice(), b"new".as_slice()), (b"k3", b"skip")],
        )
        .unwrap();

        let rows = latest_cf_rows_for_keys(
            &root,
            ColumnFamily::Base,
            &[b"k1".to_vec(), b"missing".to_vec()],
        )
        .unwrap();

        assert_eq!(rows.get(b"k1".as_slice()).unwrap(), &Some(b"new".to_vec()));
        assert_eq!(rows.get(b"missing".as_slice()).unwrap(), &None);
        assert!(!rows.contains_key(b"k3".as_slice()));
        fs::remove_dir_all(root).ok();
    }

    fn temp_root(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "calyx-cf-read-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        path
    }
}
