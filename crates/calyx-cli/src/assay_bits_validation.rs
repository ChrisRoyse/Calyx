//! `calyx assay bits-validate` — labeled multi-lens bits/contract proof.
//!
//! Proves on a labeled multi-lens embedding corpus that each real lens carries
//! `bits_about` >= `--min-bits` about a grounded binary anchor, that a planted
//! representationally-redundant lens is rejected from the admitted panel, that
//! `I(panel;anchor)` is reported with a confidence interval, and that
//! per-stratum bits are present. All measurements use the real `calyx_assay`
//! estimators and persist per-lens estimates to the Assay column family.

mod comparison;
mod correlation;
pub(crate) mod cost;
pub(crate) mod data;
mod engine;
mod metrics;
mod report;
pub(crate) mod request;
mod selection;
#[cfg(test)]
mod test_support;

use calyx_assay::{
    CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY, CALYX_ASSAY_ESTIMATOR_UNDERPOWERED,
    CALYX_ASSAY_INVALID_RESOURCE, CALYX_ASSAY_RESOURCE_BUDGET_EXCEEDED, CALYX_ASSAY_UNRESOLVED,
};
use calyx_core::{CalyxError, CalyxErrorCode};
use cost::{LensCostMap, PanelBudgetConfig};
use data::AssayCorpus;
use engine::evaluate_corpus;
use metrics::write_metric_outputs;
use request::AssayBitsRequest;

use crate::assay_anchor_audit::CALYX_FSV_ASSAY_TRIVIAL_ANCHOR;
use crate::error::{CliError, CliResult};

const FSV_REMEDIATION: &str = "inspect the assay corpus, metrics files, and Assay CF readback";
const RESOURCE_REMEDIATION: &str = "adjust the lens cost or panel budget inputs and rerun";
const UNRESOLVED_REMEDIATION: &str =
    "collect more grouped anchors and re-run multi-seed Assay measurement";
const INVALID_RESOURCE_REMEDIATION: &str = "fix the non-finite or negative resource input";

pub(crate) fn run(args: &[String]) -> CliResult {
    let request = AssayBitsRequest::parse(args).map_err(assay_cli_error)?;
    let corpus = AssayCorpus::load(&request).map_err(assay_cli_error)?;
    let cost = match &request.cost_json {
        Some(path) => Some(LensCostMap::load(path).map_err(assay_cli_error)?),
        None => None,
    };
    let panel_budget = match &request.panel_budget_json {
        Some(path) => Some(PanelBudgetConfig::load(path).map_err(assay_cli_error)?),
        None => None,
    };
    let report =
        evaluate_corpus(&corpus, &request, cost.as_ref(), panel_budget).map_err(assay_cli_error)?;
    let evidence = write_metric_outputs(&request, &report).map_err(assay_cli_error)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&evidence).map_err(CliError::from)?
    );
    Ok(())
}

pub(crate) fn calyx_error_detail(error: CalyxError) -> String {
    format!("{}: {}", error.code, error.message)
}

fn assay_cli_error(error: String) -> CliError {
    let Some((code, message)) = split_calyx_code(&error) else {
        return CliError::usage(error);
    };
    match code {
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES" => {
            CliError::from(CalyxError::assay_insufficient_samples(message))
        }
        "CALYX_ASSAY_LOW_SIGNAL" => CliError::from(CalyxError::assay_low_signal(message)),
        "CALYX_ASSAY_REDUNDANT" => CliError::from(CalyxError::assay_redundant(message)),
        CALYX_ASSAY_UNRESOLVED => local_error(code, message, UNRESOLVED_REMEDIATION),
        CALYX_ASSAY_ESTIMATOR_UNDERPOWERED => local_error(code, message, FSV_REMEDIATION),
        CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY => local_error(code, message, FSV_REMEDIATION),
        CALYX_ASSAY_RESOURCE_BUDGET_EXCEEDED => local_error(code, message, RESOURCE_REMEDIATION),
        CALYX_ASSAY_INVALID_RESOURCE => local_error(code, message, INVALID_RESOURCE_REMEDIATION),
        "CALYX_FSV_ASSAY_INVALID_CONFIG" => local_error(code, message, FSV_REMEDIATION),
        "CALYX_FSV_ASSAY_CORPUS_NOT_FOUND" => local_error(code, message, FSV_REMEDIATION),
        "CALYX_FSV_ASSAY_INVALID_CORPUS" => local_error(code, message, FSV_REMEDIATION),
        "CALYX_FSV_ASSAY_INVALID_COST" => local_error(code, message, FSV_REMEDIATION),
        "CALYX_FSV_ASSAY_COST_IO" => local_error(code, message, FSV_REMEDIATION),
        "CALYX_FSV_ASSAY_MISSING_COST" => local_error(code, message, FSV_REMEDIATION),
        "CALYX_FSV_ASSAY_INVALID_PANEL_BUDGET" => local_error(code, message, FSV_REMEDIATION),
        "CALYX_FSV_ASSAY_PANEL_BUDGET_IO" => local_error(code, message, FSV_REMEDIATION),
        "CALYX_FSV_ASSAY_SINGLE_CLASS_ANCHOR" => local_error(code, message, FSV_REMEDIATION),
        "CALYX_FSV_ASSAY_BITS_CI_BELOW_THRESHOLD" => local_error(code, message, FSV_REMEDIATION),
        "CALYX_FSV_ASSAY_REDUNDANT_LENS_NOT_REJECTED" => {
            local_error(code, message, FSV_REMEDIATION)
        }
        "CALYX_FSV_ASSAY_EMPTY_PANEL" => local_error(code, message, FSV_REMEDIATION),
        "CALYX_FSV_ASSAY_PANEL_CONTROL_NOT_BEATEN" => local_error(code, message, FSV_REMEDIATION),
        "CALYX_FSV_ASSAY_NONFINITE_METRIC" => local_error(code, message, FSV_REMEDIATION),
        "CALYX_FSV_ASSAY_MISSING_VERDICT_METADATA" => local_error(code, message, FSV_REMEDIATION),
        CALYX_FSV_ASSAY_TRIVIAL_ANCHOR => local_error(
            code,
            message,
            "use a validity-audited non-linguistic outcome anchor",
        ),
        _ => CliError::usage(error),
    }
}

fn split_calyx_code(error: &str) -> Option<(&'static str, String)> {
    let code = error.split_once(':').map_or(error, |(code, _)| code).trim();
    if !code.starts_with("CALYX_") {
        return None;
    }
    let code = static_assay_code(code)?;
    let message = error
        .strip_prefix(code)
        .and_then(|rest| rest.strip_prefix(':'))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(code)
        .to_string();
    Some((code, message))
}

fn static_assay_code(code: &str) -> Option<&'static str> {
    CalyxErrorCode::AssayInsufficientSamples
        .code()
        .eq(code)
        .then_some(CalyxErrorCode::AssayInsufficientSamples.code())
        .or_else(|| {
            CalyxErrorCode::AssayLowSignal
                .code()
                .eq(code)
                .then_some(CalyxErrorCode::AssayLowSignal.code())
        })
        .or_else(|| {
            CalyxErrorCode::AssayRedundant
                .code()
                .eq(code)
                .then_some(CalyxErrorCode::AssayRedundant.code())
        })
        .or_else(|| {
            ASSAY_LOCAL_CODES
                .iter()
                .copied()
                .find(|known| *known == code)
        })
}

fn local_error(code: &'static str, message: String, remediation: &'static str) -> CliError {
    CliError::from(CalyxError {
        code,
        message,
        remediation,
    })
}

const ASSAY_LOCAL_CODES: &[&str] = &[
    CALYX_ASSAY_UNRESOLVED,
    CALYX_ASSAY_ESTIMATOR_UNDERPOWERED,
    CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY,
    CALYX_ASSAY_RESOURCE_BUDGET_EXCEEDED,
    CALYX_ASSAY_INVALID_RESOURCE,
    "CALYX_FSV_ASSAY_INVALID_CONFIG",
    "CALYX_FSV_ASSAY_CORPUS_NOT_FOUND",
    "CALYX_FSV_ASSAY_INVALID_CORPUS",
    "CALYX_FSV_ASSAY_INVALID_COST",
    "CALYX_FSV_ASSAY_COST_IO",
    "CALYX_FSV_ASSAY_MISSING_COST",
    "CALYX_FSV_ASSAY_INVALID_PANEL_BUDGET",
    "CALYX_FSV_ASSAY_PANEL_BUDGET_IO",
    "CALYX_FSV_ASSAY_SINGLE_CLASS_ANCHOR",
    "CALYX_FSV_ASSAY_BITS_CI_BELOW_THRESHOLD",
    "CALYX_FSV_ASSAY_REDUNDANT_LENS_NOT_REJECTED",
    "CALYX_FSV_ASSAY_EMPTY_PANEL",
    "CALYX_FSV_ASSAY_PANEL_CONTROL_NOT_BEATEN",
    "CALYX_FSV_ASSAY_NONFINITE_METRIC",
    "CALYX_FSV_ASSAY_MISSING_VERDICT_METADATA",
    CALYX_FSV_ASSAY_TRIVIAL_ANCHOR,
];

#[cfg(test)]
mod power_gate_tests;
#[cfg(test)]
mod single_lens_tests;
#[cfg(test)]
mod tests;
