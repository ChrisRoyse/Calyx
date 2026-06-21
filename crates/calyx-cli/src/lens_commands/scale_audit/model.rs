use std::path::PathBuf;

use calyx_core::{Modality, Placement};
use serde::{Deserialize, Serialize};

pub(super) const SCHEMA: &str = "calyx-lens-scale-audit-v1";
pub(super) const DEFAULT_MIN_CONTENT_LENSES: usize = 10;
pub(super) const DEFAULT_MIN_GPU_CONTENT_LENSES: usize = 10;
pub(super) const DEFAULT_BATCH_SIZE: usize = 64;
pub(super) const DEFAULT_MIN_EFFECTIVE_BATCH: usize = 8;
pub(super) const DEFAULT_MIN_BATCH_COSINE: f32 = 0.999;
pub(super) const DEFAULT_MAX_ABS_DELTA: f32 = 0.02;
pub(super) const DEFAULT_LENS_TIMEOUT_SECS: u64 = 180;
pub(super) const TEMPORAL_LANE_ROLE: &str = "time_manipulation_walk_forward_backward_as_of_sidecar";

#[derive(Clone, Debug)]
pub(super) struct Flags {
    pub(super) manifests: Vec<PathBuf>,
    pub(super) out: PathBuf,
    pub(super) batch_size: usize,
    pub(super) min_content_lenses: usize,
    pub(super) min_gpu_content_lenses: usize,
    pub(super) min_effective_batch: usize,
    pub(super) min_batch_cosine: f32,
    pub(super) max_abs_delta: f32,
    pub(super) lens_timeout_secs: u64,
    pub(super) probes: Vec<String>,
    pub(super) probe_files: Vec<PathBuf>,
    pub(super) worker: bool,
}

#[derive(Serialize)]
pub(super) struct ScaleAuditReport {
    pub(super) schema: &'static str,
    pub(super) accepted: bool,
    pub(super) out: PathBuf,
    pub(super) requested_batch_size: usize,
    pub(super) min_content_lenses: usize,
    pub(super) min_gpu_content_lenses: usize,
    pub(super) min_effective_batch: usize,
    pub(super) min_batch_cosine: f32,
    pub(super) max_abs_delta: f32,
    pub(super) lens_timeout_secs: u64,
    pub(super) content_lens_count: usize,
    pub(super) gpu_content_lens_count: usize,
    pub(super) temporal_sidecar_count: usize,
    pub(super) temporal_counts_toward_content_floor: bool,
    pub(super) temporal_lane_role: &'static str,
    pub(super) rejected_count: usize,
    pub(super) rejections: Vec<Rejection>,
    pub(super) lenses: Vec<LensAudit>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct LensAudit {
    pub(super) manifest: PathBuf,
    pub(super) lens_id: String,
    pub(super) name: String,
    pub(super) modality: Modality,
    pub(super) runtime: String,
    pub(super) runtime_detail: String,
    pub(super) provider: String,
    pub(super) placement: Placement,
    pub(super) association_family: String,
    pub(super) temporal_sidecar: bool,
    pub(super) counts_toward_content_floor: bool,
    pub(super) weights_sha256: String,
    pub(super) dim: u32,
    pub(super) max_batch: Option<usize>,
    pub(super) requested_batch_size: usize,
    pub(super) effective_batch_size: usize,
    pub(super) native_batching: bool,
    pub(super) provider_placement_proof: String,
    pub(super) gpu_process_observed: Option<bool>,
    pub(super) probe_evidence: Vec<ProbeEvidence>,
    pub(super) rows_per_sec: Option<f64>,
    pub(super) batch_stability: Option<BatchStability>,
    pub(super) accepted: bool,
    pub(super) rejections: Vec<Rejection>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ProbeEvidence {
    pub(super) source: String,
    pub(super) path: Option<PathBuf>,
    pub(super) sha256: String,
    pub(super) bytes: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct BatchStability {
    pub(super) sample_rows: usize,
    pub(super) min_cosine: f32,
    pub(super) max_abs_delta: f32,
    pub(super) min_batch_cosine: f32,
    pub(super) max_allowed_abs_delta: f32,
    pub(super) acceptable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Rejection {
    pub(super) code: String,
    pub(super) message: String,
}

pub(super) fn reject(code: &'static str, message: impl Into<String>) -> Rejection {
    Rejection {
        code: code.to_string(),
        message: message.into(),
    }
}
