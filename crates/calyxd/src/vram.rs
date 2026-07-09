//! Daemon-level VRAM budget enforcer (PH65 · T03).
//!
//! `calyxd` can share one CUDA GPU with co-resident embedding services and
//! other GPU workloads. Before startup and before any Forge dispatch the daemon
//! must confirm the configured `vram_budget_mib` reservation fits *in addition
//! to* whatever is already resident. Any request that would breach either the
//! Calyx reservation or the physical board total fails closed with
//! `CALYX_FORGE_VRAM_BUDGET` — no silent over-allocation.
//!
//! Live usage is read via **NVML** (`nvml-wrapper`), not `cudaMemGetInfo`: NVML
//! is the library `nvidia-smi` itself uses, so it reports the true board total
//! and device-wide used bytes consistent with `nvidia-smi`, whereas
//! `cudaMemGetInfo` reports the runtime-usable total (~32110 MiB, net of the
//! CUDA context + driver reserve) and only post-context free. NVML is the right
//! source of truth for honoring every co-resident workload. The usage probe is injected
//! behind [`VramUsage`] so the budget arithmetic is unit-tested with
//! hand-computed MiB on any host.
//!
//! Power: Calyx does not schedule against power directly — it stays within the
//! VRAM budget, which keeps concurrent kernel pressure (and thus power) bounded.

use serde::Serialize;

use crate::cuda_probe::CudaDeviceInfo;
use crate::error::DaemonError;

const BYTES_PER_MIB: u64 = 1024 * 1024;

/// TEI endpoints documented for operator-facing budget-exhaustion errors.
const TEI_ENDPOINTS: &str = ":18190 (calyx-e5), :18188 (calyx-bge-m3), :8088 (legacy general), :8089 (reranker), :8090 (legal)";

/// A point-in-time device VRAM reading in MiB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VramReading {
    /// Total board VRAM reported by NVML.
    pub total_mib: u32,
    /// Device-wide VRAM currently in use by ALL processes (incl. resident TEI).
    pub used_mib: u32,
}

/// Source of live device VRAM usage. Production reads NVML; tests inject a
/// deterministic reading. A probe failure is `Err` — never a zero-fill.
pub trait VramUsage {
    fn read(&self) -> Result<VramReading, DaemonError>;
}

/// Startup audit snapshot, logged at boot and serialized for the health JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct VramAuditReport {
    /// Device-wide VRAM resident before Calyx allocates (the TEI + others footprint).
    pub tei_used_mib: u32,
    /// Configured Calyx ceiling from `calyx.toml`.
    pub calyx_budget_mib: u32,
    /// Board total VRAM (NVML), e.g. 32607.
    pub device_total_mib: u32,
}

/// Enforces the configured VRAM ceiling against live device usage.
pub struct VramBudget<U: VramUsage> {
    budget_mib: u32,
    device_total_mib: u32,
    resident_baseline_mib: u32,
    usage: U,
}

// Manual Debug (the NVML handle in `usage` is not `Debug`); prints the budget
// fields, which is what logs and `unwrap_err()` diagnostics actually want.
impl<U: VramUsage> std::fmt::Debug for VramBudget<U> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VramBudget")
            .field("budget_mib", &self.budget_mib)
            .field("device_total_mib", &self.device_total_mib)
            .field("resident_baseline_mib", &self.resident_baseline_mib)
            .finish_non_exhaustive()
    }
}

impl<U: VramUsage> VramBudget<U> {
    /// Direct constructor (used by `from_config` and by tests with a mock probe).
    pub fn new(
        budget_mib: u32,
        device_total_mib: u32,
        resident_baseline_mib: u32,
        usage: U,
    ) -> Self {
        Self {
            budget_mib,
            device_total_mib,
            resident_baseline_mib,
            usage,
        }
    }

    /// Build from the config budget + a probed device, validating the budget
    /// fits on the board after the resident footprint is reserved.
    ///
    /// Fail-closed at construction (not later at dispatch):
    /// - `vram_budget_mib > device board total` → `CALYX_FORGE_VRAM_BUDGET`.
    /// - `resident footprint + vram_budget_mib > device board total` →
    ///   `CALYX_FORGE_VRAM_BUDGET` naming the TEI endpoints.
    pub fn from_config(
        cfg_budget_mib: u32,
        device: &CudaDeviceInfo,
        usage: U,
    ) -> Result<Self, DaemonError> {
        let reading = usage.read()?;
        let device_total_mib = reading.total_mib;
        if cfg_budget_mib == 0 {
            return Err(DaemonError::vram_budget(
                "vram_budget_mib is 0; the daemon needs a positive budget to dispatch Forge work",
            ));
        }
        if cfg_budget_mib > device_total_mib {
            return Err(DaemonError::vram_budget(format!(
                "vram_budget_mib {cfg_budget_mib} exceeds device board total {device_total_mib} MiB \
                 (NVML); cuda-probe reported {} MiB usable",
                device.vram_total_mib
            )));
        }
        Self::ensure_device_headroom(reading.used_mib, cfg_budget_mib, device_total_mib)?;
        Ok(Self::new(
            cfg_budget_mib,
            device_total_mib,
            reading.used_mib,
            usage,
        ))
    }

    fn ensure_device_headroom(
        resident_mib: u32,
        budget_mib: u32,
        device_total_mib: u32,
    ) -> Result<(), DaemonError> {
        let projected = u64::from(resident_mib) + u64::from(budget_mib);
        if projected > u64::from(device_total_mib) {
            return Err(DaemonError::vram_budget(format!(
                "resident GPU footprint {resident_mib} MiB + vram_budget_mib {budget_mib} MiB \
                 = {projected} MiB exceeds device board total {device_total_mib} MiB; free GPU \
                 memory or lower the budget (resident TEI: {TEI_ENDPOINTS})"
            )));
        }
        Ok(())
    }

    /// Live device-wide VRAM in use (incl. resident TEI), in MiB.
    pub fn allocated_mib(&self) -> Result<u32, DaemonError> {
        Ok(self.usage.read()?.used_mib)
    }

    /// Best-effort Calyx-owned VRAM usage since construction. Co-tenant frees
    /// can make the board reading fall below the startup baseline; saturate to 0.
    pub fn calyx_used_mib(&self) -> Result<u32, DaemonError> {
        Ok(self
            .allocated_mib()?
            .saturating_sub(self.resident_baseline_mib))
    }

    /// Budget headroom remaining, bounded by both Calyx reservation and board
    /// free memory. Saturates at 0 (never underflows).
    pub fn available_mib(&self) -> Result<u32, DaemonError> {
        let allocated = self.allocated_mib()?;
        let calyx_remaining = self.budget_mib.saturating_sub(self.calyx_used_mib()?);
        let device_remaining = self.device_total_mib.saturating_sub(allocated);
        Ok(calyx_remaining.min(device_remaining))
    }

    /// Admission check before a Forge dispatch. Fails closed with
    /// `CALYX_FORGE_VRAM_BUDGET` if the request would breach either the Calyx
    /// reservation or the physical board total.
    pub fn check_can_allocate(&self, required_mib: u32) -> Result<(), DaemonError> {
        let used = self.allocated_mib()?;
        let calyx_used = used.saturating_sub(self.resident_baseline_mib);
        let calyx_projected = u64::from(calyx_used) + u64::from(required_mib);
        if calyx_projected > u64::from(self.budget_mib) {
            return Err(DaemonError::vram_budget(format!(
                "request {required_mib} MiB + Calyx in-use {calyx_used} MiB = {calyx_projected} \
                 MiB exceeds vram_budget_mib {} MiB (resident TEI: {TEI_ENDPOINTS})",
                self.budget_mib
            )));
        }
        let device_projected = u64::from(used) + u64::from(required_mib);
        if device_projected > u64::from(self.device_total_mib) {
            return Err(DaemonError::vram_budget(format!(
                "request {required_mib} MiB + device in-use {used} MiB = {device_projected} MiB \
                 exceeds device board total {} MiB (resident TEI: {TEI_ENDPOINTS})",
                self.device_total_mib
            )));
        }
        Ok(())
    }

    /// Snapshot the resident footprint vs budget at startup.
    pub fn startup_vram_audit(&self) -> Result<VramAuditReport, DaemonError> {
        let used = self.allocated_mib()?;
        Self::ensure_device_headroom(used, self.budget_mib, self.device_total_mib)?;
        Ok(VramAuditReport {
            tei_used_mib: used,
            calyx_budget_mib: self.budget_mib,
            device_total_mib: self.device_total_mib,
        })
    }
}

/// Production [`VramUsage`] backed by NVML (`nvml-wrapper`). NVML is loaded
/// dynamically at construction; absence of the driver/library is a loud error.
pub struct NvmlVramUsage {
    nvml: nvml_wrapper::Nvml,
}

impl NvmlVramUsage {
    /// Initialize NVML once (the constructor dynamically loads `libnvidia-ml`,
    /// which is comparatively expensive — hold the handle for the daemon's life).
    pub fn init() -> Result<Self, DaemonError> {
        // Load the driver-side library name for the host OS. Linux driver-only
        // hosts ship `libnvidia-ml.so.1` but not the unversioned dev symlink;
        // Windows driver hosts expose `nvml.dll` in System32.
        let library = nvml_library_name();
        let nvml = nvml_wrapper::Nvml::builder()
            .lib_path(std::ffi::OsStr::new(library))
            .init()
            .map_err(|err| {
                DaemonError::device_unavailable(format!(
                    "NVML init failed loading {library} (is the NVIDIA driver present?): {err}"
                ))
            })?;
        Ok(Self { nvml })
    }
}

impl VramUsage for NvmlVramUsage {
    fn read(&self) -> Result<VramReading, DaemonError> {
        let device = self.nvml.device_by_index(0).map_err(|err| {
            DaemonError::device_unavailable(format!("NVML device_by_index(0) failed: {err}"))
        })?;
        let mem = device.memory_info().map_err(|err| {
            DaemonError::device_unavailable(format!("NVML memory_info() failed: {err}"))
        })?;
        let total_mib = u32::try_from(mem.total / BYTES_PER_MIB).map_err(|_| {
            DaemonError::vram_budget(format!(
                "NVML total {} bytes does not fit u32 MiB",
                mem.total
            ))
        })?;
        let used_mib = u32::try_from(mem.used / BYTES_PER_MIB).map_err(|_| {
            DaemonError::vram_budget(format!("NVML used {} bytes does not fit u32 MiB", mem.used))
        })?;
        Ok(VramReading {
            total_mib,
            used_mib,
        })
    }
}

fn nvml_library_name() -> &'static str {
    if cfg!(windows) {
        "nvml.dll"
    } else {
        "libnvidia-ml.so.1"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic probe with a hand-set reading.
    struct MockUsage {
        reading: VramReading,
    }
    impl MockUsage {
        fn new(total_mib: u32, used_mib: u32) -> Self {
            Self {
                reading: VramReading {
                    total_mib,
                    used_mib,
                },
            }
        }
    }
    impl VramUsage for MockUsage {
        fn read(&self) -> Result<VramReading, DaemonError> {
            Ok(self.reading)
        }
    }

    fn device() -> CudaDeviceInfo {
        CudaDeviceInfo {
            device_name: "NVIDIA CUDA GPU".into(),
            vram_total_mib: 32110,
            compute_cap: "12.0".into(),
        }
    }

    #[test]
    fn check_can_allocate_at_and_over_budget() {
        // budget 8192, Calyx has used 4096 since the 20000 MiB resident baseline:
        // 4000 fits (8096<=8192), 4097 breaches (8193>8192).
        let budget = VramBudget::new(8192, 32607, 20000, MockUsage::new(32607, 24096));
        assert!(budget.check_can_allocate(4000).is_ok());
        let err = budget.check_can_allocate(4097).unwrap_err();
        assert_eq!(err.code(), "CALYX_FORGE_VRAM_BUDGET");
        assert!(err.to_string().contains("8192"));
    }

    #[test]
    fn from_config_rejects_budget_over_device_total() {
        let err =
            VramBudget::from_config(40000, &device(), MockUsage::new(32607, 4096)).unwrap_err();
        assert_eq!(err.code(), "CALYX_FORGE_VRAM_BUDGET");
        assert!(err.to_string().contains("exceeds device board total"));
    }

    #[test]
    fn allocated_at_budget_rejects_one_more() {
        let budget = VramBudget::new(8192, 32607, 20000, MockUsage::new(32607, 28192));
        assert_eq!(budget.available_mib().unwrap(), 0);
        assert_eq!(
            budget.check_can_allocate(1).unwrap_err().code(),
            "CALYX_FORGE_VRAM_BUDGET"
        );
    }

    #[test]
    fn available_saturates_at_zero_when_calyx_overcommitted() {
        // Calyx in-use 9000 > budget 8192 -> available saturates to 0, no underflow.
        let budget = VramBudget::new(8192, 32607, 20000, MockUsage::new(32607, 29000));
        assert_eq!(budget.available_mib().unwrap(), 0);
    }

    #[test]
    fn available_is_capped_by_board_headroom() {
        // Calyx has 4096 MiB budget left, but the board has only 1000 MiB free.
        let budget = VramBudget::new(8192, 32607, 27511, MockUsage::new(32607, 31607));
        assert_eq!(budget.available_mib().unwrap(), 1000);
    }

    #[test]
    fn zero_budget_admits_zero_rejects_one() {
        let err0 = VramBudget::from_config(0, &device(), MockUsage::new(32607, 0)).unwrap_err();
        assert_eq!(err0.code(), "CALYX_FORGE_VRAM_BUDGET");
        // A directly-constructed zero budget admits a zero-byte request, rejects 1.
        let budget = VramBudget::new(0, 32607, 0, MockUsage::new(32607, 0));
        assert!(budget.check_can_allocate(0).is_ok());
        assert_eq!(
            budget.check_can_allocate(1).unwrap_err().code(),
            "CALYX_FORGE_VRAM_BUDGET"
        );
    }

    #[test]
    fn from_config_allows_residents_larger_than_calyx_budget_when_board_has_room() {
        // Shared-host shape: resident footprint is much larger than the 4096 MiB Forge budget,
        // but resident + budget still fits the device.
        let budget =
            VramBudget::from_config(4096, &device(), MockUsage::new(32607, 20255)).unwrap();
        assert_eq!(budget.startup_vram_audit().unwrap().tei_used_mib, 20255);
        assert_eq!(budget.available_mib().unwrap(), 4096);
    }

    #[test]
    fn from_config_fails_closed_when_residents_plus_budget_exhaust_device() {
        // 30000 resident + 4096 budget > 32607 board total -> fail closed, name TEI endpoints.
        let err =
            VramBudget::from_config(4096, &device(), MockUsage::new(32607, 30000)).unwrap_err();
        assert_eq!(err.code(), "CALYX_FORGE_VRAM_BUDGET");
        let shown = err.to_string();
        assert!(shown.contains("exceeds device board total"));
        assert!(shown.contains(":18190"));
        assert!(shown.contains(":8088"));
    }

    #[test]
    fn startup_audit_reports_footprint_budget_and_total() {
        let budget = VramBudget::from_config(8192, &device(), MockUsage::new(32607, 7628)).unwrap();
        let report = budget.startup_vram_audit().unwrap();
        assert_eq!(report.tei_used_mib, 7628);
        assert_eq!(report.calyx_budget_mib, 8192);
        assert_eq!(report.device_total_mib, 32607);
        // Calyx has not allocated since construction, so its full budget is available.
        assert_eq!(budget.available_mib().unwrap(), 8192);
    }

    #[test]
    fn startup_audit_rechecks_device_headroom() {
        let budget = VramBudget::new(4096, 32607, 20000, MockUsage::new(32607, 30000));
        let err = budget.startup_vram_audit().unwrap_err();
        assert_eq!(err.code(), "CALYX_FORGE_VRAM_BUDGET");
        assert!(err.to_string().contains("exceeds device board total"));
    }

    #[test]
    fn nvml_library_name_matches_target_os() {
        if cfg!(windows) {
            assert_eq!(nvml_library_name(), "nvml.dll");
        } else {
            assert_eq!(nvml_library_name(), "libnvidia-ml.so.1");
        }
    }
}
