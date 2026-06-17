//! `calyx assay bits-validate` — labeled multi-lens bits/contract proof.
//!
//! Proves on a labeled multi-lens embedding corpus that each real lens carries
//! `bits_about` >= `--min-bits` about a grounded binary anchor, that a planted
//! representationally-redundant lens is rejected from the admitted panel, that
//! `I(panel;anchor)` is reported with a confidence interval, and that
//! per-stratum bits are present. All measurements use the real `calyx_assay`
//! estimators and persist per-lens estimates to the Assay column family.

pub(crate) mod cost;
mod data;
mod engine;
mod metrics;
mod request;
mod selection;

use cost::{LensCostMap, PanelBudgetConfig};
use data::AssayCorpus;
use engine::evaluate_corpus;
use metrics::write_metric_outputs;
use request::AssayBitsRequest;

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = AssayBitsRequest::parse(args)?;
    let corpus = AssayCorpus::load(&request)?;
    let cost = match &request.cost_json {
        Some(path) => Some(LensCostMap::load(path)?),
        None => None,
    };
    let panel_budget = match &request.panel_budget_json {
        Some(path) => Some(PanelBudgetConfig::load(path)?),
        None => None,
    };
    let report = evaluate_corpus(&corpus, &request, cost.as_ref(), panel_budget)?;
    let evidence = write_metric_outputs(&request, &report)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&evidence).map_err(|error| error.to_string())?
    );
    Ok(())
}

#[cfg(test)]
mod tests;
