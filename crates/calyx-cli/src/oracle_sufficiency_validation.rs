//! `calyx oracle sufficiency-validate` — oracle sufficiency-refusal proof.
//!
//! Proves on a labeled multi-lens embedding corpus drawn from a SWE-bench
//! problem set that a FORM-ONLY panel (text-embedding lenses over the problem's
//! surface text) is not proven sufficient to predict the binary oracle
//! `test_pass_fail` (did a model's patch resolve the instance). All
//! verdict-bearing MI measurements use the real `calyx_assay` calibrated
//! logistic probe with a planted-signal power gate and persist per-lens / panel
//! / outcome-entropy estimates to the Assay column family, then reopen and load
//! them to prove durable readback. Sufficiency is claimed only when the
//! lower-bound basis `ci_low >= H(Y)`.
//!
//! The binding outcome is that refusal fires: if the form-only panel is
//! unexpectedly sufficient the command fails closed rather than rubber-stamping.

mod data;
mod engine;
mod metrics;
mod request;

use calyx_assay::{CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY, CALYX_ASSAY_ESTIMATOR_UNDERPOWERED};
use calyx_core::CalyxError;
use data::OracleCorpus;
use engine::evaluate_corpus;
use metrics::write_metric_outputs;
use request::OracleSufficiencyRequest;

use crate::error::{CliError, CliResult};

const FSV_REMEDIATION: &str = "inspect the oracle corpus, metrics files, and Assay CF readback";

pub(crate) fn run(args: &[String]) -> CliResult {
    let request = OracleSufficiencyRequest::parse(args).map_err(oracle_cli_error)?;
    let corpus = OracleCorpus::load(&request).map_err(oracle_cli_error)?;
    let report = evaluate_corpus(&corpus, &request).map_err(oracle_cli_error)?;
    let evidence = write_metric_outputs(&request, &report).map_err(oracle_cli_error)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&evidence).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn oracle_cli_error(error: String) -> CliError {
    let Some((code, message)) = split_calyx_code(&error) else {
        return CliError::usage(error);
    };
    local_error(code, message, FSV_REMEDIATION)
}

fn split_calyx_code(error: &str) -> Option<(&'static str, String)> {
    let code = error.split_once(':').map_or(error, |(code, _)| code).trim();
    let code = ORACLE_CODES.iter().copied().find(|known| *known == code)?;
    let message = error
        .strip_prefix(code)
        .and_then(|rest| rest.strip_prefix(':'))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(code)
        .to_string();
    Some((code, message))
}

fn local_error(code: &'static str, message: String, remediation: &'static str) -> CliError {
    CliError::from(CalyxError {
        code,
        message,
        remediation,
    })
}

const ORACLE_CODES: &[&str] = &[
    CALYX_ASSAY_ESTIMATOR_UNDERPOWERED,
    CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY,
    "CALYX_ASSAY_INSUFFICIENT_SAMPLES",
    "CALYX_ASSAY_LOW_SIGNAL",
    "CALYX_FSV_ORACLE_INVALID_CONFIG",
    "CALYX_FSV_ORACLE_CORPUS_NOT_FOUND",
    "CALYX_FSV_ORACLE_INVALID_CORPUS",
    "CALYX_FSV_ORACLE_PANEL_UNEXPECTEDLY_SUFFICIENT",
    "CALYX_FSV_ORACLE_REFUSAL_DID_NOT_FIRE",
    "CALYX_FSV_ORACLE_NONFINITE_METRIC",
    "CALYX_FSV_ORACLE_MISSING_VERDICT_METADATA",
];

#[cfg(test)]
mod tests;
