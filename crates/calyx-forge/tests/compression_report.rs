use calyx_forge::{
    CompressionReportInput, CompressionSlotMeasurement, KernelCompressionMeasurement, QuantLevel,
    QuantizedVec, Quantizer, TurboQuantCodec, compression_report, new_seed,
};

fn fixture() -> CompressionReportInput {
    let text_quantized = quantized_fixture(QuantLevel::Bits3p5, 128, b"slot-text");
    let image_quantized = quantized_fixture(QuantLevel::Bits2p5, 128, b"slot-image");
    CompressionReportInput {
        vault_id: "vault-doc23-fixture".to_string(),
        slots: vec![
            CompressionSlotMeasurement {
                slot_id: "slot-text".to_string(),
                level: QuantLevel::Bits3p5,
                channel_count: 128,
                original_bytes: 512,
                compressed_bytes: text_quantized.bytes.len() as u64,
                quantized: text_quantized,
                turboquant_floor_cosine_error: 0.0015,
                achieved_cosine_error: 0.0030,
                max_cosine_error: 0.0060,
                bits_about_before: 0.420,
                bits_about_after: 0.440,
                min_bits_delta: -0.010,
                guard_far_before: 0.0100,
                guard_far_after: 0.0110,
                max_guard_far_delta: 0.0020,
                guard_frr_before: 0.0200,
                guard_frr_after: 0.0205,
                max_guard_frr_delta: 0.0010,
                kernel_only_recall_before: 0.970,
                kernel_only_recall_after: 0.971,
                min_kernel_recall_delta: 0.0,
            },
            CompressionSlotMeasurement {
                slot_id: "slot-image".to_string(),
                level: QuantLevel::Bits2p5,
                channel_count: 128,
                original_bytes: 512,
                compressed_bytes: image_quantized.bytes.len() as u64,
                quantized: image_quantized,
                turboquant_floor_cosine_error: 0.0020,
                achieved_cosine_error: 0.0045,
                max_cosine_error: 0.0080,
                bits_about_before: 0.550,
                bits_about_after: 0.552,
                min_bits_delta: -0.005,
                guard_far_before: 0.0120,
                guard_far_after: 0.0124,
                max_guard_far_delta: 0.0010,
                guard_frr_before: 0.0150,
                guard_frr_after: 0.0152,
                max_guard_frr_delta: 0.0010,
                kernel_only_recall_before: 0.965,
                kernel_only_recall_after: 0.966,
                min_kernel_recall_delta: -0.001,
            },
        ],
        kernel: KernelCompressionMeasurement {
            original_bytes: 4096,
            compressed_bytes: 1536,
            recall_before: 0.981,
            recall_after: 0.982,
            min_recall_delta: -0.001,
        },
    }
}

#[test]
fn compression_report_aggregates_doc23_fields() {
    let input = fixture();
    let expected_compressed = input.kernel.compressed_bytes
        + input
            .slots
            .iter()
            .map(|slot| slot.quantized.bytes.len() as u64)
            .sum::<u64>();
    let expected_bytes_saved = 5120 - expected_compressed;
    let report = compression_report(input).expect("report");

    assert_eq!(report.schema_version, 1);
    assert_eq!(report.slots.len(), 2);
    assert_eq!(report.totals.slot_count, 2);
    assert_eq!(report.totals.channel_count, 256);
    assert_close(
        report.totals.weighted_bits_per_channel,
        (8.0 + f64::from(5.0_f32.log2() + 1.0)) / 2.0,
    );
    assert_eq!(report.totals.original_bytes, 5120);
    assert_eq!(report.totals.compressed_bytes, expected_compressed);
    assert_eq!(report.totals.bytes_saved, expected_bytes_saved);
    assert_close(
        report.totals.storage_compression_ratio,
        5120.0 / expected_compressed as f64,
    );

    let text = &report.slots[0];
    assert_eq!(text.bits_per_channel, 8.0);
    assert_eq!(text.stored_dim, 128);
    assert_eq!(text.stored_payload_sha256.len(), 64);
    assert_close(text.distortion_vs_floor, 2.0);
    assert_close(text.distortion_margin_over_floor, 0.0015);
    assert_eq!(text.bytes_saved, 512 - text.compressed_bytes);
    assert_close(text.bits_delta, 0.020);
    assert_close(text.guard_far_delta, 0.0010);
    assert!(text.passed_contract);

    assert_eq!(report.kernel.bytes_saved, 2560);
    assert_close(report.kernel.compression_ratio, 4096.0 / 1536.0);
    assert!(report.kernel.recall_unregressed);
    assert_close(report.intelligence_delta.min_bits_delta, 0.002);
    assert_close(report.intelligence_delta.max_cosine_error, 0.0045);
    assert_close(report.intelligence_delta.max_guard_far_delta, 0.0010);
    assert_close(
        report.intelligence_delta.min_kernel_only_recall_delta,
        0.001,
    );

    let expected_yield =
        (expected_bytes_saved as f64 / 5120.0) * ((0.440 + 0.552) / (0.420 + 0.550));
    assert_close(report.meaning_compression_yield, expected_yield);
}

#[test]
fn compression_report_rejects_empty_slots() {
    let mut input = fixture();
    input.slots.clear();

    let err = compression_report(input).expect_err("empty slots fail closed");
    assert!(err.to_string().starts_with("CALYX_FORGE_QUANT_ERROR"));
}

#[test]
fn compression_report_rejects_cosine_intelligence_loss() {
    let mut input = fixture();
    input.slots[0].achieved_cosine_error = 0.020;

    let err = compression_report(input).expect_err("cosine loss fail closed");
    assert!(err.to_string().starts_with("CALYX_QUANT_INTELLIGENCE_LOSS"));
    assert!(err.to_string().contains("cosine error"));
}

#[test]
fn compression_report_rejects_guard_far_regression() {
    let mut input = fixture();
    input.slots[1].guard_far_after = 0.050;

    let err = compression_report(input).expect_err("FAR regression fail closed");
    assert!(err.to_string().starts_with("CALYX_QUANT_INTELLIGENCE_LOSS"));
    assert!(err.to_string().contains("guard FAR delta"));
}

#[test]
fn compression_report_rejects_kernel_recall_regression() {
    let mut input = fixture();
    input.kernel.recall_after = 0.970;

    let err = compression_report(input).expect_err("kernel recall fail closed");
    assert!(err.to_string().starts_with("CALYX_QUANT_INTELLIGENCE_LOSS"));
    assert!(err.to_string().contains("kernel recall delta"));
}

#[test]
fn compression_report_rejects_declared_byte_count_mismatch() {
    let mut input = fixture();
    input.slots[0].compressed_bytes -= 1;

    let err = compression_report(input).expect_err("declared bytes must match persisted bytes");
    assert!(err.to_string().starts_with("CALYX_FORGE_QUANT_ERROR"));
    assert!(err.to_string().contains("declared compressed_bytes"));
}

#[test]
fn compression_report_rejects_declared_bits8_with_raw_f32_bytes() {
    let mut input = fixture();
    let bytes = raw_f32_payload(&[0.25, -0.5, 0.75, 1.0]);
    input.slots[0].level = QuantLevel::Bits8;
    input.slots[0].channel_count = 4;
    input.slots[0].original_bytes = 16;
    input.slots[0].compressed_bytes = bytes.len() as u64;
    input.slots[0].quantized = QuantizedVec {
        level: QuantLevel::Bits8,
        dim: 4,
        bytes,
        scale: 1.0,
        seed_id: [0; 32],
    };

    let err = compression_report(input).expect_err("RawF32 bytes must not pass as Bits8");
    assert!(err.to_string().starts_with("CALYX_FORGE_QUANT_ERROR"));
    assert!(
        err.to_string()
            .contains("persisted payload length mismatch")
    );
}

#[test]
fn compression_report_rejects_truncated_payload_even_when_metadata_matches() {
    let mut input = fixture();
    input.slots[0].level = QuantLevel::Bits8;
    input.slots[0].channel_count = 4;
    input.slots[0].original_bytes = 16;
    input.slots[0].compressed_bytes = 3;
    input.slots[0].quantized = QuantizedVec {
        level: QuantLevel::Bits8,
        dim: 4,
        bytes: vec![1, 2, 3],
        scale: 1.0,
        seed_id: [0; 32],
    };

    let err = compression_report(input).expect_err("truncated Bits8 payload must fail");
    assert!(err.to_string().starts_with("CALYX_FORGE_QUANT_ERROR"));
    assert!(err.to_string().contains("expected 4 got 3"));
}

#[test]
fn compression_report_rejects_mismatched_persisted_dim() {
    let mut input = fixture();
    input.slots[0].quantized.dim += 1;

    let err = compression_report(input).expect_err("dim mismatch must fail");
    assert!(err.to_string().starts_with("CALYX_FORGE_QUANT_ERROR"));
    assert!(err.to_string().contains("declared channel_count"));
}

#[test]
fn compression_report_rejects_non_finite_raw_f32_payload() {
    let mut input = fixture();
    let bytes = raw_f32_payload(&[0.25, f32::NAN, 0.75, 1.0]);
    input.slots[0].level = QuantLevel::F32;
    input.slots[0].channel_count = 4;
    input.slots[0].original_bytes = 16;
    input.slots[0].compressed_bytes = bytes.len() as u64;
    input.slots[0].quantized = QuantizedVec {
        level: QuantLevel::F32,
        dim: 4,
        bytes,
        scale: 1.0,
        seed_id: [0; 32],
    };

    let err = compression_report(input).expect_err("NaN RawF32 payload must fail");
    assert!(err.to_string().starts_with("CALYX_FORGE_QUANT_ERROR"));
    assert!(err.to_string().contains("non-finite coefficient"));
}

fn assert_close(actual: f64, expected: f64) {
    let delta = (actual - expected).abs();
    assert!(
        delta < 1e-9,
        "actual={actual:.12} expected={expected:.12} delta={delta:.12}"
    );
}

fn quantized_fixture(level: QuantLevel, dim: usize, seed_tag: &[u8]) -> QuantizedVec {
    let codec = TurboQuantCodec::new(new_seed(dim, seed_tag), level).expect("turbo codec");
    codec
        .encode(&unit_vector(dim, seed_tag[0] as f32 / 17.0))
        .expect("encode fixture")
}

fn unit_vector(dim: usize, phase: f32) -> Vec<f32> {
    let mut vector: Vec<f32> = (0..dim)
        .map(|idx| {
            let x = idx as f32 + 1.0;
            (x * phase).sin() + (x * 0.137).cos() * 0.25
        })
        .collect();
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    for value in &mut vector {
        *value /= norm;
    }
    vector
}

fn raw_f32_payload(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(values));
    for value in values {
        bytes.extend_from_slice(&value.to_bits().to_be_bytes());
    }
    bytes
}
