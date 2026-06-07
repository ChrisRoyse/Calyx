//! Forge math runtime skeleton for CPU, CUDA, and quantized kernels.

mod backend;
pub mod cpu;
#[cfg(feature = "cuda")]
pub mod cuda;
mod error;

pub use backend::{Backend, BackendKind, BestConfig, DeviceInfo, Result};
pub use cpu::CpuBackend;
#[cfg(feature = "cuda")]
pub use cuda::{CudaBackend, CudaContext, init_cuda, query_device_info};
pub use error::ForgeError;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-forge");
    }
}
