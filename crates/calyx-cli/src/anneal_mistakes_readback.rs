use calyx_anneal::{decode_mistake_entry, mistake_seq_from_key};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::sst::SstReader;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;

use crate::cf_read::{hex_bytes, list_sst_files};

pub fn readback_mistakes(vault: &Path, last: usize) -> crate::error::CliResult {
    if last == 0 {
        return Err("anneal mistakes readback requires --last > 0"
            .to_string()
            .into());
    }
    let cf = ColumnFamily::AnnealMistakes;
    let mut physical_rows = Vec::new();
    let mut rows_by_seq = BTreeMap::new();
    for file in list_sst_files(&vault.join("cf").join(cf.name()))? {
        let reader = SstReader::open(&file).map_err(|error| error.to_string())?;
        for row in reader.iter().map_err(|error| error.to_string())? {
            let seq = mistake_seq_from_key(&row.key).map_err(|error| error.to_string())?;
            let entry = decode_mistake_entry(&row.value).map_err(|error| error.to_string())?;
            let readback = json!({
                "seq": seq,
                "file": file.display().to_string(),
                "key_hex": hex_bytes(&row.key),
                "value_hex": hex_bytes(&row.value),
                "value_len": row.value.len(),
                "entry": entry,
            });
            physical_rows.push(readback.clone());
            rows_by_seq.insert(seq, readback);
        }
    }
    physical_rows.sort_by_key(|row| {
        (
            row["seq"].as_u64().unwrap_or(u64::MAX),
            row["file"].as_str().unwrap_or_default().to_string(),
        )
    });
    let physical_row_count = physical_rows.len();
    let logical_row_count = rows_by_seq.len();
    let mut rows = rows_by_seq.into_values().collect::<Vec<_>>();
    if last < rows.len() {
        rows.drain(0..rows.len() - last);
    }
    let readback = json!({
        "cf": cf.name(),
        "vault": vault.display().to_string(),
        "last": last,
        "logical_row_count": logical_row_count,
        "physical_row_count": physical_row_count,
        "physical_rows": physical_rows,
        "rows": rows,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&readback).map_err(|error| error.to_string())?
    );
    Ok(())
}
