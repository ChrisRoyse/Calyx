use sha2::{Digest, Sha256};

use crate::mxfp4::MXFP4_PACKED_BYTES;
use crate::quant::turboquant;
use crate::{
    MXFP4_BLOCK_SIZE, MXFP8_BLOCK_BYTES, MXFP8_BLOCK_SIZE, QuantLevel, QuantizedVec, Result,
};

use super::contract::validate_slot_contract;
use super::types::{
    COMPRESSION_REPORT_SCHEMA_VERSION, CompressionReport, CompressionReportInput,
    CompressionSlotMeasurement, CompressionSlotReport, CompressionTotals, IntelligenceDeltaReport,
    KernelCompressionMeasurement, KernelCompressionReport,
};
use super::validate::{
    checked_add, intelligence_loss, quant_error, ratio, require_bytes, require_finite_f64,
    require_nonnegative_f64, require_positive_f64, require_positive_u64, require_range_f64,
    require_unit_interval, validate_slot_id, validate_vault,
};

const MXFP4_BLOCK_BYTES: usize = MXFP4_PACKED_BYTES + 1;

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
    let persisted = validate_persisted_quantized(measurement)?;
    validate_slot_id(&measurement.slot_id, persisted.level)?;
    require_positive_u64(persisted.dim, "channel_count", persisted.level)?;
    require_bytes(measurement.original_bytes, persisted.bytes, persisted.level)?;
    validate_distortion(measurement)?;
    validate_intelligence_inputs(measurement)?;

    let bits_delta = measurement.bits_about_after - measurement.bits_about_before;
    let guard_far_delta = measurement.guard_far_after - measurement.guard_far_before;
    let guard_frr_delta = measurement.guard_frr_after - measurement.guard_frr_before;
    let kernel_only_recall_delta =
        measurement.kernel_only_recall_after - measurement.kernel_only_recall_before;
    let passed_contract = validate_slot_contract(
        measurement,
        bits_delta,
        guard_far_delta,
        guard_frr_delta,
        kernel_only_recall_delta,
    )?;

    let bytes_saved = measurement.original_bytes - persisted.bytes;
    Ok(CompressionSlotReport {
        slot_id: measurement.slot_id.clone(),
        level: persisted.level,
        channel_count: persisted.dim,
        stored_dim: persisted.dim,
        bits_per_channel: f64::from(persisted.level.bits_per_channel()),
        stored_payload_sha256: persisted.payload_sha256,
        turboquant_floor_cosine_error: measurement.turboquant_floor_cosine_error,
        achieved_cosine_error: measurement.achieved_cosine_error,
        distortion_vs_floor: ratio(
            measurement.achieved_cosine_error,
            measurement.turboquant_floor_cosine_error,
        ),
        distortion_margin_over_floor: measurement.achieved_cosine_error
            - measurement.turboquant_floor_cosine_error,
        original_bytes: measurement.original_bytes,
        compressed_bytes: persisted.bytes,
        bytes_saved,
        storage_compression_ratio: ratio(measurement.original_bytes as f64, persisted.bytes as f64),
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
        passed_contract,
    })
}

struct PersistedSlot {
    level: QuantLevel,
    dim: u64,
    bytes: u64,
    payload_sha256: String,
}

fn validate_persisted_quantized(measurement: &CompressionSlotMeasurement) -> Result<PersistedSlot> {
    let qv = &measurement.quantized;
    if qv.level != measurement.level {
        return Err(quant_error(
            measurement.level,
            format!(
                "declared level {:?} does not match persisted level {:?}",
                measurement.level, qv.level
            ),
        ));
    }
    if qv.dim as u64 != measurement.channel_count {
        return Err(quant_error(
            measurement.level,
            format!(
                "declared channel_count {} does not match persisted dim {}",
                measurement.channel_count, qv.dim
            ),
        ));
    }
    let payload_len = qv.bytes.len() as u64;
    if payload_len != measurement.compressed_bytes {
        return Err(quant_error(
            measurement.level,
            format!(
                "declared compressed_bytes {} does not match persisted byte length {}",
                measurement.compressed_bytes, payload_len
            ),
        ));
    }
    validate_quantized_payload(qv)?;
    Ok(PersistedSlot {
        level: qv.level,
        dim: qv.dim as u64,
        bytes: payload_len,
        payload_sha256: sha256_hex(&qv.bytes),
    })
}

fn validate_quantized_payload(qv: &QuantizedVec) -> Result<()> {
    if qv.dim == 0 {
        return Err(quant_error(
            qv.level,
            "persisted quantized dim must be positive",
        ));
    }
    if !qv.scale.is_finite() || qv.scale < 0.0 {
        return Err(quant_error(
            qv.level,
            "persisted quantized scale must be finite and non-negative",
        ));
    }
    match qv.level {
        QuantLevel::F32 => validate_raw_f32_payload(qv),
        QuantLevel::Bits8 => require_payload_len(qv, qv.dim),
        QuantLevel::Bits8Fp => {
            require_payload_len(qv, qv.dim.div_ceil(MXFP8_BLOCK_SIZE) * MXFP8_BLOCK_BYTES)
        }
        QuantLevel::Bits4Fp => {
            require_payload_len(qv, qv.dim.div_ceil(MXFP4_BLOCK_SIZE) * MXFP4_BLOCK_BYTES)
        }
        QuantLevel::Bits3p5 | QuantLevel::Bits2p5 => {
            let minimum = turboquant::packed_len(qv.dim, qv.level);
            if qv.bytes.len() < minimum {
                return Err(quant_error(
                    qv.level,
                    format!(
                        "persisted TurboQuant payload too short: expected at least {minimum} bytes got {}",
                        qv.bytes.len()
                    ),
                ));
            }
            Ok(())
        }
        QuantLevel::Bits1 => require_payload_len(qv, qv.dim.div_ceil(8)),
    }
}

fn validate_raw_f32_payload(qv: &QuantizedVec) -> Result<()> {
    require_payload_len(qv, qv.dim * std::mem::size_of::<f32>())?;
    for (idx, chunk) in qv
        .bytes
        .chunks_exact(std::mem::size_of::<f32>())
        .enumerate()
    {
        let value = f32::from_bits(u32::from_be_bytes(chunk.try_into().unwrap()));
        if !value.is_finite() {
            return Err(quant_error(
                qv.level,
                format!("persisted F32 payload contains non-finite coefficient at index {idx}"),
            ));
        }
    }
    Ok(())
}

fn require_payload_len(qv: &QuantizedVec, expected: usize) -> Result<()> {
    if qv.bytes.len() == expected {
        return Ok(());
    }
    Err(quant_error(
        qv.level,
        format!(
            "persisted payload length mismatch: expected {expected} got {}",
            qv.bytes.len()
        ),
    ))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
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
    let recall_unregressed = recall_delta >= measurement.min_recall_delta;
    if !recall_unregressed {
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
        recall_unregressed,
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
