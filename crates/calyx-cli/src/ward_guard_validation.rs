//! `calyx ward guard-validate` — Ward injection-block validation (PH70 T04).
//!
//! Feeds a fine-tuned classifier's per-example guard scores into Ward's REAL
//! conformal tau-calibration (`calyx_ward::calibrate_slot`) and gates the
//! injection-block claim on BOTH the held-out injection-block-rate AND the
//! benign false-reject-rate (FRR). This replaces the degenerate
//! cosine-to-centroid guard (issue #693) and closes its blind spot: that guard
//! never gated benign FRR.
//!
//! The eval split is the one the classifier never trained on; rows of that split
//! are deterministically halved into an honest calibration subset and an honest
//! held-out subset by a stable per-row hash, so both subsets are out-of-sample.
//!
//! Per-example guard verdicts are persisted to `ward_guard_verdicts.jsonl` as the
//! independently-readable source of truth (no `ColumnFamily::GuardVerdicts`
//! exists — see the comment in `engine.rs`).

mod data;
mod engine;
mod metrics;
mod request;

use data::ScoreCorpus;
use engine::evaluate;
use metrics::write_metric_outputs;
use request::WardGuardRequest;

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = WardGuardRequest::parse(args)?;
    let corpus = ScoreCorpus::load(&request)?;
    let report = evaluate(&corpus, &request)?;
    let evidence = write_metric_outputs(&request, &report)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&evidence).map_err(|error| error.to_string())?
    );
    Ok(())
}

#[cfg(test)]
mod tests;
