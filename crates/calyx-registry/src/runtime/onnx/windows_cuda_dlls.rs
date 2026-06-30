#![cfg(windows)]

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use calyx_core::{CalyxError, Result};
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::System::LibraryLoader::{
    AddDllDirectory, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, SetDefaultDllDirectories,
};

const CALYX_NVIDIA_DLL_DIRS: &str = "CALYX_NVIDIA_DLL_DIRS";
const CALYX_ORT_CAPI: &str = "CALYX_ORT_CAPI";
const CUDA_PROVIDER_DLL: &str = "onnxruntime_providers_cuda.dll";
const SHARED_PROVIDER_DLL: &str = "onnxruntime_providers_shared.dll";

static DLL_STATE: OnceLock<Mutex<DllSearchState>> = OnceLock::new();

#[derive(Default)]
struct DllSearchState {
    default_dirs_set: bool,
    added_dirs: BTreeSet<PathBuf>,
}

pub(super) fn prepare_cuda_provider_dll_search(ort_dylib: &Path) -> Result<()> {
    let plan = CudaDllPlan::new(ort_dylib)?;
    add_process_dll_directories(&plan.search_dirs)?;
    prepend_process_path(&plan.search_dirs)?;
    Ok(())
}

#[derive(Debug)]
struct CudaDllPlan {
    search_dirs: Vec<PathBuf>,
}

impl CudaDllPlan {
    fn new(ort_dylib: &Path) -> Result<Self> {
        let ort_dir = canonical_parent(ort_dylib)?;
        ensure_required_file(&ort_dir.join(CUDA_PROVIDER_DLL), CUDA_PROVIDER_DLL)?;
        ensure_required_file(&ort_dir.join(SHARED_PROVIDER_DLL), SHARED_PROVIDER_DLL)?;
        let mut dirs = vec![ort_dir.clone()];
        append_explicit_dirs(&mut dirs)?;
        append_inferred_dirs(&mut dirs, &ort_dir);
        let dirs = dedupe_existing_dirs(dirs)?;
        require_dll(&dirs, "cudnn64_9.dll")?;
        require_dll_prefix(&dirs, "cudart64_", ".dll")?;
        require_dll_prefix(&dirs, "cublas64_", ".dll")?;
        require_dll_prefix(&dirs, "cublasLt64_", ".dll")?;
        Ok(Self { search_dirs: dirs })
    }
}

fn canonical_parent(path: &Path) -> Result<PathBuf> {
    let path = fs::canonicalize(path).map_err(|err| {
        CalyxError::lens_unreachable(format!(
            "canonicalize ORT_DYLIB_PATH={} failed before CUDA provider bootstrap: {err}",
            path.display()
        ))
    })?;
    path.parent().map(Path::to_path_buf).ok_or_else(|| {
        CalyxError::lens_unreachable(format!(
            "ORT_DYLIB_PATH={} has no parent directory for CUDA provider bootstrap",
            path.display()
        ))
    })
}

fn ensure_required_file(path: &Path, name: &str) -> Result<()> {
    if path.is_file() {
        Ok(())
    } else {
        Err(CalyxError::lens_unreachable(format!(
            "CUDA ONNX provider bootstrap requires {name} at {}",
            path.display()
        )))
    }
}

fn append_explicit_dirs(dirs: &mut Vec<PathBuf>) -> Result<()> {
    if let Some(capi) = env::var_os(CALYX_ORT_CAPI) {
        dirs.push(PathBuf::from(capi));
    }
    if let Some(raw) = env::var_os(CALYX_NVIDIA_DLL_DIRS) {
        for dir in env::split_paths(&raw) {
            if !dir.is_dir() {
                return Err(CalyxError::lens_unreachable(format!(
                    "{CALYX_NVIDIA_DLL_DIRS} contains non-directory {}",
                    dir.display()
                )));
            }
            dirs.push(dir);
        }
    }
    Ok(())
}

fn append_inferred_dirs(dirs: &mut Vec<PathBuf>, ort_dir: &Path) {
    let Some(site) = ort_dir
        .parent()
        .and_then(Path::parent)
        .filter(|site| site.ends_with("site-packages"))
    else {
        return;
    };
    for dir in [
        site.join("nvidia").join("cu13").join("bin"),
        site.join("nvidia").join("cu13").join("bin").join("x86_64"),
        site.join("nvidia").join("cu13").join("nvvm").join("bin"),
        site.join("nvidia").join("cudnn").join("bin"),
    ] {
        if dir.is_dir() {
            dirs.push(dir);
        }
    }
}

fn dedupe_existing_dirs(dirs: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for dir in dirs {
        let canonical = fs::canonicalize(&dir).map_err(|err| {
            CalyxError::lens_unreachable(format!(
                "canonicalize CUDA provider DLL directory {} failed: {err}",
                dir.display()
            ))
        })?;
        if !canonical.is_dir() {
            return Err(CalyxError::lens_unreachable(format!(
                "CUDA provider DLL path {} is not a directory",
                canonical.display()
            )));
        }
        if seen.insert(canonical.clone()) {
            out.push(canonical);
        }
    }
    Ok(out)
}

fn require_dll(dirs: &[PathBuf], name: &str) -> Result<()> {
    if dirs.iter().any(|dir| dir.join(name).is_file()) {
        Ok(())
    } else {
        Err(CalyxError::lens_unreachable(format!(
            "CUDA ONNX provider bootstrap could not find {name}; searched {}",
            format_dirs(dirs)
        )))
    }
}

fn require_dll_prefix(dirs: &[PathBuf], prefix: &str, suffix: &str) -> Result<()> {
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(prefix) && name.ends_with(suffix) && entry.path().is_file() {
                return Ok(());
            }
        }
    }
    Err(CalyxError::lens_unreachable(format!(
        "CUDA ONNX provider bootstrap could not find {prefix}*{suffix}; searched {}",
        format_dirs(dirs)
    )))
}

fn add_process_dll_directories(dirs: &[PathBuf]) -> Result<()> {
    let state = DLL_STATE.get_or_init(|| Mutex::new(DllSearchState::default()));
    let mut state = state.lock().map_err(|_| {
        CalyxError::lens_unreachable("CUDA provider DLL search state mutex was poisoned")
    })?;
    if !state.default_dirs_set {
        let ok = unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS) };
        if ok == 0 {
            return Err(last_error("SetDefaultDllDirectories"));
        }
        state.default_dirs_set = true;
    }
    for dir in dirs {
        if !state.added_dirs.insert(dir.clone()) {
            continue;
        }
        let wide = wide_path(dir);
        let cookie = unsafe { AddDllDirectory(wide.as_ptr()) };
        if cookie.is_null() {
            return Err(last_error(format!("AddDllDirectory {}", dir.display())));
        }
    }
    Ok(())
}

fn prepend_process_path(dirs: &[PathBuf]) -> Result<()> {
    let mut path_dirs = dirs.to_vec();
    if let Some(existing) = env::var_os("PATH") {
        let existing_dirs: Vec<_> = env::split_paths(&existing)
            .filter(|dir| !path_dirs.contains(dir))
            .collect();
        path_dirs.extend(existing_dirs);
    }
    let joined = env::join_paths(path_dirs).map_err(|err| {
        CalyxError::lens_unreachable(format!("join CUDA provider PATH entries failed: {err}"))
    })?;
    unsafe {
        env::set_var("PATH", joined);
    }
    Ok(())
}

fn wide_path(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    path.as_os_str().encode_wide().chain([0]).collect()
}

fn last_error(context: impl Into<String>) -> CalyxError {
    CalyxError::lens_unreachable(format!(
        "{} failed with Windows error {}",
        context.into(),
        unsafe { GetLastError() }
    ))
}

fn format_dirs(dirs: &[PathBuf]) -> String {
    dirs.iter()
        .map(|dir| dir.display().to_string())
        .collect::<Vec<_>>()
        .join(";")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_infers_cuda_and_cudnn_dirs_from_ort_site_packages() {
        let root = test_layout("infer");
        let plan = CudaDllPlan::new(&root.ort).unwrap();

        assert!(
            plan.search_dirs
                .contains(&fs::canonicalize(&root.capi).unwrap())
        );
        assert!(
            plan.search_dirs
                .contains(&fs::canonicalize(root.site.join("nvidia/cu13/bin/x86_64")).unwrap())
        );
        assert!(
            plan.search_dirs
                .contains(&fs::canonicalize(root.site.join("nvidia/cudnn/bin")).unwrap())
        );
    }

    #[test]
    fn plan_fails_closed_when_cudnn_is_missing() {
        let root = test_layout("missing-cudnn");
        fs::remove_file(root.site.join("nvidia/cudnn/bin/cudnn64_9.dll")).unwrap();

        let error = CudaDllPlan::new(&root.ort).unwrap_err();

        assert_eq!(error.code, "CALYX_LENS_UNREACHABLE");
        assert!(error.message.contains("cudnn64_9.dll"));
    }

    #[test]
    fn plan_fails_closed_when_cuda_runtime_is_missing() {
        let root = test_layout("missing-cudart");
        fs::remove_file(root.site.join("nvidia/cu13/bin/x86_64/cudart64_13.dll")).unwrap();

        let error = CudaDllPlan::new(&root.ort).unwrap_err();

        assert_eq!(error.code, "CALYX_LENS_UNREACHABLE");
        assert!(error.message.contains("cudart64_"));
    }

    struct TestLayout {
        site: PathBuf,
        capi: PathBuf,
        ort: PathBuf,
    }

    fn test_layout(name: &str) -> TestLayout {
        let root = env::temp_dir().join(format!(
            "calyx-windows-cuda-dlls-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let site = root.join("Lib").join("site-packages");
        let capi = site.join("onnxruntime").join("capi");
        let cuda = site.join("nvidia").join("cu13").join("bin").join("x86_64");
        let cudnn = site.join("nvidia").join("cudnn").join("bin");
        fs::create_dir_all(&capi).unwrap();
        fs::create_dir_all(&cuda).unwrap();
        fs::create_dir_all(&cudnn).unwrap();
        touch(capi.join("onnxruntime.dll"));
        touch(capi.join(CUDA_PROVIDER_DLL));
        touch(capi.join(SHARED_PROVIDER_DLL));
        touch(cuda.join("cudart64_13.dll"));
        touch(cuda.join("cublas64_13.dll"));
        touch(cuda.join("cublasLt64_13.dll"));
        touch(cudnn.join("cudnn64_9.dll"));
        TestLayout {
            site,
            ort: capi.join("onnxruntime.dll"),
            capi,
        }
    }

    fn touch(path: PathBuf) {
        fs::write(path, b"test").unwrap();
    }
}
