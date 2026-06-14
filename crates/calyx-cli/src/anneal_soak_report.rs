use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use calyx_anneal::{SoakRowKind, decode_soak_row};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::sst::SstReader;
use serde_json::json;

use crate::cf_read::{hex_bytes, list_sst_files};

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = SoakReportRequest::parse(args)?;
    let readback = read_soak_rows(&request.vault, request.last)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&readback).map_err(|error| error.to_string())?
    );
    Ok(())
}

struct SoakReportRequest {
    vault: PathBuf,
    last: usize,
}

impl SoakReportRequest {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut vault = None;
        let mut last = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--vault" => {
                    vault = args.get(idx + 1).map(PathBuf::from);
                    idx += 2;
                }
                "--last" => {
                    last = Some(
                        args.get(idx + 1)
                            .ok_or_else(|| "--last requires a value".to_string())?
                            .parse::<usize>()
                            .map_err(|error| format!("invalid --last: {error}"))?,
                    );
                    idx += 2;
                }
                other => return Err(format!("unknown soak-report arg: {other}")),
            }
        }
        let last = last.unwrap_or(1);
        if last == 0 {
            return Err("--last must be positive".to_string());
        }
        Ok(Self {
            vault: vault.ok_or_else(|| "soak-report requires --vault".to_string())?,
            last,
        })
    }
}

fn read_soak_rows(vault: &Path, last: usize) -> Result<serde_json::Value, String> {
    let cf = ColumnFamily::AnnealSoak;
    let mut reports = Vec::new();
    let mut samples = Vec::new();
    let mut physical_rows = Vec::new();
    let mut logical_rows = BTreeMap::new();
    for file in list_sst_files(&vault.join("cf").join(cf.name()))? {
        let reader = SstReader::open(&file).map_err(|error| error.to_string())?;
        for row in reader.iter().map_err(|error| error.to_string())? {
            let decoded = decode_soak_row(&row.value).map_err(|error| error.to_string())?;
            let physical = json!({
                "file": file.display().to_string(),
                "key_hex": hex_bytes(&row.key),
                "value_hex": hex_bytes(&row.value),
                "value_len": row.value.len(),
                "run_id": hex_bytes(&decoded.run_id),
                "row": decoded.row,
            });
            physical_rows.push(physical);
            logical_rows.insert(row.key, decoded);
        }
    }
    for (_key, decoded) in logical_rows {
        match decoded.row {
            SoakRowKind::Report { report } => reports.push(json!({
                "run_id": hex_bytes(&decoded.run_id),
                "report": report,
            })),
            SoakRowKind::Sample { sample } => samples.push(json!({
                "run_id": hex_bytes(&decoded.run_id),
                "sample": sample,
            })),
        }
    }
    reports.sort_by_key(|value| {
        value
            .pointer("/report/ts")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    });
    samples.sort_by_key(|value| {
        value
            .pointer("/sample/query_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    });
    if last < reports.len() {
        reports.drain(0..reports.len() - last);
    }
    Ok(json!({
        "source_of_truth": "Aster anneal_soak CF SST rows under <vault>/cf/anneal_soak; physical_rows preserves duplicate raw SST bytes",
        "vault": vault.display().to_string(),
        "cf": cf.name(),
        "last": last,
        "reports": reports,
        "logical_row_count": reports.len() + samples.len(),
        "sample_row_count": samples.len(),
        "samples": samples,
        "physical_row_count": physical_rows.len(),
        "physical_rows": physical_rows,
    }))
}
