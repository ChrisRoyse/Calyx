use calyx_anneal::{HeadKind, decode_online_head, head_key};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::sst::SstReader;
use serde_json::json;
use std::path::Path;

use crate::cf_read::{hex_bytes, list_sst_files};

pub fn head_status(vault: &Path, kind_label: &str) -> crate::error::CliResult {
    let kind = HeadKind::from_label(kind_label).map_err(|error| error.to_string())?;
    let cf = ColumnFamily::AnnealHeads;
    let wanted_key = head_key(kind);
    let mut physical_rows = Vec::new();
    let mut latest = None;
    for file in list_sst_files(&vault.join("cf").join(cf.name()))? {
        let reader = SstReader::open(&file).map_err(|error| error.to_string())?;
        for row in reader.iter().map_err(|error| error.to_string())? {
            let head = decode_online_head(&row.value).map_err(|error| error.to_string())?;
            let readback = json!({
                "file": file.display().to_string(),
                "key_hex": hex_bytes(&row.key),
                "value_hex": hex_bytes(&row.value),
                "value_len": row.value.len(),
                "head": head,
            });
            if row.key == wanted_key {
                latest = Some((row.key, row.value, head));
            }
            physical_rows.push(readback);
        }
    }
    let (version, param_count, param_norm, fisher_norm, row) = match latest {
        Some((key, value, head)) => (
            json!(head.version),
            head.params.len(),
            norm(&head.params),
            norm(&head.fisher_diag),
            json!({
                "key_hex": hex_bytes(&key),
                "value_hex": hex_bytes(&value),
                "head": head,
            }),
        ),
        None => (
            json!(null),
            0,
            0.0,
            0.0,
            json!({"key_hex": hex_bytes(&wanted_key), "head": null}),
        ),
    };
    let readback = json!({
        "cf": cf.name(),
        "vault": vault.display().to_string(),
        "kind": kind.key(),
        "version": version,
        "param_count": param_count,
        "param_norm": param_norm,
        "fisher_norm": fisher_norm,
        "physical_row_count": physical_rows.len(),
        "physical_rows": physical_rows,
        "row": row,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&readback).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn norm(values: &[f32]) -> f64 {
    values
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>()
        .sqrt()
}
