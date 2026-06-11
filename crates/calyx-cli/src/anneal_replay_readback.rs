use calyx_anneal::{decode_replay_snapshot, replay_snapshot_key};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::sst::SstReader;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

pub fn replay_status(vault: &Path) -> Result<(), String> {
    let cf = ColumnFamily::AnnealReplay;
    let snapshot_key = replay_snapshot_key();
    let mut physical_rows = Vec::new();
    let mut latest = None;
    for file in list_sst_files(&vault.join("cf").join(cf.name()))? {
        let reader = SstReader::open(&file).map_err(|error| error.to_string())?;
        for row in reader.iter().map_err(|error| error.to_string())? {
            let snapshot = decode_replay_snapshot(&row.value).map_err(|error| error.to_string())?;
            let readback = json!({
                "file": file.display().to_string(),
                "key_hex": hex_bytes(&row.key),
                "value_hex": hex_bytes(&row.value),
                "value_len": row.value.len(),
                "snapshot": snapshot.clone(),
            });
            physical_rows.push(readback);
            if row.key == snapshot_key {
                latest = Some((row.key, row.value, snapshot));
            }
        }
    }
    let (capacity, len, top_surprises, rows) = match latest {
        Some((key, value, snapshot)) => {
            let mut entries = snapshot.entries;
            entries.sort_by(|left, right| right.cmp(left));
            let top_surprises = entries
                .iter()
                .take(5)
                .map(|entry| entry.surprise)
                .collect::<Vec<_>>();
            let rows = json!({
                "key_hex": hex_bytes(&key),
                "value_hex": hex_bytes(&value),
                "entries": entries,
            });
            (json!(snapshot.capacity), entries.len(), top_surprises, rows)
        }
        None => (
            json!(null),
            0,
            Vec::new(),
            json!({"key_hex": hex_bytes(&snapshot_key), "entries": []}),
        ),
    };
    let readback = json!({
        "cf": cf.name(),
        "vault": vault.display().to_string(),
        "len": len,
        "capacity": capacity,
        "top_surprises": top_surprises,
        "physical_row_count": physical_rows.len(),
        "physical_rows": physical_rows,
        "rows": rows,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&readback).map_err(|error| error.to_string())?
    );
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
