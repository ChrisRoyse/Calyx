use calyx_forge::{
    CompressionReportInput, CompressionSlotMeasurement, KernelCompressionMeasurement, QuantLevel,
    compression_report,
};

fn fixture() -> CompressionReportInput {
    CompressionReportInput {
        vault_id: "vault-doc23-fixture".to_string(),
        slots: vec![
            CompressionSlotMeasurement {
                slot_id: "slot-text".to_string(),
                level: QuantLevel::Bits3p5,
                channel_count: 128,
                original_bytes: 512,
                compressed_bytes: 56,
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
                compressed_bytes: 40,
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
    let report = compression_report(fixture()).expect("report");

    assert_eq!(report.schema_version, 1);
    assert_eq!(report.slots.len(), 2);
    assert_eq!(report.totals.slot_count, 2);
    assert_eq!(report.totals.channel_count, 256);
    assert_close(report.totals.weighted_bits_per_channel, 3.0);
    assert_eq!(report.totals.original_bytes, 5120);
    assert_eq!(report.totals.compressed_bytes, 1632);
    assert_eq!(report.totals.bytes_saved, 3488);
    assert_close(report.totals.storage_compression_ratio, 5120.0 / 1632.0);

    let text = &report.slots[0];
    assert_eq!(text.bits_per_channel, 3.5);
    assert_close(text.distortion_vs_floor, 2.0);
    assert_close(text.distortion_margin_over_floor, 0.0015);
    assert_eq!(text.bytes_saved, 456);
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

    let expected_yield = (3488.0 / 5120.0) * ((0.440 + 0.552) / (0.420 + 0.550));
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

fn assert_close(actual: f64, expected: f64) {
    let delta = (actual - expected).abs();
    assert!(
        delta < 1e-9,
        "actual={actual:.12} expected={expected:.12} delta={delta:.12}"
    );
}
