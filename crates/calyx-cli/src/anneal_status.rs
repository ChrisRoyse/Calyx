use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_anneal::decode_health_value;
use calyx_aster::cf::ColumnFamily;
use calyx_aster::sst::SstReader;
use calyx_aster::vault::encode::decode_write_batch;
use calyx_aster::wal::replay_dir;

pub(crate) fn status_health(vault: &Path) -> Result<(), String> {
    if !vault.is_dir() {
        return Err(format!(
            "--vault path {} is not a directory",
            vault.display()
        ));
    }
    let mut rows = BTreeMap::<Vec<u8>, Vec<u8>>::new();
    read_sst_rows(vault, &mut rows)?;
    read_wal_rows(vault, &mut rows)?;

    if rows.is_empty() {
        println!("ANNEAL_HEALTH empty");
        return Ok(());
    }
    for value in rows.values() {
        let row = decode_health_value(value).map_err(|error| error.to_string())?;
        println!("{}: {}", row.kind, row.health);
    }
    Ok(())
}

fn read_sst_rows(vault: &Path, rows: &mut BTreeMap<Vec<u8>, Vec<u8>>) -> Result<(), String> {
    for file in list_sst_files(&vault.join("cf").join(ColumnFamily::AnnealHealth.name()))? {
        let reader = SstReader::open(&file).map_err(|error| error.to_string())?;
        for row in reader.iter().map_err(|error| error.to_string())? {
            rows.insert(row.key, row.value);
        }
    }
    Ok(())
}

fn read_wal_rows(vault: &Path, rows: &mut BTreeMap<Vec<u8>, Vec<u8>>) -> Result<(), String> {
    let wal_dir = vault.join("wal");
    if !wal_dir.is_dir() {
        return Ok(());
    }
    let replay = replay_dir(wal_dir).map_err(|error| error.to_string())?;
    if let Some(torn) = replay.torn_tail {
        return Err(torn.error().to_string());
    }
    for record in replay.records {
        for row in decode_write_batch(&record.payload).map_err(|error| error.to_string())? {
            if row.cf == ColumnFamily::AnnealHealth {
                rows.insert(row.key, row.value);
            }
        }
    }
    Ok(())
}

fn list_sst_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    if !dir.exists() {
        return Ok(files);
    }
    for entry in fs::read_dir(dir).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().and_then(|value| value.to_str()) == Some("sst") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}
