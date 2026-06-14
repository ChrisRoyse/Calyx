use calyx_anneal::{FrozenCheckReport, FrozenLensReportRow};
use serde_json::json;
use std::fs;
use std::path::Path;

pub fn frozen_guard_report(path: &Path) -> crate::error::CliResult {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let report: FrozenCheckReport =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let rows = report
        .rows
        .iter()
        .map(row_json)
        .collect::<Result<Vec<_>, _>>()?;
    let readback = json!({
        "artifact": path.display().to_string(),
        "ok_count": report.ok.len(),
        "violation_count": report.violations.len(),
        "new_count": report.new_lenses.len(),
        "ok": report.ok,
        "violations": report.violations,
        "new_lenses": report.new_lenses,
        "rows": rows,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&readback).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn row_json(row: &FrozenLensReportRow) -> Result<serde_json::Value, String> {
    Ok(json!({
        "lens_id": row.lens_id,
        "known_hash_hex": row.known_hash.map(|hash| hex32(&hash)).transpose()?,
        "observed_hash_hex": hex32(&row.observed_hash)?,
        "stable": row.stable,
        "status": row.status,
    }))
}

fn hex32(bytes: &[u8; 32]) -> Result<String, String> {
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(hex_digit(byte >> 4)?);
        out.push(hex_digit(byte & 0x0f)?);
    }
    Ok(out)
}

fn hex_digit(value: u8) -> Result<char, String> {
    match value {
        0..=9 => Ok(char::from(b'0' + value)),
        10..=15 => Ok(char::from(b'a' + value - 10)),
        _ => Err("nibble out of range".to_string()),
    }
}
