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
//! Co-residency safety: K>1 loads K models onto the GPU at once. A per-worker
//! ONNX arena budget (#1143) is necessary but NOT sufficient — `gpu_mem_limit`
//! bounds only the execution-provider arena; the CUDA primary context and the
//! cuDNN/cuBLAS handles and their workspaces are allocated outside it, so total
//! per-process device use is higher (microsoft/onnxruntime#7612). The scheduler
//! therefore runs a measured preflight (#1263): it sums the estimated resident
//! footprint of the K co-resident GPU workers — each worker's declared arena cap
//! plus a per-process context/workspace overhead — probes live free VRAM via the
//! same NVML source `lens add` placement uses, and fails closed with the derived
//! max-safe-K when the schedule would exceed the board. The safe K is thus
//! derived from measured memory, never a static runtime allowlist.
//!
//! Start order pairs CPU-heavy lenses with GPU lenses first so the wall-clock
//! win of overlapping them is realized from the start of the run; slot
//! numbering and per-slot outputs are untouched by start order.

use std::path::PathBuf;

use calyx_registry::{LensRuntime, lens_spec_metadata_from_manifest_path};
use calyxd::vram::{NvmlVramUsage, VramUsage};
use serde_json::json;

pub(crate) const GPU_MEM_LIMIT_ENV: &str = "CALYX_ONNX_GPU_MEM_LIMIT_MIB";

const BYTES_PER_MIB: u64 = 1024 * 1024;

/// Per-worker VRAM that lives OUTSIDE the ONNX arena cap: the CUDA primary
/// context plus cuBLAS/cuDNN handles and their default workspaces. ORT's
/// `gpu_mem_limit` bounds only the EP arena, so an estimate that used the arena
/// cap alone would under-count and still OOM (microsoft/onnxruntime#7612).
/// Conservative default; override with `CALYX_GPU_WORKER_CONTEXT_OVERHEAD_MIB`.
const DEFAULT_WORKER_CONTEXT_OVERHEAD_MIB: u64 = 1024;
const WORKER_CONTEXT_OVERHEAD_ENV: &str = "CALYX_GPU_WORKER_CONTEXT_OVERHEAD_MIB";

/// VRAM held back for the NVIDIA driver, other GPU tenants, and allocation
/// spikes (a model's arena can transiently exceed its steady state) so a
/// co-resident set never claims the whole board. Matches the `lens add`
/// placement headroom convention. Override with `CALYX_GPU_HEADROOM_BYTES`.
const DEFAULT_GPU_HEADROOM_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const GPU_HEADROOM_ENV: &str = "CALYX_GPU_HEADROOM_BYTES";

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

/// Refuse K-way schedules whose measured co-resident VRAM footprint exceeds the
/// live free VRAM on the device (#1263). Fails closed with the derived max-safe
/// K, never a static runtime allowlist.
pub(crate) fn ensure_worker_vram_safety(
    lens_parallelism: usize,
    worker_gpu_mem_limit_mib: Option<usize>,
    manifests: &[PathBuf],
) -> Result<(), String> {
    ensure_worker_vram_budget(lens_parallelism, worker_gpu_mem_limit_mib)?;

    // Only GPU-resident workers co-locate on the board; CPU-heavy lenses
    // (algorithmic/static/sparse) hold no CUDA context. A single GPU worker is
    // the sequential footprint and carries no co-residency risk.
    let gpu_workers = gpu_worker_count(lens_parallelism, manifests);
    if gpu_workers <= 1 {
        return Ok(());
    }

    let arena_cap_mib = resolve_arena_cap_mib(worker_gpu_mem_limit_mib)?.ok_or_else(|| {
        format!(
            "lens-parallelism {lens_parallelism} passed the arena-budget gate but no numeric \
             {GPU_MEM_LIMIT_ENV} / --worker-gpu-mem-limit-mib value could be resolved for the \
             co-residency preflight"
        )
    })?;
    let overhead_bytes = context_overhead_bytes()?;
    let per_worker_bytes = arena_cap_mib
        .saturating_mul(BYTES_PER_MIB)
        .saturating_add(overhead_bytes);
    let headroom_bytes = gpu_headroom_bytes()?;
    let free_bytes = probe_free_vram_bytes()?;

    let estimated_bytes = (gpu_workers as u64).saturating_mul(per_worker_bytes);
    let usable_bytes = free_bytes.saturating_sub(headroom_bytes);
    eprintln!(
        "{}",
        json!({
            "event": "assay_lens_parallelism_vram_preflight",
            "gpu_workers": gpu_workers,
            "per_worker_mib": per_worker_bytes / BYTES_PER_MIB,
            "arena_cap_mib": arena_cap_mib,
            "context_overhead_mib": overhead_bytes / BYTES_PER_MIB,
            "estimated_mib": estimated_bytes / BYTES_PER_MIB,
            "free_mib": free_bytes / BYTES_PER_MIB,
            "headroom_mib": headroom_bytes / BYTES_PER_MIB,
            "usable_mib": usable_bytes / BYTES_PER_MIB,
        })
    );

    evaluate_coresidency(gpu_workers, per_worker_bytes, free_bytes, headroom_bytes)
}

/// How many workers can be GPU-resident at once: min(K, count of GPU lenses).
/// A manifest whose metadata cannot be read is counted as GPU (the conservative,
/// fail-safe classification the start-order scheduler also uses).
fn gpu_worker_count(lens_parallelism: usize, manifests: &[PathBuf]) -> usize {
    let gpu_lenses = manifests.iter().filter(|m| !is_cpu_heavy(m)).count();
    lens_parallelism.min(gpu_lenses)
}

/// Pure co-residency arithmetic (no IO) so it is unit-testable on a CPU-only
/// host with a hand-set free-VRAM reading.
fn evaluate_coresidency(
    gpu_workers: usize,
    per_worker_bytes: u64,
    free_bytes: u64,
    headroom_bytes: u64,
) -> Result<(), String> {
    let usable_bytes = free_bytes.saturating_sub(headroom_bytes);
    let estimated_bytes = (gpu_workers as u64).saturating_mul(per_worker_bytes);
    if estimated_bytes <= usable_bytes {
        return Ok(());
    }
    let max_safe_k = usable_bytes
        .checked_div(per_worker_bytes)
        .map(|k| k as usize)
        .unwrap_or(gpu_workers);
    let safe_hint = if max_safe_k >= 1 {
        format!("re-run with --lens-parallelism {max_safe_k}")
    } else {
        "even one GPU worker will not fit — free device VRAM or lower --worker-gpu-mem-limit-mib"
            .to_string()
    };
    Err(format!(
        "lens-parallelism co-residency preflight failed: {gpu_workers} GPU workers need ~{} MiB \
         (~{} MiB each) but only ~{} MiB of the ~{} MiB free is usable after {} MiB headroom; {safe_hint}",
        estimated_bytes / BYTES_PER_MIB,
        per_worker_bytes / BYTES_PER_MIB,
        usable_bytes / BYTES_PER_MIB,
        free_bytes / BYTES_PER_MIB,
        headroom_bytes / BYTES_PER_MIB,
    ))
}

/// Effective per-worker arena cap in MiB: the explicit flag wins, else the env
/// the worker inherits. Returns `Ok(None)` only when neither is set (K>1 without
/// a budget is already refused upstream); a malformed env fails closed.
fn resolve_arena_cap_mib(worker_gpu_mem_limit_mib: Option<usize>) -> Result<Option<u64>, String> {
    if let Some(mib) = worker_gpu_mem_limit_mib {
        return Ok(Some(mib as u64));
    }
    match std::env::var(GPU_MEM_LIMIT_ENV) {
        Ok(raw) if !raw.trim().is_empty() => raw.trim().parse::<u64>().map(Some).map_err(|error| {
            format!("{GPU_MEM_LIMIT_ENV}={raw:?} is not a valid MiB integer: {error}")
        }),
        _ => Ok(None),
    }
}

fn context_overhead_bytes() -> Result<u64, String> {
    Ok(env_u64(
        WORKER_CONTEXT_OVERHEAD_ENV,
        DEFAULT_WORKER_CONTEXT_OVERHEAD_MIB,
    )?
    .saturating_mul(BYTES_PER_MIB))
}

fn gpu_headroom_bytes() -> Result<u64, String> {
    env_u64(GPU_HEADROOM_ENV, DEFAULT_GPU_HEADROOM_BYTES)
}

fn env_u64(name: &str, default: u64) -> Result<u64, String> {
    match std::env::var(name) {
        Ok(raw) if !raw.trim().is_empty() => raw
            .trim()
            .parse::<u64>()
            .map_err(|error| format!("{name}={raw:?} is not a valid unsigned integer: {error}")),
        _ => Ok(default),
    }
}

/// Live free VRAM (total − used) in bytes via NVML — the same source of truth
/// `lens add` placement and the daemon budget enforcer use. Fails closed with
/// remediation when the driver/library is unreachable (doctrine: never guess a
/// budget).
fn probe_free_vram_bytes() -> Result<u64, String> {
    let reading = NvmlVramUsage::init()
        .map_err(|error| {
            format!(
                "co-residency preflight could not probe GPU VRAM via NVML ({error}); ensure the \
                 NVIDIA driver NVML library is reachable, or run with --lens-parallelism 1"
            )
        })?
        .read()
        .map_err(|error| format!("co-residency preflight NVML memory read failed: {error}"))?;
    let total = u64::from(reading.total_mib).saturating_mul(BYTES_PER_MIB);
    let used = u64::from(reading.used_mib).saturating_mul(BYTES_PER_MIB);
    Ok(total.saturating_sub(used))
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

    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * MIB;

    #[test]
    fn coresidency_accepts_a_schedule_that_fits() {
        // 3 workers × (6 GiB arena + 1 GiB overhead) = 21 GiB, 4 GiB headroom.
        // A 32 GiB board with 28 GiB free leaves 24 GiB usable ≥ 21 GiB → fits.
        let per_worker = 7 * GIB;
        assert!(evaluate_coresidency(3, per_worker, 28 * GIB, 4 * GIB).is_ok());
    }

    #[test]
    fn coresidency_refuses_over_commit_and_reports_max_safe_k() {
        // Same 7 GiB/worker but only 21 GiB free (the #1263 post-OOM readback):
        // usable = 17 GiB, so at most floor(17/7)=2 workers fit; K=3 must refuse.
        let per_worker = 7 * GIB;
        let error = evaluate_coresidency(3, per_worker, 21 * GIB, 4 * GIB)
            .expect_err("3 workers over-commit 21 GiB free");
        assert!(error.contains("co-residency preflight failed"), "{error}");
        assert!(error.contains("--lens-parallelism 2"), "{error}");
    }

    #[test]
    fn coresidency_refuses_when_not_even_one_worker_fits() {
        let per_worker = 20 * GIB;
        let error = evaluate_coresidency(2, per_worker, 10 * GIB, 4 * GIB)
            .expect_err("worker larger than the whole board");
        assert!(
            error.contains("even one GPU worker will not fit"),
            "{error}"
        );
    }

    #[test]
    fn gpu_worker_count_excludes_cpu_heavy_lenses_and_caps_at_k() {
        let root = temp_root("assay-parallel-gpu-count");
        let manifests = vec![
            write_manifest(&root, "dense-a", "onnx"),
            write_manifest(&root, "dense-b", "onnx"),
            write_manifest(&root, "sparse-splade-a", "fastembed-sparse"),
            write_manifest(&root, "sparse-splade-b", "fastembed-sparse"),
        ];
        // 2 GPU lenses (onnx), 2 CPU-heavy (fastembed-sparse).
        assert_eq!(gpu_worker_count(4, &manifests), 2);
        // K caps the co-resident count below the GPU-lens total.
        assert_eq!(gpu_worker_count(1, &manifests), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn arena_cap_flag_wins_over_env_and_malformed_env_fails_closed() {
        assert_eq!(resolve_arena_cap_mib(Some(6144)).unwrap(), Some(6144));
    }

    /// Hardware FSV (#1263): drives the REAL NVML probe and the full preflight
    /// against the live device. Gated so CPU-only CI skips it; run on a box with
    /// the NVIDIA driver via `CALYX_FSV_GPU=1 cargo test -- --nocapture
    /// fsv_live_nvml`. Self-calibrating from the live free reading so the
    /// accept/refuse assertions hold regardless of the board's current free VRAM.
    #[test]
    fn fsv_live_nvml_coresidency_preflight() {
        if std::env::var("CALYX_FSV_GPU")
            .map(|v| v.trim().is_empty())
            .unwrap_or(true)
        {
            eprintln!("skip fsv_live_nvml_coresidency_preflight (set CALYX_FSV_GPU=1 to run)");
            return;
        }
        let free_bytes = probe_free_vram_bytes().expect("live NVML free-VRAM probe");
        let free_mib = free_bytes / BYTES_PER_MIB;
        eprintln!(
            "{}",
            json!({ "event": "fsv_live_nvml_free", "free_mib": free_mib })
        );
        assert!(free_mib > 0, "NVML reported zero free VRAM");

        let root = temp_root("assay-parallel-fsv-nvml");
        let manifests = vec![
            write_manifest(&root, "dense-0", "onnx"),
            write_manifest(&root, "dense-1", "onnx"),
            write_manifest(&root, "dense-2", "onnx"),
        ];

        // Over-commit: 3 workers each sized to the whole free board cannot co-reside.
        let over_cap = free_mib as usize;
        let over = ensure_worker_vram_safety(3, Some(over_cap), &manifests)
            .expect_err("3 workers each sized to the whole board must refuse");
        assert!(over.contains("co-residency preflight failed"), "{over}");

        // Fit: a small per-worker cap that comfortably fits 3 workers + headroom.
        ensure_worker_vram_safety(3, Some(256), &manifests)
            .expect("3 small workers fit the live board");

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
