use calyx_core::CalyxError;

use super::{CALYX_TRIPWIRE_INVALID_CONFIG, CALYX_TRIPWIRE_INVALID_METRIC, TripwireMetric};

pub(super) fn invalid_metric(metric: TripwireMetric, value: f64) -> CalyxError {
    CalyxError {
        code: CALYX_TRIPWIRE_INVALID_METRIC,
        message: format!("{} metric value must be finite, got {value}", metric.key()),
        remediation: "drop the Anneal candidate and re-measure the guarded metric",
    }
}

pub(super) fn invalid_config(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_TRIPWIRE_INVALID_CONFIG,
        message: message.into(),
        remediation: "fix vault .anneal/tripwire.toml before running Anneal",
    }
}
