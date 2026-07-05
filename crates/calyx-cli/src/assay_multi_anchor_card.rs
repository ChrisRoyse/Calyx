mod engine;
pub(crate) mod model;
mod request;
mod write;

#[cfg(test)]
mod tests;

use calyx_core::CalyxError;

use crate::error::{CliError, CliResult};

use engine::evaluate;
use request::Request;
use write::write_outputs;

const CODE_INVALID_CONFIG: &str = "CALYX_FSV_ASSAY_MULTI_ANCHOR_INVALID_CONFIG";
const CODE_INVALID_REPORT: &str = "CALYX_FSV_ASSAY_MULTI_ANCHOR_INVALID_REPORT";
const CODE_OUTPUT_EXISTS: &str = "CALYX_FSV_ASSAY_MULTI_ANCHOR_OUTPUT_EXISTS";
const CODE_READBACK_MISMATCH: &str = "CALYX_FSV_ASSAY_MULTI_ANCHOR_READBACK_MISMATCH";
const CODE_GATE_REFUSED: &str = "CALYX_FSV_ASSAY_MULTI_ANCHOR_GATE_REFUSED";
const REMEDIATION: &str =
    "inspect the input EnsembleCards, multi-anchor report, and readback hashes";

pub(crate) fn run(args: &[String]) -> CliResult {
    let request = Request::parse(args).map_err(multi_anchor_error)?;
    request
        .ensure_fresh_output()
        .map_err(multi_anchor_runtime_error)?;
    let report = evaluate(&request).map_err(multi_anchor_runtime_error)?;
    if request.mode.requires_gate() && !report.gate_passed {
        return Err(multi_anchor_error(format!(
            "{CODE_GATE_REFUSED}: multi-anchor A37 requires status={} but got {}; passing_lenses={}/{} weakest_lens={} best_marginal_bits={:.6}",
            calyx_assay::A37_DIVERSITY_GATE_PASSED,
            report.status,
            report.passing_lens_count,
            report.lens_count,
            report.weakest_lens,
            report.min_best_marginal_bits
        )));
    }
    let evidence = write_outputs(&request, &report).map_err(multi_anchor_runtime_error)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&evidence).map_err(|error| {
            CliError::runtime(format!("serialize multi-anchor evidence: {error}"))
        })?
    );
    Ok(())
}

/// Same code recovery as [`multi_anchor_error`], but uncoded strings classify as
/// runtime failures: these call sites run after argument parsing succeeded,
/// so `--help` can never be the remedy (issue #1145).
fn multi_anchor_runtime_error(error: String) -> CliError {
    match multi_anchor_error(error) {
        CliError::Usage(message) => CliError::runtime(message),
        typed => typed,
    }
}

fn multi_anchor_error(error: String) -> CliError {
    let raw_code = error
        .split_once(':')
        .map_or(error.as_str(), |(code, _)| code)
        .trim();
    let code = match raw_code {
        calyx_assay::CALYX_ASSAY_PANEL_TOO_SMALL => calyx_assay::CALYX_ASSAY_PANEL_TOO_SMALL,
        CODE_INVALID_CONFIG => CODE_INVALID_CONFIG,
        CODE_INVALID_REPORT => CODE_INVALID_REPORT,
        CODE_OUTPUT_EXISTS => CODE_OUTPUT_EXISTS,
        CODE_READBACK_MISMATCH => CODE_READBACK_MISMATCH,
        CODE_GATE_REFUSED => CODE_GATE_REFUSED,
        "CALYX_FSV_A37_ADMISSION_DB_MISSING" => "CALYX_FSV_A37_ADMISSION_DB_MISSING",
        "CALYX_FSV_A37_ADMISSION_DB_MISMATCH" => "CALYX_FSV_A37_ADMISSION_DB_MISMATCH",
        "CALYX_FSV_A37_ADMISSION_DB_INVALID_KEY" => "CALYX_FSV_A37_ADMISSION_DB_INVALID_KEY",
        "CALYX_FSV_A37_ADMISSION_DB_ENCODE" => "CALYX_FSV_A37_ADMISSION_DB_ENCODE",
        _ => return CliError::usage(error),
    };
    CliError::from(CalyxError {
        code,
        message: error
            .strip_prefix(code)
            .and_then(|rest| rest.strip_prefix(':'))
            .map(str::trim)
            .unwrap_or(&error)
            .to_string(),
        remediation: REMEDIATION,
    })
}
