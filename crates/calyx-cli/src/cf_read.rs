use calyx_aster::cf::ColumnFamily;
use calyx_aster::manifest::ManifestStore;
use calyx_aster::sst::SstReader;
use calyx_aster::sst::level::SstLevel;
use calyx_aster::storage_names::{
    SstOrderKey, classify_sst, ensure_unambiguous_sst_order, sst_order_key,
};
use calyx_aster::vault::encode::{decode_constellation_base, decode_write_batch};
use calyx_aster::wal::{ReplayOutcome, replay_dir_after};
use calyx_core::VaultId;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{CliError, CliResult};

/// Lists canonical Aster SST files in deterministic readback order, failing
/// closed on seq-domain-ambiguous layouts (issue #1138): callers fold rows
/// newest-wins in this order, so an ambiguous order would read stale rows.
pub(crate) fn list_sst_files(dir: &Path) -> CliResult<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !dir.exists() {
        return Ok(files);
    }
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if classify_sst(&path)?.is_some() {
            files.push(path);
        }
    }
    ensure_unambiguous_sst_order(files.iter().map(PathBuf::as_path))?;
    order_sst_files(files)
}

pub(crate) fn sst_order(path: &Path) -> CliResult<SstOrderKey> {
    sst_order_key(path)?.ok_or_else(|| {
        CliError::runtime(format!(
            "unrecognized canonical SST order for {}",
            path.display()
        ))
    })
}

pub(crate) fn order_sst_files(files: Vec<PathBuf>) -> CliResult<Vec<PathBuf>> {
    let mut ordered = Vec::with_capacity(files.len());
    for path in files {
        ordered.push((sst_order(&path)?, path));
    }
    ordered.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
    Ok(ordered.into_iter().map(|(_, path)| path).collect())
}

pub(crate) fn latest_cf_rows(
    vault: &Path,
    cf: ColumnFamily,
) -> CliResult<BTreeMap<Vec<u8>, Vec<u8>>> {
    let mut rows = BTreeMap::new();
    for file in list_sst_files(&vault.join("cf").join(cf.name()))? {
        let reader = SstReader::open(&file)?;
        for row in reader.iter()? {
            rows.insert(row.key, row.value);
        }
    }
    let replay = replay_after_manifest(vault)?;
    for record in replay.records {
        for row in decode_write_batch(&record.payload)? {
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
) -> CliResult<Option<Vec<u8>>> {
    let sst_files = list_sst_files(&vault.join("cf").join(cf.name()))?;
    let level = SstLevel::from_oldest_first(sst_files);
    let mut value = level.get(key)?;
    let replay = replay_after_manifest(vault)?;
    for record in replay.records {
        for row in decode_write_batch(&record.payload)? {
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
) -> CliResult<BTreeMap<Vec<u8>, Option<Vec<u8>>>> {
    let sst_files = list_sst_files(&vault.join("cf").join(cf.name()))?;
    let level = SstLevel::from_oldest_first(sst_files);
    let mut rows = BTreeMap::new();
    for key in keys {
        rows.insert(key.clone(), level.get(key)?);
    }
    let replay = replay_after_manifest(vault)?;
    for record in replay.records {
        for row in decode_write_batch(&record.payload)? {
            if row.cf == cf && rows.contains_key(&row.key) {
                rows.insert(row.key, Some(row.value));
            }
        }
    }
    Ok(rows)
}

pub(crate) fn replay_after_manifest(vault: &Path) -> CliResult<ReplayOutcome> {
    let floor = wal_replay_floor(vault)?;
    Ok(replay_dir_after(vault.join("wal"), floor)?)
}

fn wal_replay_floor(vault: &Path) -> CliResult<u64> {
    if vault.join("CURRENT").exists() || vault.join("MANIFEST").exists() {
        return Ok(ManifestStore::open(vault)
            .load_current()
            .map(|manifest| manifest.durable_seq)?);
    }
    Ok(0)
}

pub(crate) fn vault_id_from_base(vault: &Path) -> CliResult<VaultId> {
    latest_cf_rows(vault, ColumnFamily::Base)?
        .into_values()
        .next()
        .map(|bytes| decode_constellation_base(&bytes).map(|cx| cx.vault_id))
        .transpose()?
        .ok_or_else(|| CliError::runtime("cannot infer vault id: base CF has no rows"))
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
            sst_order(Path::new("00000000000000000007-0001.sst")).unwrap()
                < sst_order(Path::new("compacted-00000000000000000007.sst")).unwrap()
        );
    }

    #[test]
    fn sst_order_rejects_unrecognized_names() {
        assert!(sst_order(Path::new("not-a-calyx-sst.sst")).is_err());
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
