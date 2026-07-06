mod engine;
mod label_store;
mod matrix;
mod metrics;
mod plan;
mod request;
mod rows;

use calyx_core::CalyxError;

use crate::error::{CliError, CliResult};

use engine::{enforce_a37_mode, evaluate};
use metrics::write_outputs;
use request::I8binEnsembleRequest;

const FSV_REMEDIATION: &str = "inspect the streamed plan Graph CF row, label-anchor Graph CF row, vector bytes, metrics files, and Assay CF readback";

pub(crate) fn run(args: &[String]) -> CliResult {
    let request = I8binEnsembleRequest::parse(args).map_err(i8bin_card_error)?;
    request
        .ensure_fresh_outputs()
        .map_err(i8bin_card_runtime_error)?;
    let report = evaluate(&request).map_err(i8bin_card_runtime_error)?;
    enforce_a37_mode(&request, &report).map_err(i8bin_card_runtime_error)?;
    let evidence = write_outputs(&request, &report)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&evidence).map_err(|error| CliError::runtime(format!(
            "serialize i8bin ensemble card evidence: {error}"
        )))?
    );
    Ok(())
}

pub(crate) fn run_label_import(args: &[String]) -> CliResult {
    label_store::run_import(args)
}

/// Same code recovery as [`i8bin_card_error`], but uncoded strings classify as
/// runtime failures: these call sites run after argument parsing succeeded,
/// so `--help` can never be the remedy (issue #1145).
fn i8bin_card_runtime_error(error: String) -> CliError {
    match i8bin_card_error(error) {
        CliError::Usage(message) => CliError::runtime(message),
        typed => typed,
    }
}

fn i8bin_card_error(error: String) -> CliError {
    let Some((code, message)) = split_calyx_code(&error) else {
        return CliError::usage(error);
    };
    match code {
        calyx_assay::CALYX_ASSAY_PANEL_TOO_SMALL => local_error(code, message, FSV_REMEDIATION),
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES" => {
            CliError::from(CalyxError::assay_insufficient_samples(message))
        }
        "CALYX_ASSAY_LOW_SIGNAL" => CliError::from(CalyxError::assay_low_signal(message)),
        "CALYX_ASSAY_REDUNDANT" => CliError::from(CalyxError::assay_redundant(message)),
        calyx_assay::CALYX_ASSAY_ESTIMATOR_UNDERPOWERED
        | calyx_assay::CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY => {
            local_error(code, message, FSV_REMEDIATION)
        }
        _ => local_error(code, message, FSV_REMEDIATION),
    }
}

fn split_calyx_code(error: &str) -> Option<(&'static str, String)> {
    let code = error.split_once(':').map_or(error, |(code, _)| code).trim();
    let code = I8BIN_CARD_CODES
        .iter()
        .copied()
        .find(|known| *known == code)?;
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

const I8BIN_CARD_CODES: &[&str] = &[
    calyx_assay::CALYX_ASSAY_PANEL_TOO_SMALL,
    "CALYX_ASSAY_INSUFFICIENT_SAMPLES",
    "CALYX_ASSAY_LOW_SIGNAL",
    "CALYX_ASSAY_REDUNDANT",
    calyx_assay::CALYX_ASSAY_ESTIMATOR_UNDERPOWERED,
    calyx_assay::CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY,
    "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_CONFIG",
    "CALYX_FSV_ASSAY_I8BIN_CARD_NOT_FOUND",
    "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_PLAN",
    "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_ROWS",
    "CALYX_FSV_ASSAY_I8BIN_LABELS_IMPORT_IO",
    "CALYX_FSV_ASSAY_I8BIN_LABELS_IMPORT_INVALID",
    "CALYX_FSV_ASSAY_I8BIN_LABELS_INVALID",
    "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_INVALID",
    "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_EXISTS",
    "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_MISSING",
    "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_MISSING_AFTER_WRITE",
    "CALYX_FSV_ASSAY_I8BIN_LABELS_CHUNK_DB_MISSING",
    "CALYX_FSV_ASSAY_I8BIN_LABELS_CHUNK_DB_MISSING_AFTER_WRITE",
    "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_MISMATCH",
    "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_INVALID_KEY",
    "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_ENCODE",
    "CALYX_FSV_ASSAY_I8BIN_LABELS_DB_DECODE",
    "CALYX_FSV_ASSAY_I8BIN_LABELS_TARGET_MISMATCH",
    "CALYX_FSV_ASSAY_I8BIN_CARD_VECTOR_MISMATCH",
    "CALYX_FSV_ASSAY_A37_DIVERSITY_GATE_REFUSED",
    "CALYX_FSV_ASSAY_I8BIN_CARD_OUTPUT_EXISTS",
    "CALYX_FSV_ASSAY_CARD_READBACK_MISSING",
    "CALYX_FSV_ASSAY_CARD_READBACK_MISMATCH",
    "CALYX_FSV_ASSAY_NONFINITE_METRIC",
    "CALYX_FSV_PARTITIONED_RRF_PLAN_DB_INVALID",
    "CALYX_FSV_PARTITIONED_RRF_PLAN_DB_EXISTS",
    "CALYX_FSV_PARTITIONED_RRF_PLAN_DB_MISSING",
    "CALYX_FSV_PARTITIONED_RRF_PLAN_DB_MISMATCH",
    "CALYX_FSV_PARTITIONED_RRF_PLAN_DB_INVALID_KEY",
    "CALYX_FSV_PARTITIONED_RRF_PLAN_DB_ENCODE",
    "CALYX_FSV_PARTITIONED_RRF_PLAN_DB_DECODE",
];

#[cfg(test)]
mod tests;
