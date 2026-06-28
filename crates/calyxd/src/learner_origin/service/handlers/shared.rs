use calyx_oracle::OracleError;

use super::super::{OriginError, STATUS_UNPROCESSABLE};
pub(super) fn require_unit_interval(field: &str, value: f32) -> Result<f32, OriginError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(OriginError::bad_request(
            "CALYX_ORIGIN_INVALID_NUMBER",
            format!("{field} must be finite and within [0, 1]"),
        ))
    }
}

pub(super) fn require_nonnegative_bits(field: &str, value: f32) -> Result<f32, OriginError> {
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(OriginError::bad_request(
            "CALYX_ORIGIN_INVALID_NUMBER",
            format!("{field} must be finite and non-negative"),
        ))
    }
}

pub(super) fn oracle_origin_error(error: OracleError) -> OriginError {
    OriginError::new(
        STATUS_UNPROCESSABLE,
        "CALYX_ORIGIN_ORACLE_REJECTED",
        format!("{}: {} ({})", error.code(), error, error.remediation()),
    )
}
