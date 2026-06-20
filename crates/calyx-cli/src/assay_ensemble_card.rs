mod engine;
mod metrics;
mod request;

use calyx_assay::{
    CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY, CALYX_ASSAY_ESTIMATOR_UNDERPOWERED,
    CALYX_ASSAY_PANEL_TOO_SMALL,
};
use calyx_core::CalyxError;

use crate::error::{CliError, CliResult};

use engine::evaluate;
use metrics::write_outputs;
use request::EnsembleCardRequest;

const FSV_REMEDIATION: &str = "inspect the ensemble card artifact and Assay CF readback";

pub(crate) fn run(args: &[String]) -> CliResult {
    let request = EnsembleCardRequest::parse(args).map_err(ensemble_cli_error)?;
    let report = evaluate(&request).map_err(ensemble_cli_error)?;
    let evidence = write_outputs(&request, &report).map_err(ensemble_cli_error)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&evidence).map_err(CliError::from)?
    );
    Ok(())
}

fn ensemble_cli_error(error: String) -> CliError {
    let Some((code, message)) = split_calyx_code(&error) else {
        return CliError::usage(error);
    };
    match code {
        CALYX_ASSAY_PANEL_TOO_SMALL => local_error(code, message, FSV_REMEDIATION),
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES" => {
            CliError::from(CalyxError::assay_insufficient_samples(message))
        }
        "CALYX_ASSAY_LOW_SIGNAL" => CliError::from(CalyxError::assay_low_signal(message)),
        "CALYX_ASSAY_REDUNDANT" => CliError::from(CalyxError::assay_redundant(message)),
        CALYX_ASSAY_ESTIMATOR_UNDERPOWERED | CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY => {
            local_error(code, message, FSV_REMEDIATION)
        }
        "CALYX_FSV_ASSAY_INVALID_CONFIG" | "CALYX_FSV_ASSAY_CORPUS_NOT_FOUND" => {
            local_error(code, message, FSV_REMEDIATION)
        }
        "CALYX_FSV_ASSAY_INVALID_CORPUS" if message.contains("samples") => {
            CliError::from(CalyxError::assay_insufficient_samples(message))
        }
        "CALYX_FSV_ASSAY_INVALID_CORPUS"
        | "CALYX_FSV_ASSAY_NONFINITE_METRIC"
        | "CALYX_FSV_ASSAY_CARD_READBACK_MISSING"
        | "CALYX_FSV_ASSAY_CARD_READBACK_MISMATCH" => local_error(code, message, FSV_REMEDIATION),
        _ => CliError::usage(error),
    }
}

fn split_calyx_code(error: &str) -> Option<(&'static str, String)> {
    let code = error.split_once(':').map_or(error, |(code, _)| code).trim();
    let code = ASSAY_ENSEMBLE_CODES
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

const ASSAY_ENSEMBLE_CODES: &[&str] = &[
    CALYX_ASSAY_PANEL_TOO_SMALL,
    "CALYX_ASSAY_INSUFFICIENT_SAMPLES",
    "CALYX_ASSAY_LOW_SIGNAL",
    "CALYX_ASSAY_REDUNDANT",
    CALYX_ASSAY_ESTIMATOR_UNDERPOWERED,
    CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY,
    "CALYX_FSV_ASSAY_INVALID_CONFIG",
    "CALYX_FSV_ASSAY_CORPUS_NOT_FOUND",
    "CALYX_FSV_ASSAY_INVALID_CORPUS",
    "CALYX_FSV_ASSAY_NONFINITE_METRIC",
    "CALYX_FSV_ASSAY_CARD_READBACK_MISSING",
    "CALYX_FSV_ASSAY_CARD_READBACK_MISMATCH",
];

#[cfg(test)]
mod tests;
