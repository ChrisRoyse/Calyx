use crate::Result;

use super::types::CompressionSlotMeasurement;
use super::validate::reject_if;

pub(crate) fn validate_slot_contract(
    measurement: &CompressionSlotMeasurement,
    bits_delta: f64,
    guard_far_delta: f64,
    guard_frr_delta: f64,
    kernel_only_recall_delta: f64,
) -> Result<bool> {
    reject_if(
        !cosine_error_within_bound(measurement),
        &measurement.slot_id,
        format!(
            "cosine error {:.8} exceeds bound {:.8}",
            measurement.achieved_cosine_error, measurement.max_cosine_error
        ),
    )?;
    reject_if(
        !bits_delta_within_bound(measurement, bits_delta),
        &measurement.slot_id,
        format!(
            "bits delta {:.8} below bound {:.8}",
            bits_delta, measurement.min_bits_delta
        ),
    )?;
    reject_if(
        !guard_far_delta_within_bound(measurement, guard_far_delta),
        &measurement.slot_id,
        format!(
            "guard FAR delta {:.8} exceeds bound {:.8}",
            guard_far_delta, measurement.max_guard_far_delta
        ),
    )?;
    reject_if(
        !guard_frr_delta_within_bound(measurement, guard_frr_delta),
        &measurement.slot_id,
        format!(
            "guard FRR delta {:.8} exceeds bound {:.8}",
            guard_frr_delta, measurement.max_guard_frr_delta
        ),
    )?;
    reject_if(
        !kernel_recall_delta_within_bound(measurement, kernel_only_recall_delta),
        &measurement.slot_id,
        format!(
            "kernel-only recall delta {:.8} below bound {:.8}",
            kernel_only_recall_delta, measurement.min_kernel_recall_delta
        ),
    )?;
    Ok(slot_contract_passed(
        measurement,
        bits_delta,
        guard_far_delta,
        guard_frr_delta,
        kernel_only_recall_delta,
    ))
}

fn slot_contract_passed(
    measurement: &CompressionSlotMeasurement,
    bits_delta: f64,
    guard_far_delta: f64,
    guard_frr_delta: f64,
    kernel_only_recall_delta: f64,
) -> bool {
    cosine_error_within_bound(measurement)
        && bits_delta_within_bound(measurement, bits_delta)
        && guard_far_delta_within_bound(measurement, guard_far_delta)
        && guard_frr_delta_within_bound(measurement, guard_frr_delta)
        && kernel_recall_delta_within_bound(measurement, kernel_only_recall_delta)
}

fn cosine_error_within_bound(measurement: &CompressionSlotMeasurement) -> bool {
    measurement.achieved_cosine_error <= measurement.max_cosine_error
}

fn bits_delta_within_bound(measurement: &CompressionSlotMeasurement, bits_delta: f64) -> bool {
    bits_delta >= measurement.min_bits_delta
}

fn guard_far_delta_within_bound(
    measurement: &CompressionSlotMeasurement,
    guard_far_delta: f64,
) -> bool {
    guard_far_delta <= measurement.max_guard_far_delta
}

fn guard_frr_delta_within_bound(
    measurement: &CompressionSlotMeasurement,
    guard_frr_delta: f64,
) -> bool {
    guard_frr_delta <= measurement.max_guard_frr_delta
}

fn kernel_recall_delta_within_bound(
    measurement: &CompressionSlotMeasurement,
    kernel_only_recall_delta: f64,
) -> bool {
    kernel_only_recall_delta >= measurement.min_kernel_recall_delta
}
