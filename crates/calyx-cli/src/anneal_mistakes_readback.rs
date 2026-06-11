use calyx_anneal::{decode_mistake_entry, mistake_seq_from_key};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::sst::SstReader;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn readback_mistakes(vault: &Path, last: usize) -> Result<(), String> {
    if last == 0 {
        return Err("anneal mistakes readback requires --last > 0".to_string());
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
