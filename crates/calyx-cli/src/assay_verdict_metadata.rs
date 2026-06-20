use calyx_assay::{EstimateBound, MiEstimate, PowerCalibrationStatus};

pub(crate) fn estimate_bound_name(bound: EstimateBound) -> &'static str {
    match bound {
        EstimateBound::LowerBound => "lower_bound",
        EstimateBound::Point => "point",
        EstimateBound::UpperBound => "upper_bound",
    }
}

pub(crate) fn calibration_status(estimate: &MiEstimate) -> Option<String> {
    estimate
        .power_calibration
        .as_ref()
        .map(|calibration| match calibration.status {
            PowerCalibrationStatus::Passed => "passed",
            PowerCalibrationStatus::Underpowered => "underpowered",
        })
        .map(str::to_string)
}

pub(crate) fn calibration_recovery_ratio(estimate: &MiEstimate) -> Option<f32> {
    estimate
        .power_calibration
        .as_ref()
        .map(|calibration| calibration.recovery_ratio)
}

pub(crate) fn calibration_recovered_bits(estimate: &MiEstimate) -> Option<f32> {
    estimate
        .power_calibration
        .as_ref()
        .map(|calibration| calibration.recovered_bits)
}

pub(crate) fn calibration_planted_bits(estimate: &MiEstimate) -> Option<f32> {
    estimate
        .power_calibration
        .as_ref()
        .map(|calibration| calibration.planted_bits)
}
