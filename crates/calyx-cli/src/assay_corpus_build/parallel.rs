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
//! Co-residency safety: K>1 loads K models onto the GPU at once. Every
//! worker session must have an explicit #1143 arena budget, and known
//! allocator-heavy runtimes must run alone until the scheduler has a real
//! global CUDA reservation model.
//!
//! Start order pairs CPU-heavy lenses with GPU lenses first so the wall-clock
//! win of overlapping them is realized from the start of the run; slot
//! numbering and per-slot outputs are untouched by start order.

use std::path::PathBuf;

use calyx_registry::{FastembedBgem3Output, LensRuntime, lens_spec_metadata_from_manifest_path};
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

/// Refuse K-way schedules that are not safe under the current process model.
pub(crate) fn ensure_worker_vram_safety(
    lens_parallelism: usize,
    worker_gpu_mem_limit_mib: Option<usize>,
    manifests: &[PathBuf],
) -> Result<(), String> {
    ensure_worker_vram_budget(lens_parallelism, worker_gpu_mem_limit_mib)?;
    ensure_exclusive_gpu_runtimes_are_serial(lens_parallelism, manifests)
}

fn ensure_exclusive_gpu_runtimes_are_serial(
    lens_parallelism: usize,
    manifests: &[PathBuf],
) -> Result<(), String> {
    let co_resident = lens_parallelism.min(manifests.len());
    if co_resident <= 1 {
        return Ok(());
    }
    let exclusive = exclusive_gpu_runtime_lenses(manifests);
    if exclusive.is_empty() {
        return Ok(());
    }
    Err(format!(
        "lens-parallelism {lens_parallelism} would run up to {co_resident} workers co-resident, but these runtimes require exclusive GPU residency under the current scheduler: {}; use --lens-parallelism 1 for this roster or split the exclusive GPU lens into a separate pass",
        exclusive.join(", ")
    ))
}

fn exclusive_gpu_runtime_lenses(manifests: &[PathBuf]) -> Vec<String> {
    manifests
        .iter()
        .filter_map(
            |manifest| match lens_spec_metadata_from_manifest_path(manifest) {
                Ok(spec) => exclusive_gpu_runtime_name(&spec.runtime)
                    .map(|runtime| format!("{} ({runtime})", spec.name)),
                Err(error) => {
                    eprintln!(
                        "{}",
                        json!({
                            "event": "assay_lens_parallelism_vram_classify_failed",
                            "manifest": manifest,
                            "code": error.code,
                            "message": error.message,
                            "scheduled_as": "gpu",
                        })
                    );
                    None
                }
            },
        )
        .collect()
}

fn exclusive_gpu_runtime_name(runtime: &LensRuntime) -> Option<&'static str> {
    match runtime {
        LensRuntime::FastembedQwen3 { .. } => Some("fastembed-qwen3"),
        LensRuntime::OnnxColbert { .. } => Some("onnx-colbert"),
        LensRuntime::FastembedBgem3 {
            output: FastembedBgem3Output::Colbert,
            ..
        } => Some("fastembed-bgem3-colbert"),
        _ => None,
    }
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

    #[test]
    fn vram_safety_rejects_exclusive_gpu_runtime_parallelism() {
        let root = temp_root("assay-parallel-exclusive-gpu");
        let manifests = vec![
            write_manifest(&root, "exclusive-qwen", "fastembed-qwen3"),
            write_manifest(&root, "dense-onnx", "onnx"),
        ];

        let error =
            ensure_worker_vram_safety(2, Some(6144), &manifests).expect_err("exclusive GPU K>1");

        assert!(error.contains("exclusive-qwen (fastembed-qwen3)"));
        assert!(error.contains("--lens-parallelism 1"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn vram_safety_allows_exclusive_gpu_runtime_when_serial() {
        let root = temp_root("assay-parallel-exclusive-gpu-k1");
        let manifests = vec![write_manifest(&root, "exclusive-qwen", "fastembed-qwen3")];

        assert!(ensure_worker_vram_safety(1, None, &manifests).is_ok());
        assert!(ensure_worker_vram_safety(4, Some(6144), &manifests).is_ok());
        let _ = std::fs::remove_dir_all(root);
    }

    fn write_manifest(root: &std::path::Path, name: &str, runtime: &str) -> PathBuf {
        use calyx_core::{Modality, QuantPolicy};
        use calyx_registry::{LensForgeFile, LensForgeManifest};

        std::fs::create_dir_all(root).unwrap();
        let hash = "00".repeat(32);
        let manifest = LensForgeManifest {
            name: name.to_string(),
            modality: Modality::Text,
            runtime: runtime.to_string(),
            dim: 4,
            shape: None,
            dtype: "fp16".to_string(),
            weights_sha256: hash.clone(),
            artifact_set_sha256: Some(hash.clone()),
            files: vec![LensForgeFile {
                role: "model".to_string(),
                path: "model.onnx".into(),
                sha256: hash,
                bytes: 1,
            }],
            pooling: "mean".to_string(),
            norm: "unit".to_string(),
            source_hf_id: format!("calyx/{name}"),
            endpoint: None,
            license: Some("apache-2.0".to_string()),
            non_commercial: false,
            quant_default: QuantPolicy::turboquant_default(),
            truncate_dim: None,
            recall_delta: calyx_registry::spec::default_recall_delta(),
            max_batch: None,
            batch_policy: None,
        };
        let path = root.join(format!("{name}.json"));
        std::fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
        path
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }
}
