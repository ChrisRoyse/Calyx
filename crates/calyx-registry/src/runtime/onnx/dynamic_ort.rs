use std::env;
use std::fs;
use std::path::PathBuf;

use calyx_core::{CalyxError, Result};

const ORT_DYLIB_PATH: &str = "ORT_DYLIB_PATH";

pub(super) fn ensure_dynamic_ort() -> Result<PathBuf> {
    let path = env::var_os(ORT_DYLIB_PATH).ok_or_else(|| {
        CalyxError::lens_unreachable(format!(
            "{ORT_DYLIB_PATH} must point to a sm_120-capable ONNX Runtime dynamic library; \
             this build uses ort/load-dynamic and has no bundled ORT fallback"
        ))
    })?;
    let path = PathBuf::from(path);
    let metadata = fs::metadata(&path).map_err(|err| {
        CalyxError::lens_unreachable(format!(
            "stat {ORT_DYLIB_PATH}={} failed: {err}",
            path.display()
        ))
    })?;
    if metadata.is_file() {
        Ok(path)
    } else {
        Err(CalyxError::lens_unreachable(format!(
            "{ORT_DYLIB_PATH}={} is not a file",
            path.display()
        )))
    }
}
