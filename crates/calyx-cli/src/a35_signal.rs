use calyx_core::CalyxError;
use calyx_registry::{CapabilitySignalKind, LensRuntime, signal_kind_from_runtime};

use crate::error::{CliError, CliResult};

pub(crate) const LEARNED_SIGNAL_KIND: &str = "learned_encoder";
pub(crate) const DETERMINISTIC_CONTENT_SIGNAL_KIND: &str = "deterministic_content_feature";

pub(crate) fn runtime_signal_kind(runtime: &LensRuntime) -> CapabilitySignalKind {
    signal_kind_from_runtime(runtime)
}

pub(crate) fn runtime_signal_kind_name(runtime: &LensRuntime) -> &'static str {
    runtime_signal_kind(runtime).as_str()
}

pub(crate) fn require_countable_content_signal_kind(
    lens_name: &str,
    signal_kind: &str,
    gate: &'static str,
) -> CliResult {
    if is_temporal_sidecar_name(lens_name) {
        return Err(signal_error(
            "CALYX_FSV_A35_TEMPORAL_SIDECAR_NOT_CONTENT",
            format!(
                "{gate} lens {lens_name} is a temporal/as-of sidecar; A35 counts content lenses only"
            ),
            "keep temporal/time-capture lanes as the forward/backward/as-of sidecar and provide at least ten separate content lenses",
        ));
    }
    if signal_kind == LEARNED_SIGNAL_KIND || signal_kind == DETERMINISTIC_CONTENT_SIGNAL_KIND {
        return Ok(());
    }
    Err(signal_error(
        "CALYX_FSV_A35_NON_LEARNED_LENS",
        format!(
            "{gate} lens {lens_name} has signal_kind={signal_kind}; A35 requires learned_encoder or deterministic_content_feature content lenses"
        ),
        "use real frozen learned encoders or explicitly typed deterministic content-feature lenses; legacy algorithmic, placeholder, unknown, and temporal lenses are diagnostic/sidecar only",
    ))
}

pub(crate) fn require_recorded_countable_content_signal_kind<'a>(
    lens_name: &str,
    signal_kind: Option<&'a str>,
    gate: &'static str,
) -> CliResult<&'a str> {
    let Some(signal_kind) = signal_kind.filter(|value| !value.trim().is_empty()) else {
        return Err(signal_error(
            "CALYX_FSV_A35_SIGNAL_KIND_REQUIRED",
            format!("{gate} lens {lens_name} is missing signal_kind"),
            "regenerate the A35 plan/export with current Calyx so every counted content lens records signal_kind",
        ));
    };
    require_countable_content_signal_kind(lens_name, signal_kind, gate)?;
    Ok(signal_kind)
}

fn is_temporal_sidecar_name(lens_name: &str) -> bool {
    let name = lens_name.to_ascii_lowercase();
    name.contains("temporal")
        || name.contains("as-of")
        || name.contains("as_of")
        || name.contains("time-control")
        || name.contains("time_control")
        || name.contains("time-capture")
        || name.contains("time_capture")
        || name.contains("time-manipulation")
        || name.contains("time_manipulation")
}

fn signal_error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> CliError {
    CliError::Calyx(CalyxError {
        code,
        message: message.into(),
        remediation,
    })
}
