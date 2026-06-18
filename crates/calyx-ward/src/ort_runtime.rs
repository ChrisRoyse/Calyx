use std::env;
use std::fs;
use std::path::PathBuf;

use crate::error::WardError;

const ORT_DYLIB_PATH: &str = "ORT_DYLIB_PATH";

pub(crate) fn ensure_dynamic_ort() -> Result<PathBuf, WardError> {
    let path = env::var_os(ORT_DYLIB_PATH).ok_or_else(|| WardError::Runtime {
        reason: format!(
            "{ORT_DYLIB_PATH} must point to a sm_120-capable ONNX Runtime dynamic library; \
             this build uses ort/load-dynamic and has no bundled ORT fallback"
        ),
    })?;
    let path = PathBuf::from(path);
    let metadata = fs::metadata(&path).map_err(|err| WardError::Runtime {
        reason: format!("stat {ORT_DYLIB_PATH}={} failed: {err}", path.display()),
    })?;
    if metadata.is_file() {
        Ok(path)
    } else {
        Err(WardError::Runtime {
            reason: format!("{ORT_DYLIB_PATH}={} is not a file", path.display()),
        })
    }
}
