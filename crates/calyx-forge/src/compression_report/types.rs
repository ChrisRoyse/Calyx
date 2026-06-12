use serde::{Deserialize, Serialize};

use crate::QuantLevel;

pub const COMPRESSION_REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CompressionReportInput {
    pub vault_id: String,
    pub slots: Vec<CompressionSlotMeasurement>,
    pub kernel: KernelCompressionMeasurement,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CompressionSlotMeasurement {
    pub slot_id: String,
    pub level: QuantLevel,
    pub channel_count: u64,
    pub original_bytes: u64,
    pub compressed_bytes: u64,
    pub turboquant_floor_cosine_error: f64,
    pub achieved_cosine_error: f64,
    pub max_cosine_error: f64,
    pub bits_about_before: f64,
    pub bits_about_after: f64,
    pub min_bits_delta: f64,
    pub guard_far_before: f64,
    pub guard_far_after: f64,
    pub max_guard_far_delta: f64,
    pub guard_frr_before: f64,
    pub guard_frr_after: f64,
    pub max_guard_frr_delta: f64,
    pub kernel_only_recall_before: f64,
    pub kernel_only_recall_after: f64,
    pub min_kernel_recall_delta: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct KernelCompressionMeasurement {
    pub original_bytes: u64,
    pub compressed_bytes: u64,
    pub recall_before: f64,
    pub recall_after: f64,
    pub min_recall_delta: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CompressionReport {
    pub schema_version: u32,
    pub vault_id: String,
    pub slots: Vec<CompressionSlotReport>,
    pub totals: CompressionTotals,
    pub kernel: KernelCompressionReport,
    pub intelligence_delta: IntelligenceDeltaReport,
    pub meaning_compression_yield: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CompressionSlotReport {
    pub slot_id: String,
    pub level: QuantLevel,
    pub channel_count: u64,
    pub bits_per_channel: f64,
    pub turboquant_floor_cosine_error: f64,
    pub achieved_cosine_error: f64,
    pub distortion_vs_floor: f64,
    pub distortion_margin_over_floor: f64,
    pub original_bytes: u64,
    pub compressed_bytes: u64,
    pub bytes_saved: u64,
    pub storage_compression_ratio: f64,
    pub bits_about_before: f64,
    pub bits_about_after: f64,
    pub bits_delta: f64,
    pub guard_far_before: f64,
    pub guard_far_after: f64,
    pub guard_far_delta: f64,
    pub guard_frr_before: f64,
    pub guard_frr_after: f64,
    pub guard_frr_delta: f64,
    pub kernel_only_recall_before: f64,
    pub kernel_only_recall_after: f64,
    pub kernel_only_recall_delta: f64,
    pub passed_contract: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CompressionTotals {
    pub slot_count: u64,
    pub channel_count: u64,
    pub weighted_bits_per_channel: f64,
    pub original_bytes: u64,
    pub compressed_bytes: u64,
    pub bytes_saved: u64,
    pub storage_compression_ratio: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct KernelCompressionReport {
    pub original_bytes: u64,
    pub compressed_bytes: u64,
    pub bytes_saved: u64,
    pub compression_ratio: f64,
    pub recall_before: f64,
    pub recall_after: f64,
    pub recall_delta: f64,
    pub recall_unregressed: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct IntelligenceDeltaReport {
    pub min_bits_delta: f64,
    pub max_cosine_error: f64,
    pub max_guard_far_delta: f64,
    pub max_guard_frr_delta: f64,
    pub min_kernel_only_recall_delta: f64,
}
