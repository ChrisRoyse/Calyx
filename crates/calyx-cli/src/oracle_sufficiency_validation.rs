//! `calyx oracle sufficiency-validate` — oracle sufficiency-refusal proof.
//!
//! Proves on a labeled multi-lens embedding corpus drawn from a SWE-bench
//! problem set that a FORM-ONLY panel (text-embedding lenses over the problem's
//! surface text) is INSUFFICIENT to predict the binary oracle `test_pass_fail`
//! (did a model's patch resolve the instance) — i.e. `I(panel;oracle) < H(Y)` —
//! so the sufficiency-refusal gate FIRES. All measurements use the real
//! `calyx_assay` estimators (`logistic_probe_mi`, `entropy_bits`) and persist
//! per-lens / panel / outcome-entropy estimates to the Assay column family, then
//! reopen and load them to prove durable readback. The sufficiency bound mirrors
//! `calyx_oracle` honesty-gate semantics (honesty_gate.rs:49:
//! `sufficient = panel_bits >= anchor_entropy_bits`).
//!
//! The binding outcome is that refusal fires: if the form-only panel is
//! unexpectedly sufficient the command fails closed rather than rubber-stamping.

mod data;
mod engine;
mod metrics;
mod request;

use data::OracleCorpus;
use engine::evaluate_corpus;
use metrics::write_metric_outputs;
use request::OracleSufficiencyRequest;

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = OracleSufficiencyRequest::parse(args)?;
    let corpus = OracleCorpus::load(&request)?;
    let report = evaluate_corpus(&corpus, &request)?;
    let evidence = write_metric_outputs(&request, &report)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&evidence).map_err(|error| error.to_string())?
    );
    Ok(())
}

#[cfg(test)]
mod tests;
