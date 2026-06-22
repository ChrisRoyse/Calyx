use std::sync::{Arc, OnceLock};

use cudarc::driver::{CudaContext as CudarcContext, CudaModule};

use crate::{BackendKind, DeviceInfo, ForgeError, Result};

const BYTES_PER_MIB: u64 = 1024 * 1024;
const MIN_FREE_VRAM_MIB: u64 = 4096;
const CUDA_REMEDIATION: &str =
    "Check that CUDA is installed and nvidia-smi shows an available CUDA GPU";

#[derive(Clone, Debug)]
pub struct CudaContext {
    inner: Arc<CudarcContext>,
    determinism: bool,
    device_idx: u32,
    name: String,
    compute_capability: (i32, i32),
    total_mem_mib: u64,
    free_mem_mib_at_init: u64,
    distance_module: Arc<OnceLock<Arc<CudaModule>>>,
    topk_module: Arc<OnceLock<Arc<CudaModule>>>,
}

impl CudaContext {
    pub fn inner(&self) -> &Arc<CudarcContext> {
        &self.inner
    }

    pub fn determinism(&self) -> bool {
        self.determinism
    }

    pub fn device_idx(&self) -> u32 {
        self.device_idx
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn compute_capability(&self) -> (i32, i32) {
        self.compute_capability
    }

    pub fn total_mem_mib(&self) -> u64 {
        self.total_mem_mib
    }

    pub fn free_mem_mib_at_init(&self) -> u64 {
        self.free_mem_mib_at_init
    }

    /// Live free device VRAM in bytes via `cudaMemGetInfo` (in-process — never
    /// `nvidia-smi`). The returned value reflects *current* free memory and
    /// therefore accounts for every other resident process on the GPU (the TEI
    /// containers, dcgm-exporter). This is the truth source the VRAM budgeter
    /// consults before each large dispatch; it never assumes a fixed 32 GiB.
    ///
    /// Fail-loud: a driver error surfaces as
    /// [`ForgeError::DeviceUnavailable`] (`CALYX_FORGE_DEVICE_UNAVAILABLE`) —
    /// there is no zero-fill fallback, so callers can treat the unknown state
    /// as over-budget.
    pub fn free_device_vram_bytes(&self) -> Result<usize> {
        let (free_bytes, _total_bytes) =
            self.inner
                .mem_get_info()
                .map_err(|err| ForgeError::DeviceUnavailable {
                    device: device_label(self.device_idx),
                    detail: format!("CUDA cudaMemGetInfo (live free-VRAM query) failed: {err}"),
                    remediation: CUDA_REMEDIATION.to_string(),
                })?;
        Ok(free_bytes)
    }

    pub(crate) fn distance_module_cache(&self) -> &OnceLock<Arc<CudaModule>> {
        &self.distance_module
    }

    pub(crate) fn topk_module_cache(&self) -> &OnceLock<Arc<CudaModule>> {
        &self.topk_module
    }
}

pub fn init_cuda(device_idx: u32, determinism: bool) -> Result<CudaContext> {
    let device = device_label(device_idx);
    let inner = CudarcContext::new(device_idx as usize).map_err(|err| {
        device_unavailable(device_idx, format!("CUDA context init failed: {err}"))
    })?;

    let name = inner.name().map_err(|err| {
        device_unavailable(device_idx, format!("CUDA device name query failed: {err}"))
    })?;
    let compute_capability = inner.compute_capability().map_err(|err| {
        device_unavailable(
            device_idx,
            format!("CUDA compute capability query failed: {err}"),
        )
    })?;
    let (free_bytes, total_bytes) = inner
        .mem_get_info()
        .map_err(|err| device_unavailable(device_idx, format!("CUDA VRAM query failed: {err}")))?;
    let free_mem_mib = bytes_to_mib(free_bytes);
    ensure_min_free_vram(&device, free_mem_mib)?;

    Ok(CudaContext {
        inner,
        determinism,
        device_idx,
        name,
        compute_capability,
        total_mem_mib: bytes_to_mib(total_bytes),
        free_mem_mib_at_init: free_mem_mib,
        distance_module: Arc::new(OnceLock::new()),
        topk_module: Arc::new(OnceLock::new()),
    })
}

pub fn query_device_info(ctx: &CudaContext) -> DeviceInfo {
    DeviceInfo {
        kind: BackendKind::Cuda,
        name: ctx.name.clone(),
        avx512: false,
        vram_mib: Some(ctx.total_mem_mib),
    }
}

fn ensure_min_free_vram(device: &str, free_mem_mib: u64) -> Result<()> {
    if free_mem_mib < MIN_FREE_VRAM_MIB {
        return Err(ForgeError::DeviceUnavailable {
            device: device.to_string(),
            detail: format!(
                "less than 4 GiB VRAM free; free_vram_mib={free_mem_mib}; TEI containers may be using GPU memory"
            ),
            remediation: CUDA_REMEDIATION.to_string(),
        });
    }
    Ok(())
}

fn device_unavailable(device_idx: u32, detail: String) -> ForgeError {
    ForgeError::DeviceUnavailable {
        device: device_label(device_idx),
        detail,
        remediation: CUDA_REMEDIATION.to_string(),
    }
}

fn device_label(device_idx: u32) -> String {
    format!("cuda:{device_idx}")
}

fn bytes_to_mib(bytes: usize) -> u64 {
    (bytes as u64) / BYTES_PER_MIB
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backend, CudaBackend};
    use proptest::prelude::*;

    const BAD_DEVICE_IDX: u32 = 99;

    #[test]
    fn cuda_backend_new_reports_manual_gpu() -> Result<()> {
        let backend = CudaBackend::new()?;
        let info = backend.device_info();
        let ctx = backend.context();
        let (cc_major, cc_minor) = ctx.compute_capability();

        println!(
            "CUDA_DEVICE name={} vram_mib={} free_vram_mib_at_init={} compute_capability={}.{} determinism={}",
            info.name,
            info.vram_mib.unwrap_or_default(),
            ctx.free_mem_mib_at_init(),
            cc_major,
            cc_minor,
            ctx.determinism()
        );

        let lower_name = info.name.to_ascii_lowercase();
        assert!(lower_name.contains("5090") || lower_name.contains("rtx"));
        assert_eq!(info.kind, BackendKind::Cuda);
        assert!(!info.avx512);
        assert!(info.vram_mib.unwrap_or_default() >= 30_000);
        assert_eq!((cc_major, cc_minor), (12, 0));
        Ok(())
    }

    #[test]
    fn query_device_info_returns_cuda_kind() -> Result<()> {
        let ctx = init_cuda(0, true)?;
        let info = query_device_info(&ctx);

        println!(
            "CUDA_QUERY name={} vram_mib={} compute_capability={:?}",
            info.name,
            info.vram_mib.unwrap_or_default(),
            ctx.compute_capability()
        );

        assert_eq!(info.kind, BackendKind::Cuda);
        assert!(!info.name.is_empty());
        assert!(info.vram_mib.unwrap_or_default() > 0);
        assert!(ctx.determinism());
        Ok(())
    }

    #[test]
    fn bad_device_index_fails_closed() {
        let err = init_cuda(BAD_DEVICE_IDX, false).expect_err("bad CUDA index must fail closed");
        let display = err.to_string();

        println!("{display}");

        assert!(matches!(err, ForgeError::DeviceUnavailable { .. }));
        assert!(display.starts_with("CALYX_FORGE_DEVICE_UNAVAILABLE"));
        assert!(display.contains("Remediation:"));
        assert!(display.contains("/usr/local/cuda-13.3"));
    }

    #[test]
    fn cuda_context_can_initialize_twice() -> Result<()> {
        let first = init_cuda(0, false)?;
        let second = init_cuda(0, false)?;

        println!(
            "CUDA_DOUBLE_INIT first={} second={} vram_mib={}",
            first.name(),
            second.name(),
            second.total_mem_mib()
        );

        assert_eq!(first.device_idx(), 0);
        assert_eq!(second.device_idx(), 0);
        assert!(!second.name().is_empty());
        Ok(())
    }

    #[test]
    fn vram_soft_cap_rejects_low_free_memory() {
        let err = ensure_min_free_vram("cuda:0", MIN_FREE_VRAM_MIB - 1)
            .expect_err("low free VRAM must fail closed");
        let display = err.to_string();

        println!("{display}");

        assert!(display.starts_with("CALYX_FORGE_DEVICE_UNAVAILABLE"));
        assert!(display.contains("less than 4 GiB VRAM free"));
        assert!(display.contains("Remediation:"));
    }

    proptest! {
        #[test]
        fn device_unavailable_display_contains_code_and_remediation(
            device in ".{0,32}",
            detail in ".{0,96}",
            remediation in ".{0,96}"
        ) {
            let err = ForgeError::DeviceUnavailable { device, detail, remediation };
            let display = err.to_string();

            prop_assert!(display.starts_with("CALYX_FORGE_DEVICE_UNAVAILABLE"));
            prop_assert!(display.contains("Remediation:"));
        }
    }
}
