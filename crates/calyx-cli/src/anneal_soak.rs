mod corpus;
mod metrics;
mod request;
mod runner;

use std::fs;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::VaultId;

use corpus::ingest_corpus;
use metrics::write_metric_outputs;
use request::SoakRequest;
use runner::run_seeded_soak;

pub(super) const DEFAULT_MIN_DOCS: usize = 50_000;

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = SoakRequest::parse(args)?;
    fs::create_dir_all(&request.metrics_dir).map_err(|error| error.to_string())?;
    let vault_id = request
        .vault_id
        .parse::<VaultId>()
        .map_err(|error| format!("CALYX_ANNEAL_SOAK_INVALID_CONFIG: {error}"))?;
    let vault = AsterVault::open(
        &request.vault,
        vault_id,
        request.vault_salt.as_bytes().to_vec(),
        VaultOptions::default(),
    )
    .map_err(|error| error.to_string())?;
    let stats = ingest_corpus(&vault, &request)?;
    let report = run_seeded_soak(&vault, &request, &stats)?;
    let evidence = write_metric_outputs(&vault, &request, &stats, &report)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&evidence).map_err(|error| error.to_string())?
    );
    Ok(())
}
