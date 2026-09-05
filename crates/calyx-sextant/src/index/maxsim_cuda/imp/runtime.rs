#[cfg(sextant_cuvs)]
use super::*;

#[cfg(sextant_cuvs)]
use std::sync::{Arc, OnceLock};

#[cfg(sextant_cuvs)]
use cudarc::driver::{CudaContext, CudaFunction, CudaStream};
#[cfg(sextant_cuvs)]
use cudarc::nvrtc::Ptx;

#[cfg(sextant_cuvs)]
pub(super) struct Runtime {
    pub(super) stream: Arc<CudaStream>,
    pub(super) score_fn: CudaFunction,
    pub(super) topk_fn: CudaFunction,
    pub(super) merge_fn: CudaFunction,
    pub(super) batch_score_fn: CudaFunction,
    pub(super) batch_topk_fn: CudaFunction,
    pub(super) batch_merge_fn: CudaFunction,
}

#[cfg(sextant_cuvs)]
pub(super) fn get() -> Result<&'static Runtime> {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    if let Some(runtime) = RUNTIME.get() {
        return Ok(runtime);
    }
    let cuda = CudaContext::new(0).map_err(cuda_error("context init"))?;
    let stream = cuda.default_stream();
    let module = cuda
        .load_module(Ptx::from_binary(CUBIN.to_vec()))
        .map_err(cuda_error("CUBIN load"))?;
    let candidate = Runtime {
        score_fn: load(&module, "maxsim_score_rows", "score load")?,
        topk_fn: load(&module, "maxsim_chunk_topk", "topk load")?,
        merge_fn: load(&module, "maxsim_merge_topk", "merge load")?,
        batch_score_fn: load(&module, "maxsim_score_rows_batch", "batch score load")?,
        batch_topk_fn: load(&module, "maxsim_chunk_topk_batch", "batch topk load")?,
        batch_merge_fn: load(&module, "maxsim_merge_topk_batch", "batch merge load")?,
        stream,
    };
    let _ = RUNTIME.set(candidate);
    Ok(RUNTIME.get().expect("MaxSim CUDA runtime initialized"))
}
