//! CUDA device preflight for `calyxd` (PH65 · T02).
//!
//! In server mode any CUDA initialization failure is immediately fatal with a
//! structured [`DaemonError::DeviceUnavailable`] (`CALYX_FORGE_DEVICE_UNAVAILABLE`)
//! — there is NO silent fallback to CPU (16 §4, A16). The probe runs at startup
//! before the daemon accepts any work, so a GPU-less or mis-driver'd host fails
//! loud at boot instead of degrading silently at dispatch time.
//!
//! The env var `CALYX_FORCE_CUDA_FAIL=1` forces the failure path deterministically
//! for FSV (only the exact string `"1"` triggers it).

use crate::error::DaemonError;

/// Env var that deterministically forces the failure path (FSV injection).
pub const FORCE_FAIL_ENV: &str = "CALYX_FORCE_CUDA_FAIL";

/// Device facts captured at a successful CUDA init. Logged at startup and reused
/// by the VRAM budget enforcer (T03) and the healthcheck (T04).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CudaDeviceInfo {
    /// Marketing name reported by the CUDA/NVML stack.
    pub device_name: String,
    /// Total device VRAM in MiB.
    pub vram_total_mib: u32,
    /// Compute capability as `"major.minor"`, e.g. `"12.0"` (sm_120).
    pub compute_cap: String,
}

/// Probe the CUDA device `calyxd` will run Forge on. Fatal on any failure;
/// never returns a CPU-fallback placeholder.
///
/// `CALYX_FORCE_CUDA_FAIL=1` short-circuits to a forced failure for FSV. Any
/// other value (absent, `"0"`, etc.) runs the real probe.
pub fn probe_cuda_device() -> Result<CudaDeviceInfo, DaemonError> {
    if std::env::var(FORCE_FAIL_ENV).as_deref() == Ok("1") {
        return Err(DaemonError::device_unavailable(format!(
            "forced by {FORCE_FAIL_ENV}=1 (deterministic FSV injection)"
        )));
    }
    probe_real_device()
}

#[cfg(feature = "cuda")]
fn probe_real_device() -> Result<CudaDeviceInfo, DaemonError> {
    // Real `cudaSetDevice`/`cuInit` via calyx-forge. determinism=false: the
    // budgeter/probe don't need the deterministic-kernel mode here.
    let ctx = calyx_forge::init_cuda(0, false).map_err(|err| {
        DaemonError::device_unavailable(format!("CUDA init on device 0 failed: {err}"))
    })?;
    let (major, minor) = ctx.compute_capability();
    let total = ctx.total_mem_mib();
    let vram_total_mib = u32::try_from(total).map_err(|_| {
        DaemonError::device_unavailable(format!("device VRAM {total} MiB does not fit u32"))
    })?;
    Ok(CudaDeviceInfo {
        device_name: ctx.name().to_string(),
        vram_total_mib,
        compute_cap: format!("{major}.{minor}"),
    })
}

#[cfg(not(feature = "cuda"))]
fn probe_real_device() -> Result<CudaDeviceInfo, DaemonError> {
    // Fail loud: a non-CUDA build cannot serve in GPU mode. This is the absence
    // of a capability, not a fallback — the daemon refuses to start.
    Err(DaemonError::device_unavailable(
        "calyxd was built without the `cuda` feature; rebuild with `--features cuda` \
         on an NVIDIA GPU host (server mode requires a working GPU and will not start without one)",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // CALYX_FORCE_CUDA_FAIL is process-global; serialize the env-mutating tests.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn force_fail_one_returns_device_unavailable() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: serialized by ENV_LOCK; restored below.
        unsafe { std::env::set_var(FORCE_FAIL_ENV, "1") };
        let result = probe_cuda_device();
        unsafe { std::env::remove_var(FORCE_FAIL_ENV) };

        let error = result.expect_err("force-fail must error");
        assert_eq!(error.code(), "CALYX_FORGE_DEVICE_UNAVAILABLE");
        let shown = error.to_string();
        assert!(shown.contains("CALYX_FORGE_DEVICE_UNAVAILABLE"));
        assert!(shown.contains("remediation:"));
        assert!(shown.contains("forced by CALYX_FORCE_CUDA_FAIL"));
    }

    #[test]
    fn force_fail_zero_is_not_the_injected_failure() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: serialized by ENV_LOCK; restored below.
        unsafe { std::env::set_var(FORCE_FAIL_ENV, "0") };
        let result = probe_cuda_device();
        unsafe { std::env::remove_var(FORCE_FAIL_ENV) };

        // "0" must NOT trigger the injection. On a non-cuda build the real probe
        // still fails loud (no GPU compiled in); on a cuda build with a live GPU
        // it succeeds. Either outcome is valid — it just must not be the forced
        // injection detail.
        if let Err(error) = result {
            assert!(
                !error.to_string().contains("forced by"),
                "value \"0\" must not trigger the FSV injection"
            );
        }
    }

    #[test]
    fn absent_env_runs_real_probe_without_panic() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: serialized by ENV_LOCK.
        unsafe { std::env::remove_var(FORCE_FAIL_ENV) };
        // Must not panic. On a non-cuda build this is Err(DeviceUnavailable);
        // on a cuda+GPU host it is Ok. We only assert it returns (no panic) and,
        // if Err, carries the right fail-loud code.
        if let Err(error) = probe_cuda_device() {
            assert_eq!(error.code(), "CALYX_FORGE_DEVICE_UNAVAILABLE");
        }
    }
}
