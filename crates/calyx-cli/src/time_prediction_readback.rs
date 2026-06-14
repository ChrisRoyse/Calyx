use std::path::Path;
use std::str::FromStr;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::CxId;
use calyx_oracle::{CALYX_ORACLE_INSUFFICIENT, predict_next_occurrence};
use serde_json::json;

use crate::cf_read::vault_id_from_base;

pub fn readback_time_prediction(
    vault: &Path,
    cx_id: &str,
    ceiling: &str,
) -> crate::error::CliResult {
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
        Err(error) => return Err(error.to_string().into()),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
    );
    Ok(())
}
