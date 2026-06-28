use super::*;

pub(super) fn cuda_ld_library_path(command: &str) -> Option<OsString> {
    let mut dirs = nvidia_library_dirs(command);
    if dirs.is_empty() {
        return env::var_os("LD_LIBRARY_PATH");
    }
    if let Some(existing) = env::var_os("LD_LIBRARY_PATH") {
        dirs.extend(env::split_paths(&existing));
    }
    env::join_paths(dirs).ok()
}

fn nvidia_library_dirs(command: &str) -> Vec<PathBuf> {
    let python = Path::new(command);
    let Some(venv_root) = python.parent().and_then(Path::parent) else {
        return Vec::new();
    };
    let lib_root = venv_root.join("lib");
    let Ok(python_dirs) = std::fs::read_dir(lib_root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for python_dir in python_dirs.flatten() {
        let site = python_dir.path().join("site-packages").join("nvidia");
        collect_nvidia_lib_dirs(&site, &mut out);
    }
    out
}

#[cfg(windows)]
pub(super) fn gpu_dll_path(command: &str) -> Option<OsString> {
    let mut dirs = windows_gpu_dll_dirs(command);
    if let Some(cuda_path) = env::var_os("CUDA_PATH") {
        let cuda_bin = PathBuf::from(cuda_path).join("bin");
        if cuda_bin.is_dir() {
            dirs.push(cuda_bin);
        }
    }
    if dirs.is_empty() {
        return env::var_os("PATH");
    }
    if let Some(existing) = env::var_os("PATH") {
        dirs.extend(env::split_paths(&existing));
    }
    env::join_paths(dirs).ok()
}

#[cfg(windows)]
fn windows_gpu_dll_dirs(command: &str) -> Vec<PathBuf> {
    let python = Path::new(command);
    let Some(venv_root) = python.parent().and_then(Path::parent) else {
        return Vec::new();
    };
    let site = venv_root.join("Lib").join("site-packages");
    let mut out = Vec::new();
    for candidate in [
        site.join("tensorrt_libs"),
        site.join("onnxruntime").join("capi"),
        site.join("nvidia").join("cu13").join("bin").join("x86_64"),
        site.join("nvidia").join("cudnn").join("bin"),
    ] {
        if candidate.is_dir() {
            out.push(candidate);
        }
    }
    out
}

fn collect_nvidia_lib_dirs(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(packages) = std::fs::read_dir(root) else {
        return;
    };
    for package in packages.flatten() {
        let candidate = package.path().join("lib");
        if candidate.is_dir() {
            out.push(candidate);
        }
    }
}
