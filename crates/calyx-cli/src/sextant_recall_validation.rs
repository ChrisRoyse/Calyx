mod data;
mod engine;
mod metrics;
mod request;

use std::fs;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::VaultId;

use data::ValidationData;
use engine::{build_engine, evaluate_recall};
use metrics::write_metric_outputs;
use request::RecallRequest;

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = RecallRequest::parse(args)?;
    fs::create_dir_all(&request.metrics_dir).map_err(|error| error.to_string())?;
    let vault_id = request
        .vault_id
        .parse::<VaultId>()
        .map_err(|error| format!("CALYX_FSV_SEXTANT_INVALID_CONFIG: {error}"))?;
    let vault = AsterVault::new_durable(
        &request.vault,
        vault_id,
        request.vault_salt.as_bytes().to_vec(),
        VaultOptions::default(),
    )
    .map_err(|error| error.to_string())?;
    let data = ValidationData::load(&request)?;
    let indexed = build_engine(&vault, &data)?;
    let report = evaluate_recall(&indexed.engine, &data, &request, &indexed)?;
    let evidence = write_metric_outputs(&vault, &request, report)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&evidence).map_err(|error| error.to_string())?
    );
    Ok(())
}

#[cfg(test)]
mod tests;
