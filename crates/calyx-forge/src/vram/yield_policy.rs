//! Anneal yield policy for PH57 T05.
//!
//! The policy gives serving/search/embed work priority over Anneal background
//! math by splitting VRAM accounting, capping Anneal reservations, and backing
//! off Anneal dispatch when GPU power approaches the host budget.

use std::process::Command;
use std::time::Duration;

use crate::vram::{Category, VRAM_BUDGET_REMEDIATION, VramBudgeter, VramGuard, VramProbe};
use crate::{ForgeError, Result};

pub const ANNEAL_VRAM_BUDGET_ENV: &str = "CALYX_ANNEAL_VRAM_BUDGET";
pub const DEFAULT_ANNEAL_VRAM_CAP_BYTES: usize = 2 * 1024 * 1024 * 1024;
pub const DEFAULT_SERVING_STREAM_PRIORITY: i32 = 0;
pub const DEFAULT_ANNEAL_STREAM_PRIORITY: i32 = -1;
pub const DEFAULT_POWER_BACKOFF_THRESHOLD_W: u32 = 560;
pub const DEFAULT_ANNEAL_THROTTLE_SLEEP: Duration = Duration::from_millis(50);

const GPU_REMEDIATION: &str =
    "Check NVIDIA driver/NVML/nvidia-smi availability and confirm GPU power readback works";

#[cfg(feature = "cuda")]
pub type CudaStream = std::sync::Arc<cudarc::driver::CudaStream>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct YieldPolicy {
    pub anneal_vram_cap_bytes: usize,
    pub serving_stream_priority: i32,
    pub anneal_stream_priority: i32,
    pub power_backoff_threshold_w: u32,
}

impl Default for YieldPolicy {
    fn default() -> Self {
        Self {
            anneal_vram_cap_bytes: DEFAULT_ANNEAL_VRAM_CAP_BYTES,
            serving_stream_priority: DEFAULT_SERVING_STREAM_PRIORITY,
            anneal_stream_priority: DEFAULT_ANNEAL_STREAM_PRIORITY,
            power_backoff_threshold_w: DEFAULT_POWER_BACKOFF_THRESHOLD_W,
        }
    }
}

impl YieldPolicy {
    /// Build from environment. Invalid `CALYX_ANNEAL_VRAM_BUDGET` fails closed by
    /// setting the Anneal cap to zero and logging the exact parsing error.
    pub fn from_env() -> Self {
        let mut policy = Self::default();
        let raw = std::env::var(ANNEAL_VRAM_BUDGET_ENV).ok();
        match parse_anneal_vram_cap(raw.as_deref()) {
            Ok(cap) => policy.anneal_vram_cap_bytes = cap,
            Err(err) => {
                tracing::error!(
                    target: "calyx_forge::vram::yield_policy",
                    code = err.code(),
                    error = %err,
                    "invalid Anneal VRAM budget; failing closed with anneal_vram_cap_bytes=0"
                );
                policy.anneal_vram_cap_bytes = 0;
            }
        }
        policy
    }

    /// Current Anneal usage must be at or below the configured sub-budget cap.
    pub fn anneal_budget_check<P: VramProbe>(&self, budgeter: &VramBudgeter<P>) -> Result<()> {
        let allocated = budgeter.allocated_bytes_for(Category::Anneal);
        if allocated > self.anneal_vram_cap_bytes {
            budgeter.record_anneal_vram_rejection();
            return Err(vram_budget_err(format!(
                "Anneal VRAM budget exceeded: anneal_allocated_bytes={allocated} > anneal_vram_cap_bytes={}",
                self.anneal_vram_cap_bytes
            )));
        }
        Ok(())
    }

    /// Reserve Anneal VRAM with the background sub-budget enforced atomically.
    pub fn reserve_anneal<'b, P: VramProbe>(
        &self,
        budgeter: &'b VramBudgeter<P>,
        bytes: usize,
    ) -> Result<VramGuard<'b, P>> {
        match budgeter.reserve_category_with_cap(
            bytes,
            Category::Anneal,
            self.anneal_vram_cap_bytes,
        ) {
            Ok(guard) => Ok(guard),
            Err(err) => {
                budgeter.record_anneal_vram_rejection();
                Err(err)
            }
        }
    }

    /// Query current GPU power draw in watts using NVML, then nvidia-smi.
    pub fn query_power_draw_w() -> Result<u32> {
        NvmlPowerProbe::default().power_draw_w()
    }

    /// Return true when Anneal should back off. Unknown power is non-fatal and
    /// returns false after logging, matching the issue contract.
    pub fn should_throttle_anneal(&self) -> bool {
        self.should_throttle_with(&NvmlPowerProbe::default())
    }

    pub fn should_throttle_with<P: PowerProbe>(&self, probe: &P) -> bool {
        match probe.power_draw_w() {
            Ok(power_w) => power_w > self.power_backoff_threshold_w,
            Err(err) => {
                tracing::warn!(
                    target: "calyx_forge::vram::yield_policy",
                    code = err.code(),
                    error = %err,
                    "GPU power draw unknown; Anneal throttle decision is false"
                );
                false
            }
        }
    }

    /// Record and sleep if current power requires Anneal throttling.
    pub fn throttle_anneal_if_needed<P: VramProbe>(&self, budgeter: &VramBudgeter<P>) -> bool {
        self.throttle_anneal_if_needed_with(
            budgeter,
            &NvmlPowerProbe::default(),
            Some(DEFAULT_ANNEAL_THROTTLE_SLEEP),
        )
    }

    pub fn throttle_anneal_if_needed_with<B, P>(
        &self,
        budgeter: &VramBudgeter<B>,
        probe: &P,
        sleep: Option<Duration>,
    ) -> bool
    where
        B: VramProbe,
        P: PowerProbe,
    {
        if !self.should_throttle_with(probe) {
            return false;
        }
        budgeter.record_anneal_throttle_event();
        if let Some(duration) = sleep {
            std::thread::sleep(duration);
        }
        true
    }

    #[cfg(feature = "cuda")]
    pub fn create_anneal_stream(&self) -> Result<CudaStream> {
        let ctx = crate::cuda::init_cuda(0, false)?;
        self.create_anneal_stream_for_context(&ctx)
    }

    #[cfg(feature = "cuda")]
    pub fn create_serving_stream(&self) -> Result<CudaStream> {
        let ctx = crate::cuda::init_cuda(0, false)?;
        self.create_serving_stream_for_context(&ctx)
    }

    #[cfg(feature = "cuda")]
    pub fn create_anneal_stream_for_context(
        &self,
        ctx: &crate::cuda::CudaContext,
    ) -> Result<CudaStream> {
        let priority = self.resolved_anneal_cuda_priority(ctx)?;
        ctx.inner()
            .new_stream_with_priority(priority)
            .map_err(driver_gpu_err)
    }

    #[cfg(feature = "cuda")]
    pub fn create_serving_stream_for_context(
        &self,
        ctx: &crate::cuda::CudaContext,
    ) -> Result<CudaStream> {
        let priority = self.resolved_serving_cuda_priority(ctx)?;
        ctx.inner()
            .new_stream_with_priority(priority)
            .map_err(driver_gpu_err)
    }

    #[cfg(feature = "cuda")]
    pub fn resolved_serving_cuda_priority(&self, ctx: &crate::cuda::CudaContext) -> Result<i32> {
        self.resolve_cuda_priority(ctx, self.serving_stream_priority)
    }

    #[cfg(feature = "cuda")]
    pub fn resolved_anneal_cuda_priority(&self, ctx: &crate::cuda::CudaContext) -> Result<i32> {
        self.resolve_cuda_priority(ctx, self.anneal_stream_priority)
    }

    #[cfg(feature = "cuda")]
    pub fn stream_priority_range_for_context(ctx: &crate::cuda::CudaContext) -> Result<(i32, i32)> {
        ctx.inner().bind_to_thread().map_err(driver_gpu_err)?;
        cudarc::driver::result::stream::get_priority_range().map_err(driver_gpu_err)
    }

    #[cfg(feature = "cuda")]
    fn resolve_cuda_priority(
        &self,
        ctx: &crate::cuda::CudaContext,
        logical_priority: i32,
    ) -> Result<i32> {
        let (least_priority, greatest_priority) = Self::stream_priority_range_for_context(ctx)?;
        if least_priority == greatest_priority {
            return Ok(least_priority);
        }
        if logical_priority >= self.serving_stream_priority {
            Ok(greatest_priority)
        } else {
            Ok(least_priority)
        }
    }
}

pub trait PowerProbe {
    fn power_draw_w(&self) -> Result<u32>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NvmlPowerProbe {
    device_index: u32,
}

impl NvmlPowerProbe {
    pub fn with_device_index(device_index: u32) -> Self {
        Self { device_index }
    }

    fn nvml_power_draw_w(&self) -> Result<u32> {
        let nvml = nvml_wrapper::Nvml::init().map_err(|err| {
            gpu_error(format!("NVML init failed while querying GPU power: {err}"))
        })?;
        let device = nvml.device_by_index(self.device_index).map_err(|err| {
            gpu_error(format!(
                "NVML device_by_index({}) failed while querying GPU power: {err}",
                self.device_index
            ))
        })?;
        let milliwatts = device.power_usage().map_err(|err| {
            gpu_error(format!(
                "NVML power_usage failed for device {}: {err}",
                self.device_index
            ))
        })?;
        Ok(milliwatts.saturating_add(999) / 1000)
    }
}

impl PowerProbe for NvmlPowerProbe {
    fn power_draw_w(&self) -> Result<u32> {
        match self.nvml_power_draw_w() {
            Ok(power_w) => Ok(power_w),
            Err(nvml_err) => nvidia_smi_power_draw_w(self.device_index).map_err(|smi_err| {
                gpu_error(format!(
                    "GPU power query failed via NVML ({nvml_err}) and nvidia-smi ({smi_err})"
                ))
            }),
        }
    }
}

fn nvidia_smi_power_draw_w(device_index: u32) -> Result<u32> {
    let output = Command::new("nvidia-smi")
        .args([
            format!("--id={device_index}"),
            "--query-gpu=power.draw".to_string(),
            "--format=csv,noheader,nounits".to_string(),
        ])
        .output()
        .map_err(|err| gpu_error(format!("nvidia-smi power query failed to launch: {err}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(gpu_error(format!(
            "nvidia-smi power query exited with status {}: {}",
            output.status,
            stderr.trim()
        )));
    }

    parse_nvidia_smi_power_stdout(&output.stdout)
}

fn parse_nvidia_smi_power_stdout(stdout: &[u8]) -> Result<u32> {
    let text = std::str::from_utf8(stdout)
        .map_err(|err| gpu_error(format!("nvidia-smi power output was not UTF-8: {err}")))?;
    let first = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or_else(|| gpu_error("nvidia-smi power output was empty".to_string()))?;
    let watts = first
        .trim_end_matches('W')
        .trim()
        .parse::<f64>()
        .map_err(|err| {
            gpu_error(format!(
                "nvidia-smi power output {first:?} was invalid: {err}"
            ))
        })?;
    if !watts.is_finite() || watts < 0.0 || watts > u32::MAX as f64 {
        return Err(gpu_error(format!(
            "nvidia-smi power output {first:?} was outside valid watt range"
        )));
    }
    Ok(watts.ceil() as u32)
}

fn parse_anneal_vram_cap(raw: Option<&str>) -> Result<usize> {
    match raw {
        None => Ok(DEFAULT_ANNEAL_VRAM_CAP_BYTES),
        Some(value) => value.trim().parse::<usize>().map_err(|_| {
            vram_budget_err(format!(
                "{ANNEAL_VRAM_BUDGET_ENV}={value:?} is not a valid byte count"
            ))
        }),
    }
}

fn vram_budget_err(detail: String) -> ForgeError {
    ForgeError::VramBudget {
        detail,
        remediation: VRAM_BUDGET_REMEDIATION.to_string(),
    }
}

fn gpu_error(detail: String) -> ForgeError {
    ForgeError::GpuError {
        detail,
        remediation: GPU_REMEDIATION.to_string(),
    }
}

#[cfg(feature = "cuda")]
fn driver_gpu_err(err: cudarc::driver::DriverError) -> ForgeError {
    gpu_error(format!("CUDA stream priority operation failed: {err}"))
}

#[cfg(test)]
#[path = "yield_policy_tests.rs"]
mod tests;
