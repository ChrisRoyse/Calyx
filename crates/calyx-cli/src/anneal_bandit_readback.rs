use calyx_anneal::{bandit_key, decode_config_bandit, shape_key_hash};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::sst::SstReader;
use serde_json::json;
use std::path::Path;

use crate::cf_read::{hex_bytes, list_sst_files};

pub fn bandit_status(vault: &Path, shape_key: &str) -> crate::error::CliResult {
    let cf = ColumnFamily::AnnealBandit;
    let shape_hash = shape_key_hash(shape_key);
    let wanted_key = bandit_key(shape_hash);
    let mut physical_rows = Vec::new();
    let mut latest = None;
    for file in list_sst_files(&vault.join("cf").join(cf.name()))? {
        let reader = SstReader::open(&file).map_err(|error| error.to_string())?;
        for row in reader.iter().map_err(|error| error.to_string())? {
            let bandit = decode_config_bandit(&row.value).map_err(|error| error.to_string())?;
            let status = bandit
                .status(shape_hash)
                .map_err(|error| error.to_string())?;
            let readback = json!({
                "file": file.display().to_string(),
                "key_hex": hex_bytes(&row.key),
                "value_hex": hex_bytes(&row.value),
                "value_len": row.value.len(),
                "status": status,
            });
            if row.key == wanted_key {
                latest = Some(readback.clone());
            }
            physical_rows.push(readback);
        }
    }
    let status = latest.as_ref().and_then(|row| row.get("status")).cloned();
    let readback = json!({
        "cf": cf.name(),
        "vault": vault.display().to_string(),
        "shape_key": shape_key,
        "shape_key_hash": hex_bytes(&shape_hash),
        "key_hex": hex_bytes(&wanted_key),
        "found": latest.is_some(),
        "incumbent": status.as_ref().and_then(|value| value.get("incumbent")).cloned(),
        "arm_count": status.as_ref().and_then(|value| value.get("arm_count")).cloned(),
        "arms": status.as_ref().and_then(|value| value.get("arms")).cloned(),
        "row": latest,
        "physical_row_count": physical_rows.len(),
        "physical_rows": physical_rows,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&readback).map_err(|error| error.to_string())?
    );
    Ok(())
}
