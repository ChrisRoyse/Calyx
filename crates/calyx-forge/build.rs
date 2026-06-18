use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CUDA_PATH_DEFAULT: &str = "/usr/local/cuda-13.3";
const CUDA_ARCH: &str = "sm_120";

struct Kernel {
    name: &'static str,
    src: &'static str,
    ptx_env: &'static str,
    cubin_env: &'static str,
}

const KERNELS: &[Kernel] = &[
    Kernel {
        name: "distance",
        src: "src/cuda/kernels/distance.cu",
        ptx_env: "FORGE_DISTANCE_PTX_PATH",
        cubin_env: "FORGE_DISTANCE_CUBIN_PATH",
    },
    Kernel {
        name: "topk",
        src: "src/cuda/kernels/topk.cu",
        ptx_env: "FORGE_TOPK_PTX_PATH",
        cubin_env: "FORGE_TOPK_CUBIN_PATH",
    },
    Kernel {
        name: "mxfp4_gemm",
        src: "src/cuda/kernels/mxfp4_gemm.cu",
        ptx_env: "FORGE_MXFP4_GEMM_PTX_PATH",
        cubin_env: "FORGE_MXFP4_GEMM_CUBIN_PATH",
    },
];

fn main() {
    if !cuda_feature_enabled() {
        println!("cargo:warning=cuda feature not enabled, skipping kernel compilation");
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let kernel_out_dir = out_dir.join("forge-cuda-kernels");
    std::fs::create_dir_all(&kernel_out_dir).expect("create CUDA kernel OUT_DIR");

    println!("cargo:rerun-if-env-changed=CUDA_PATH");
    let nvcc = locate_nvcc();
    warn_nvcc_version(&nvcc);

    for kernel in KERNELS {
        let src = manifest_dir.join(kernel.src);
        println!("cargo:rerun-if-changed={}", kernel.src);
        assert_source_exists(&src);

        let ptx = kernel_out_dir.join(format!("{}.ptx", kernel.name));
        let cubin = kernel_out_dir.join(format!("{}.cubin", kernel.name));

        compile_ptx(&nvcc, &src, &ptx);
        compile_cubin(&nvcc, &src, &cubin);

        println!("cargo:rustc-env={}={}", kernel.ptx_env, ptx.display());
        println!("cargo:rustc-env={}={}", kernel.cubin_env, cubin.display());
    }
}

fn cuda_feature_enabled() -> bool {
    cfg!(feature = "cuda") || env::var_os("CARGO_FEATURE_CUDA").is_some()
}

fn locate_nvcc() -> PathBuf {
    let cuda_path = env::var_os("CUDA_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(CUDA_PATH_DEFAULT));
    let nvcc = cuda_path.join("bin").join("nvcc");

    if !nvcc.is_file() {
        panic!(
            "nvcc not found at {}; set CUDA_PATH to CUDA 13.3 root",
            nvcc.display()
        );
    }
    nvcc
}

fn warn_nvcc_version(nvcc: &Path) {
    let output = Command::new(nvcc)
        .arg("--version")
        .output()
        .unwrap_or_else(|err| panic!("failed to run {} --version: {err}", nvcc.display()));
    assert_success(nvcc, &["--version"], output);

    let stdout = String::from_utf8_lossy(
        &Command::new(nvcc)
            .arg("--version")
            .output()
            .expect("rerun nvcc --version")
            .stdout,
    )
    .to_string();
    let summary = stdout
        .lines()
        .find(|line| line.contains("release") || line.contains("V13.3"))
        .unwrap_or_else(|| stdout.lines().next().unwrap_or("unknown nvcc version"));
    println!("cargo:warning=nvcc detected: {summary}");
}

fn compile_ptx(nvcc: &Path, src: &Path, out: &Path) {
    let args = deterministic_args(src, out, "--ptx");
    let output = Command::new(nvcc)
        .args(&args)
        .output()
        .unwrap_or_else(|err| panic!("failed to run {} for PTX: {err}", nvcc.display()));
    assert_success(nvcc, &args, output);
}

fn compile_cubin(nvcc: &Path, src: &Path, out: &Path) {
    let args = deterministic_args(src, out, "-cubin");
    let output = Command::new(nvcc)
        .args(&args)
        .output()
        .unwrap_or_else(|err| panic!("failed to run {} for cubin: {err}", nvcc.display()));
    assert_success(nvcc, &args, output);
}

fn deterministic_args(src: &Path, out: &Path, output_kind: &str) -> Vec<String> {
    vec![
        format!("-arch={CUDA_ARCH}"),
        "-O3".to_string(),
        "--ftz=false".to_string(),
        "--prec-div=true".to_string(),
        "--prec-sqrt=true".to_string(),
        "--fmad=false".to_string(),
        "-Xcompiler".to_string(),
        "-fPIC".to_string(),
        output_kind.to_string(),
        "-o".to_string(),
        out.display().to_string(),
        src.display().to_string(),
    ]
}

fn assert_source_exists(src: &Path) {
    if !src.is_file() {
        panic!("CUDA kernel source not found: {}", src.display());
    }
}

fn assert_success(nvcc: &Path, args: &[impl AsRef<str>], output: Output) {
    if output.status.success() {
        return;
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let joined_args = args
        .iter()
        .map(|arg| arg.as_ref())
        .collect::<Vec<_>>()
        .join(" ");
    panic!(
        "nvcc command failed: {} {}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        nvcc.display(),
        joined_args,
        output.status,
        stdout,
        stderr
    );
}
