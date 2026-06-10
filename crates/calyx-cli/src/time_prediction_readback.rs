use std::path::Path;
use std::str::FromStr;

use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::sst::SstReader;
use calyx_aster::vault::encode::{decode_constellation_base, decode_write_batch};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::wal::replay_dir;
use calyx_core::{CxId, VaultId};
use calyx_oracle::{CALYX_ORACLE_INSUFFICIENT, predict_next_occurrence};
use serde_json::json;

pub fn readback_time_prediction(vault: &Path, cx_id: &str, ceiling: &str) -> Result<(), String> {
    let cx_id = CxId::from_str(cx_id).map_err(|error| format!("invalid --cx-id: {error}"))?;
    let confidence_ceiling = ceiling
        .parse::<f32>()
        .map_err(|error| format!("invalid --confidence-ceiling: {error}"))?;
    let vault_id = vault_id_from_base(vault)?;
    let store = AsterVault::open(
        vault,
        vault_id,
        b"calyx-time-prediction-readback".to_vec(),
        VaultOptions::default(),
    )
    .map_err(|error| error.to_string())?;
    let value = match predict_next_occurrence(&store, cx_id, confidence_ceiling) {
        Ok(prediction) => json!({
            "vault": vault.display().to_string(),
            "cx_id": cx_id,
            "confidence_ceiling": confidence_ceiling,
            "sufficient": true,
            "prediction": prediction,
            "error": null,
        }),
        Err(error) if error.code == CALYX_ORACLE_INSUFFICIENT => json!({
            "vault": vault.display().to_string(),
            "cx_id": cx_id,
            "confidence_ceiling": confidence_ceiling,
            "sufficient": false,
            "prediction": null,
            "error": error,
        }),
        Err(error) => return Err(error.to_string()),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn vault_id_from_base(vault: &Path) -> Result<VaultId, String> {
    let base_rows = latest_base_rows(vault)?;
    if let Some(value) = base_rows.into_iter().next() {
        return decode_constellation_base(&value)
            .map(|cx| cx.vault_id)
            .map_err(|error| error.to_string());
    }
    Err("cannot infer vault id: base CF has no rows".to_string())
}

fn latest_base_rows(vault: &Path) -> Result<Vec<Vec<u8>>, String> {
    let mut rows = Vec::new();
    for file in list_sst_files(&vault.join("cf").join(ColumnFamily::Base.name()))? {
        let reader = SstReader::open(&file).map_err(|error| error.to_string())?;
        for row in reader.iter().map_err(|error| error.to_string())? {
            if row.key.len() == base_key(CxId::from_bytes([0; 16])).len() {
                rows.push(row.value);
            }
        }
    }
    let replay = replay_dir(vault.join("wal")).map_err(|error| error.to_string())?;
    for record in replay.records {
        for row in decode_write_batch(&record.payload).map_err(|error| error.to_string())? {
            if row.cf == ColumnFamily::Base {
                rows.push(row.value);
            }
        }
    }
    Ok(rows)
}

fn list_sst_files(dir: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let mut files = Vec::new();
    if !dir.exists() {
        return Ok(files);
    }
    for entry in std::fs::read_dir(dir).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().is_some_and(|ext| ext == "sst") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}
