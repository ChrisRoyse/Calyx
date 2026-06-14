use calyx_anneal::{decode_replay_snapshot, replay_snapshot_key};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::sst::SstReader;
use serde_json::json;
use std::path::Path;

use crate::cf_read::{hex_bytes, list_sst_files};

pub fn replay_status(vault: &Path) -> crate::error::CliResult {
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
