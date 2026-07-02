mod data;
mod engine;
mod metrics;
mod request;

use std::fs;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::VaultId;

use data::ValidationData;
use engine::evaluate_emotion;
use metrics::write_metric_outputs;
use request::EmotionRequest;

use crate::error::CliError;

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = EmotionRequest::parse(args).map_err(CliError::usage)?;
    fs::create_dir_all(&request.metrics_dir)?;
    let vault_id = request.vault_id.parse::<VaultId>().map_err(|error| {
        CliError::usage(format!("CALYX_FSV_MEDIA_EMOTION_INVALID_CONFIG: {error}"))
    })?;
    let vault = AsterVault::new_durable(
        &request.vault,
        vault_id,
        request.vault_salt.as_bytes().to_vec(),
        VaultOptions::default(),
    )?;
    let data = ValidationData::load(&request.samples).map_err(CliError::runtime)?;
    let report = evaluate_emotion(&data, &request)?;
    let evidence = write_metric_outputs(&vault, &request, report)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&evidence).map_err(|error| CliError::runtime(format!(
            "serialize media emotion validation evidence: {error}"
        )))?
    );
    Ok(())
}

#[cfg(test)]
mod tests;
