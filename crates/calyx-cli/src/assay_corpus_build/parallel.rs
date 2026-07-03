//! K-way lens-worker scheduling for the assay harnesses (#1160, child of
//! #1152).
//!
//! The harnesses historically ran one lens worker at a time, so a CPU-bound
//! lens (SPLADE burned ~1h50m of single-core time in the #791 gate) held the
//! whole box while the GPU sat idle. `--lens-parallelism K` schedules up to K
//! single-lens worker processes concurrently. Defaults keep today's behavior:
//! K=1 is the evidence-isolation mode and produces the exact sequential
//! event order.
//!
//! Co-residency safety: K>1 loads K models onto the GPU at once, which is
//! only safe when every worker session has an explicit #1143 arena budget.
//! The scheduler therefore refuses K>1 unless `CALYX_ONNX_GPU_MEM_LIMIT_MIB`
//! is set (inherited by workers) or `--worker-gpu-mem-limit-mib` is passed —
//! never a silent default.
//!
//! Start order pairs CPU-heavy lenses with GPU lenses first so the wall-clock
//! win of overlapping them is realized from the start of the run; slot
//! numbering and per-slot outputs are untouched by start order.

use std::path::PathBuf;

use calyx_registry::{LensRuntime, lens_spec_metadata_from_manifest_path};
use serde_json::json;

pub(crate) const GPU_MEM_LIMIT_ENV: &str = "CALYX_ONNX_GPU_MEM_LIMIT_MIB";

/// Refuse unbudgeted K>1 co-residency (#1143 arena budgets make it safe).
pub(crate) fn ensure_worker_vram_budget(
    lens_parallelism: usize,
    worker_gpu_mem_limit_mib: Option<usize>,
) -> Result<(), String> {
    if lens_parallelism <= 1 || worker_gpu_mem_limit_mib.is_some() {
        return Ok(());
    }
    let env_set = std::env::var(GPU_MEM_LIMIT_ENV)
        .map(|raw| !raw.trim().is_empty())
        .unwrap_or(false);
    if env_set {
        return Ok(());
    }
    Err(format!(
        "lens-parallelism {lens_parallelism} runs {lens_parallelism} GPU sessions co-resident, but no per-worker CUDA arena budget is set; set {GPU_MEM_LIMIT_ENV} or pass --worker-gpu-mem-limit-mib so an over-committed worker fails at a defined budget instead of eating co-tenants"
    ))
}

/// Start order for K-way scheduling: alternate CPU-heavy and GPU lenses so
/// the CPU-bound work overlaps GPU compute from the first scheduling wave.
/// Classification is a scheduling hint only — a manifest whose metadata
/// cannot be read is logged and scheduled as GPU; its worker will fail loud
/// with full attribution if it is actually broken.
pub(crate) fn interleaved_start_order(manifests: &[PathBuf]) -> Vec<usize> {
    let mut cpu_heavy = Vec::new();
    let mut gpu = Vec::new();
    for (index, manifest) in manifests.iter().enumerate() {
        if is_cpu_heavy(manifest) {
            cpu_heavy.push(index);
        } else {
            gpu.push(index);
        }
    }
    let mut order = Vec::with_capacity(manifests.len());
    let mut cpu_iter = cpu_heavy.into_iter();
    let mut gpu_iter = gpu.into_iter();
    loop {
        match (cpu_iter.next(), gpu_iter.next()) {
            (None, None) => break,
            (cpu, gpu) => {
                if let Some(index) = cpu {
                    order.push(index);
                }
                if let Some(index) = gpu {
                    order.push(index);
                }
            }
        }
    }
    order
}

fn is_cpu_heavy(manifest: &PathBuf) -> bool {
    match lens_spec_metadata_from_manifest_path(manifest) {
        Ok(spec) => matches!(
            spec.runtime,
            LensRuntime::Algorithmic { .. }
                | LensRuntime::StaticLookup { .. }
                | LensRuntime::FastembedSparse { .. }
        ),
        Err(error) => {
            eprintln!(
                "{}",
                json!({
                    "event": "assay_lens_parallelism_classify_failed",
                    "manifest": manifest,
                    "code": error.code,
                    "message": error.message,
                    "scheduled_as": "gpu",
                })
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_gate_only_binds_above_one() {
        assert!(ensure_worker_vram_budget(1, None).is_ok());
        assert!(ensure_worker_vram_budget(3, Some(2048)).is_ok());
        // K>1 with neither flag nor env must refuse. The env var may leak in
        // from an outer FSV harness, so only assert when it is absent.
        if std::env::var(GPU_MEM_LIMIT_ENV).is_err() {
            let error = ensure_worker_vram_budget(3, None).expect_err("unbudgeted K>1");
            assert!(error.contains(GPU_MEM_LIMIT_ENV));
        }
    }

    #[test]
    fn interleave_pairs_cpu_heavy_with_gpu_and_covers_all_slots() {
        // Non-existent manifests classify as GPU (logged), so the order is a
        // permutation of all indices even when metadata is unreadable.
        let manifests: Vec<PathBuf> = (0..5)
            .map(|index| PathBuf::from(format!("missing-{index}.json")))
            .collect();
        let mut order = interleaved_start_order(&manifests);
        order.sort_unstable();
        assert_eq!(order, vec![0, 1, 2, 3, 4]);
    }
}
