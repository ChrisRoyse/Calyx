use std::env;
use std::fs;
use std::path::PathBuf;

use calyx_core::{CalyxError, Result};

use super::OnnxProviderPolicy;

const ORT_DYLIB_PATH: &str = "ORT_DYLIB_PATH";
const CALYX_ORT_CAPI: &str = "CALYX_ORT_CAPI";

pub(super) fn ensure_dynamic_ort(provider_policy: OnnxProviderPolicy) -> Result<PathBuf> {
    let path = resolve_ort_dylib_path()?;
    ensure_file(&path)?;
    #[cfg(not(windows))]
    let _ = provider_policy;
    #[cfg(windows)]
    if provider_policy == OnnxProviderPolicy::CudaFailLoud {
        super::windows_cuda_dlls::prepare_cuda_provider_dll_search(&path)?;
    }
    Ok(path)
}

fn resolve_ort_dylib_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os(ORT_DYLIB_PATH) {
        return Ok(PathBuf::from(path));
    }
    if let Some(capi) = env::var_os(CALYX_ORT_CAPI) {
        let path = PathBuf::from(capi).join("onnxruntime.dll");
        ensure_file(&path).map_err(|error| {
            CalyxError::lens_unreachable(format!(
                "{CALYX_ORT_CAPI} is set but {} is not a usable ORT dynamic library: {}",
                path.display(),
                error.message
            ))
        })?;
        unsafe {
            env::set_var(ORT_DYLIB_PATH, &path);
        }
        return Ok(path);
    }
    Err(CalyxError::lens_unreachable(format!(
        "{ORT_DYLIB_PATH} must point to a sm_120-capable ONNX Runtime dynamic library; \
         this build uses ort/load-dynamic and has no bundled ORT fallback. On Windows, \
         set {ORT_DYLIB_PATH} directly or set {CALYX_ORT_CAPI} to the ONNX Runtime capi \
         directory before starting GPU resident/search/ingest commands"
    )))
}

fn ensure_file(path: &PathBuf) -> Result<()> {
    let metadata = fs::metadata(path).map_err(|err| {
        CalyxError::lens_unreachable(format!(
            "stat {ORT_DYLIB_PATH}={} failed: {err}",
            path.display()
        ))
    })?;
    if metadata.is_file() {
        Ok(())
    } else {
        Err(CalyxError::lens_unreachable(format!(
            "{ORT_DYLIB_PATH}={} is not a file",
            path.display()
        )))
    }
}
