use serde::Serialize;

use crate::assay_anchor_audit::AnchorAudit;

use super::super::format::VectorFormat;
use super::super::rows::RowStats;

pub(crate) const TEMPORAL_COUNTS_TOWARD_A35: bool = false;
pub(crate) const TEMPORAL_LANE_ROLE: &str = "time_manipulation_walk_forward_backward_as_of_sidecar";

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Evidence {
    pub(crate) out_dir: String,
    pub(crate) rows_jsonl: String,
    pub(crate) plan_path: String,
    pub(crate) timeline_path: String,
    pub(crate) progress_path: String,
    pub(crate) export_report_path: String,
    pub(crate) vector_dir: String,
    pub(crate) fbin_dir: Option<String>,
    pub(crate) vault_root: String,
    pub(crate) dataset: String,
    pub(crate) vector_format: VectorFormat,
    pub(crate) vector_storage_contract: &'static str,
    pub(crate) rows: RowStats,
    pub(crate) query_count: usize,
    pub(crate) batch_size: usize,
    pub(crate) min_bits: f32,
    pub(crate) pre_encode_gate: PreEncodeGateEvidence,
    pub(crate) streaming: bool,
    pub(crate) temporal_counts_toward_a35: bool,
    pub(crate) temporal_lane_role: &'static str,
    pub(crate) lens_roster: Vec<LensEvidence>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PreEncodeGateEvidence {
    pub(crate) mode: &'static str,
    pub(crate) diagnostic_only: bool,
    pub(crate) bits_report: String,
    pub(crate) anchor_entropy_bits: f32,
    pub(crate) sufficiency_basis_bits: f32,
    pub(crate) deficit_bits: f32,
    pub(crate) estimate_bound: String,
    pub(crate) power_calibration_status: String,
    pub(crate) power_recovery_ratio: f32,
    pub(crate) min_power_recovery_ratio: f32,
    pub(crate) sufficient: bool,
    pub(crate) grounded_gate_eligible: bool,
    pub(crate) anchor_audit: AnchorAudit,
    pub(crate) admitted_lenses: Vec<String>,
    pub(crate) streamed_lenses: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LensEvidence {
    pub(crate) slot: u16,
    pub(crate) name: String,
    pub(crate) lens_id: String,
    pub(crate) weights_sha256: String,
    pub(crate) signal_kind: String,
    pub(crate) bits_about: f32,
    pub(crate) dim: usize,
    pub(crate) native_dim: usize,
    pub(crate) assay_projection: String,
    pub(crate) max_batch: Option<usize>,
    pub(crate) effective_batch_size: usize,
    pub(crate) elapsed_ms: u64,
    pub(crate) ms_per_input: f64,
    pub(crate) manifest: String,
    pub(crate) corpus_path: String,
    pub(crate) queries_path: String,
    pub(crate) vault_path: String,
    pub(crate) corpus_rows_written: usize,
    pub(crate) query_rows_written: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) worker_pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) worker_report_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) worker_stderr_path: Option<String>,
}
