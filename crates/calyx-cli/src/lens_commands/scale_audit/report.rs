use std::fs;
use std::path::{Path, PathBuf};

use super::model::{Flags, LensAudit, SCHEMA, ScaleAuditReport, TEMPORAL_LANE_ROLE, reject};
use crate::error::CliResult;
use calyx_core::Placement;

pub(super) struct ProgressUpdate {
    pub(super) event: &'static str,
    pub(super) phase: &'static str,
    pub(super) message: Option<String>,
    pub(super) worker_pid: Option<u32>,
    pub(super) worker_report_path: Option<PathBuf>,
    pub(super) worker_stderr_path: Option<PathBuf>,
}

impl ProgressUpdate {
    pub(super) fn new(event: &'static str, phase: &'static str) -> Self {
        Self {
            event,
            phase,
            message: None,
            worker_pid: None,
            worker_report_path: None,
            worker_stderr_path: None,
        }
    }

    pub(super) fn with_worker(
        mut self,
        message: String,
        worker_pid: Option<u32>,
        worker_report_path: PathBuf,
        worker_stderr_path: PathBuf,
    ) -> Self {
        self.message = Some(message);
        self.worker_pid = worker_pid;
        self.worker_report_path = Some(worker_report_path);
        self.worker_stderr_path = Some(worker_stderr_path);
        self
    }
}

#[derive(serde::Serialize)]
struct ProgressSnapshot<'a> {
    schema: &'static str,
    event: &'static str,
    current_index: usize,
    lens_total: usize,
    current_manifest: &'a PathBuf,
    phase: &'static str,
    message: Option<String>,
    worker_pid: Option<u32>,
    worker_report_path: Option<PathBuf>,
    worker_stderr_path: Option<PathBuf>,
    completed_lenses: usize,
    progress_path: PathBuf,
    report_path: PathBuf,
    lenses: &'a [LensAudit],
}

pub(super) fn build_report(lenses: Vec<LensAudit>, flags: &Flags) -> ScaleAuditReport {
    let content_lens_count = lenses
        .iter()
        .filter(|lens| lens.counts_toward_content_floor && lens.accepted)
        .count();
    let gpu_content_lens_count = lenses
        .iter()
        .filter(|lens| {
            lens.counts_toward_content_floor && lens.accepted && lens.placement == Placement::Gpu
        })
        .count();
    let temporal_sidecar_count = lenses.iter().filter(|lens| lens.temporal_sidecar).count();
    let mut rejections = lenses
        .iter()
        .flat_map(|lens| lens.rejections.iter().cloned())
        .collect::<Vec<_>>();
    if content_lens_count < flags.min_content_lenses {
        rejections.push(reject(
            "CALYX_LENS_SCALE_CONTENT_FLOOR",
            format!(
                "accepted content lenses {content_lens_count} below floor {}",
                flags.min_content_lenses
            ),
        ));
    }
    if gpu_content_lens_count < flags.min_gpu_content_lenses {
        rejections.push(reject(
            "CALYX_LENS_SCALE_GPU_CONTENT_FLOOR",
            format!(
                "accepted GPU content lenses {gpu_content_lens_count} below floor {}",
                flags.min_gpu_content_lenses
            ),
        ));
    }
    ScaleAuditReport {
        schema: SCHEMA,
        accepted: rejections.is_empty(),
        out: flags.out.clone(),
        requested_batch_size: flags.batch_size,
        min_content_lenses: flags.min_content_lenses,
        min_gpu_content_lenses: flags.min_gpu_content_lenses,
        min_effective_batch: flags.min_effective_batch,
        min_batch_cosine: flags.min_batch_cosine,
        max_abs_delta: flags.max_abs_delta,
        lens_timeout_secs: flags.lens_timeout_secs,
        content_lens_count,
        gpu_content_lens_count,
        temporal_sidecar_count,
        temporal_counts_toward_content_floor: false,
        temporal_lane_role: TEMPORAL_LANE_ROLE,
        rejected_count: rejections.len(),
        rejections,
        lenses,
    }
}

pub(super) fn write_progress(
    flags: &Flags,
    current_index: usize,
    current_manifest: &PathBuf,
    update: ProgressUpdate,
    lenses: &[LensAudit],
) -> CliResult {
    let path = progress_path(&flags.out);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let snapshot = ProgressSnapshot {
        schema: "calyx-lens-scale-audit-progress-v1",
        event: update.event,
        current_index,
        lens_total: flags.manifests.len(),
        current_manifest,
        phase: update.phase,
        message: update.message,
        worker_pid: update.worker_pid,
        worker_report_path: update.worker_report_path,
        worker_stderr_path: update.worker_stderr_path,
        completed_lenses: lenses.len(),
        progress_path: path.clone(),
        report_path: flags.out.clone(),
        lenses,
    };
    let bytes = serde_json::to_vec_pretty(&snapshot)?;
    fs::write(path, bytes)?;
    Ok(())
}

pub(super) fn write_report(path: &PathBuf, report: &ScaleAuditReport) -> CliResult {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(report)?;
    fs::write(path, bytes)?;
    Ok(())
}

pub(super) fn write_lens_audit(path: &PathBuf, audit: &LensAudit) -> CliResult {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(audit)?;
    fs::write(path, bytes)?;
    Ok(())
}

fn progress_path(report_path: &Path) -> PathBuf {
    report_path.with_file_name("scale_audit_progress.json")
}
