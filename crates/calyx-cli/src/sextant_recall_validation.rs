mod data;
mod engine;
mod ir;
mod metrics;
mod panel;
mod real;
mod real_output;
mod real_types;
mod request;
mod rerank;

use std::fs;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::VaultId;

use crate::error::CliError;
use data::ValidationData;
use engine::{build_engine, evaluate_recall};
use metrics::write_metric_outputs;
use real::run_real_panel;
use request::RecallRequest;

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = RecallRequest::parse(args).map_err(CliError::usage)?;
    let data = ValidationData::load(&request).map_err(CliError::runtime)?;
    fs::create_dir_all(&request.metrics_dir)?;
    let vault_id = request
        .vault_id
        .parse::<VaultId>()
        .map_err(|error| CliError::usage(format!("CALYX_FSV_SEXTANT_INVALID_CONFIG: {error}")))?;
    if request.real_panel_enabled() {
        let evidence = run_real_panel(&request, &data, vault_id)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&evidence).map_err(|error| CliError::runtime(format!(
                "serialize sextant recall evidence: {error}"
            )))?
        );
        return Ok(());
    }
    let vault = AsterVault::new_durable(
        &request.vault,
        vault_id,
        request.vault_salt.as_bytes().to_vec(),
        VaultOptions::default(),
    )?;
    let indexed = build_engine(&vault, &data)?;
    let report = evaluate_recall(&indexed.engine, &data, &request, &indexed)?;
    let evidence = write_metric_outputs(&vault, &request, report)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&evidence).map_err(|error| CliError::runtime(format!(
            "serialize sextant recall evidence: {error}"
        )))?
    );
    Ok(())
}

#[cfg(test)]
mod tests;
