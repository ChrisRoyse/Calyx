use crate::{QuantLevel, Result};

use super::types::{
    COMPRESSION_REPORT_SCHEMA_VERSION, CompressionReport, CompressionReportInput,
    CompressionSlotMeasurement, CompressionSlotReport, CompressionTotals, IntelligenceDeltaReport,
    KernelCompressionMeasurement, KernelCompressionReport,
};
use super::validate::{
    checked_add, intelligence_loss, quant_error, ratio, reject_if, require_bytes,
    require_finite_f64, require_nonnegative_f64, require_positive_f64, require_positive_u64,
    require_range_f64, require_unit_interval, validate_slot_id, validate_vault,
};

pub fn compression_report(input: CompressionReportInput) -> Result<CompressionReport> {
    validate_vault(&input.vault_id)?;
    if input.slots.is_empty() {
        return Err(quant_error("report", "no quantized slots supplied"));
    }

    let mut slots = Vec::with_capacity(input.slots.len());
    for measurement in &input.slots {
        slots.push(slot_report(measurement)?);
    }

    let kernel = kernel_report(&input.kernel)?;
    let totals = totals_report(&slots, &kernel)?;
    let intelligence_delta = intelligence_delta(&slots);
    let meaning_compression_yield = meaning_yield(&slots, &totals)?;

    Ok(CompressionReport {
        schema_version: COMPRESSION_REPORT_SCHEMA_VERSION,
        vault_id: input.vault_id,
        slots,
        totals,
        kernel,
        intelligence_delta,
        meaning_compression_yield,
    })
}

fn slot_report(measurement: &CompressionSlotMeasurement) -> Result<CompressionSlotReport> {
    validate_slot_id(&measurement.slot_id, measurement.level)?;
    require_positive_u64(
        measurement.channel_count,
        "channel_count",
        measurement.level,
    )?;
    require_bytes(
        measurement.original_bytes,
        measurement.compressed_bytes,
        measurement.level,
    )?;
    validate_distortion(measurement)?;
    validate_intelligence_inputs(measurement)?;

    let bits_delta = measurement.bits_about_after - measurement.bits_about_before;
    let guard_far_delta = measurement.guard_far_after - measurement.guard_far_before;
    let guard_frr_delta = measurement.guard_frr_after - measurement.guard_frr_before;
    let kernel_only_recall_delta =
        measurement.kernel_only_recall_after - measurement.kernel_only_recall_before;
    validate_slot_contract(
        measurement,
        bits_delta,
        guard_far_delta,
        guard_frr_delta,
        kernel_only_recall_delta,
    )?;

    let bytes_saved = measurement.original_bytes - measurement.compressed_bytes;
    Ok(CompressionSlotReport {
        slot_id: measurement.slot_id.clone(),
        level: measurement.level,
        channel_count: measurement.channel_count,
        bits_per_channel: f64::from(measurement.level.bits_per_channel()),
        turboquant_floor_cosine_error: measurement.turboquant_floor_cosine_error,
        achieved_cosine_error: measurement.achieved_cosine_error,
        distortion_vs_floor: ratio(
            measurement.achieved_cosine_error,
            measurement.turboquant_floor_cosine_error,
        ),
        distortion_margin_over_floor: measurement.achieved_cosine_error
            - measurement.turboquant_floor_cosine_error,
        original_bytes: measurement.original_bytes,
        compressed_bytes: measurement.compressed_bytes,
        bytes_saved,
        storage_compression_ratio: ratio(
            measurement.original_bytes as f64,
            measurement.compressed_bytes as f64,
        ),
        bits_about_before: measurement.bits_about_before,
        bits_about_after: measurement.bits_about_after,
        bits_delta,
        guard_far_before: measurement.guard_far_before,
        guard_far_after: measurement.guard_far_after,
        guard_far_delta,
        guard_frr_before: measurement.guard_frr_before,
        guard_frr_after: measurement.guard_frr_after,
        guard_frr_delta,
        kernel_only_recall_before: measurement.kernel_only_recall_before,
        kernel_only_recall_after: measurement.kernel_only_recall_after,
        kernel_only_recall_delta,
        passed_contract: true,
    })
}

fn validate_distortion(measurement: &CompressionSlotMeasurement) -> Result<()> {
    require_positive_f64(
        measurement.turboquant_floor_cosine_error,
        "turboquant_floor_cosine_error",
        measurement.level,
    )?;
    require_range_f64(
        measurement.achieved_cosine_error,
        "achieved_cosine_error",
        0.0,
        2.0,
        measurement.level,
    )?;
    require_range_f64(
        measurement.max_cosine_error,
        "max_cosine_error",
        0.0,
        2.0,
        measurement.level,
    )
}

fn validate_intelligence_inputs(measurement: &CompressionSlotMeasurement) -> Result<()> {
    require_positive_f64(
        measurement.bits_about_before,
        "bits_about_before",
        measurement.level,
    )?;
    require_nonnegative_f64(
        measurement.bits_about_after,
        "bits_about_after",
        measurement.level,
    )?;
    require_finite_f64(
        measurement.min_bits_delta,
        "min_bits_delta",
        measurement.level,
    )?;
    require_unit_interval(
        measurement.guard_far_before,
        "guard_far_before",
        measurement.level,
    )?;
    require_unit_interval(
        measurement.guard_far_after,
        "guard_far_after",
        measurement.level,
    )?;
    require_nonnegative_f64(
        measurement.max_guard_far_delta,
        "max_guard_far_delta",
        measurement.level,
    )?;
    require_unit_interval(
        measurement.guard_frr_before,
        "guard_frr_before",
        measurement.level,
    )?;
    require_unit_interval(
        measurement.guard_frr_after,
        "guard_frr_after",
        measurement.level,
    )?;
    require_nonnegative_f64(
        measurement.max_guard_frr_delta,
        "max_guard_frr_delta",
        measurement.level,
    )?;
    require_unit_interval(
        measurement.kernel_only_recall_before,
        "kernel_only_recall_before",
        measurement.level,
    )?;
    require_unit_interval(
        measurement.kernel_only_recall_after,
        "kernel_only_recall_after",
        measurement.level,
    )?;
    require_finite_f64(
        measurement.min_kernel_recall_delta,
        "min_kernel_recall_delta",
        measurement.level,
    )
}

fn validate_slot_contract(
    measurement: &CompressionSlotMeasurement,
    bits_delta: f64,
    guard_far_delta: f64,
    guard_frr_delta: f64,
    kernel_only_recall_delta: f64,
) -> Result<()> {
    reject_if(
        measurement.achieved_cosine_error > measurement.max_cosine_error,
        &measurement.slot_id,
        format!(
            "cosine error {:.8} exceeds bound {:.8}",
            measurement.achieved_cosine_error, measurement.max_cosine_error
        ),
    )?;
    reject_if(
        bits_delta < measurement.min_bits_delta,
        &measurement.slot_id,
        format!(
            "bits delta {:.8} below bound {:.8}",
            bits_delta, measurement.min_bits_delta
        ),
    )?;
    reject_if(
        guard_far_delta > measurement.max_guard_far_delta,
        &measurement.slot_id,
        format!(
            "guard FAR delta {:.8} exceeds bound {:.8}",
            guard_far_delta, measurement.max_guard_far_delta
        ),
    )?;
    reject_if(
        guard_frr_delta > measurement.max_guard_frr_delta,
        &measurement.slot_id,
        format!(
            "guard FRR delta {:.8} exceeds bound {:.8}",
            guard_frr_delta, measurement.max_guard_frr_delta
        ),
    )?;
    reject_if(
        kernel_only_recall_delta < measurement.min_kernel_recall_delta,
        &measurement.slot_id,
        format!(
            "kernel-only recall delta {:.8} below bound {:.8}",
            kernel_only_recall_delta, measurement.min_kernel_recall_delta
        ),
    )
}

fn kernel_report(measurement: &KernelCompressionMeasurement) -> Result<KernelCompressionReport> {
    require_bytes(
        measurement.original_bytes,
        measurement.compressed_bytes,
        QuantLevel::F32,
    )?;
    require_unit_interval(
        measurement.recall_before,
        "kernel.recall_before",
        QuantLevel::F32,
    )?;
    require_unit_interval(
        measurement.recall_after,
        "kernel.recall_after",
        QuantLevel::F32,
    )?;
    require_finite_f64(
        measurement.min_recall_delta,
        "kernel.min_recall_delta",
        QuantLevel::F32,
    )?;

    let recall_delta = measurement.recall_after - measurement.recall_before;
    if recall_delta < measurement.min_recall_delta {
        return Err(intelligence_loss(
            "kernel",
            format!(
                "kernel recall delta {:.8} below bound {:.8}",
                recall_delta, measurement.min_recall_delta
            ),
        ));
    }

    Ok(KernelCompressionReport {
        original_bytes: measurement.original_bytes,
        compressed_bytes: measurement.compressed_bytes,
        bytes_saved: measurement.original_bytes - measurement.compressed_bytes,
        compression_ratio: ratio(
            measurement.original_bytes as f64,
            measurement.compressed_bytes as f64,
        ),
        recall_before: measurement.recall_before,
        recall_after: measurement.recall_after,
        recall_delta,
        recall_unregressed: true,
    })
}

fn totals_report(
    slots: &[CompressionSlotReport],
    kernel: &KernelCompressionReport,
) -> Result<CompressionTotals> {
    let mut channel_count = 0_u64;
    let mut weighted_bits = 0.0_f64;
    let mut original_bytes = kernel.original_bytes;
    let mut compressed_bytes = kernel.compressed_bytes;

    for slot in slots {
        channel_count = checked_add(channel_count, slot.channel_count, "channel_count")?;
        weighted_bits += slot.bits_per_channel * slot.channel_count as f64;
        original_bytes = checked_add(original_bytes, slot.original_bytes, "original_bytes")?;
        compressed_bytes =
            checked_add(compressed_bytes, slot.compressed_bytes, "compressed_bytes")?;
    }

    let bytes_saved = original_bytes - compressed_bytes;
    Ok(CompressionTotals {
        slot_count: slots.len() as u64,
        channel_count,
        weighted_bits_per_channel: ratio(weighted_bits, channel_count as f64),
        original_bytes,
        compressed_bytes,
        bytes_saved,
        storage_compression_ratio: ratio(original_bytes as f64, compressed_bytes as f64),
    })
}

fn intelligence_delta(slots: &[CompressionSlotReport]) -> IntelligenceDeltaReport {
    IntelligenceDeltaReport {
        min_bits_delta: slots
            .iter()
            .map(|slot| slot.bits_delta)
            .fold(f64::INFINITY, f64::min),
        max_cosine_error: slots
            .iter()
            .map(|slot| slot.achieved_cosine_error)
            .fold(0.0, f64::max),
        max_guard_far_delta: slots
            .iter()
            .map(|slot| slot.guard_far_delta)
            .fold(f64::NEG_INFINITY, f64::max),
        max_guard_frr_delta: slots
            .iter()
            .map(|slot| slot.guard_frr_delta)
            .fold(f64::NEG_INFINITY, f64::max),
        min_kernel_only_recall_delta: slots
            .iter()
            .map(|slot| slot.kernel_only_recall_delta)
            .fold(f64::INFINITY, f64::min),
    }
}

fn meaning_yield(slots: &[CompressionSlotReport], totals: &CompressionTotals) -> Result<f64> {
    let bits_before: f64 = slots.iter().map(|slot| slot.bits_about_before).sum();
    let bits_after: f64 = slots.iter().map(|slot| slot.bits_about_after).sum();
    let retained_bits_ratio = ratio(bits_after, bits_before);
    let saved_ratio = ratio(totals.bytes_saved as f64, totals.original_bytes as f64);
    Ok(saved_ratio * retained_bits_ratio)
}
