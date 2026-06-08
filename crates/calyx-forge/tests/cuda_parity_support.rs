#[cfg(feature = "cuda")]
use std::{fs, path::PathBuf};

#[cfg(feature = "cuda")]
use serde::Deserialize;

#[cfg(feature = "cuda")]
const GOLDEN_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden");
pub const PARITY_TOL: f32 = 1e-3;
pub const PARITY_ABS_TOL: f32 = 1e-6;

#[cfg(feature = "cuda")]
#[derive(Debug, Deserialize)]
pub struct GoldenManifest {
    pub n_vecs: usize,
    pub dim: usize,
    pub gemm_m: usize,
    pub gemm_k: usize,
    pub gemm_n: usize,
    pub topk: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct ParityReport {
    pub worst_rel_idx: usize,
    pub max_rel_err: f32,
    pub worst_abs_idx: usize,
    pub max_abs_err: f32,
}

#[cfg(feature = "cuda")]
pub fn load_golden_f32(name: &str) -> Vec<f32> {
    let path = PathBuf::from(GOLDEN_DIR).join(format!("{name}.bin"));
    let bytes = fs::read(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    if !bytes.len().is_multiple_of(4) {
        panic!("{name}: unexpected EOF in f32 little-endian bytes");
    }
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

#[cfg(feature = "cuda")]
pub fn load_manifest() -> GoldenManifest {
    let path = PathBuf::from(GOLDEN_DIR).join("golden_manifest.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

#[cfg(feature = "cuda")]
pub fn l2_norm(values: &[f32]) -> f32 {
    values.iter().map(|value| value * value).sum::<f32>().sqrt()
}

#[cfg(feature = "cuda")]
pub fn write_cuda_fsv_readback(file_name: &str, value: &serde_json::Value) {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    let path = PathBuf::from(root).join(file_name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|err| panic!("{}: {err}", parent.display()));
    }
    let bytes = serde_json::to_vec_pretty(value).expect("serialize cuda fsv readback");
    fs::write(&path, bytes).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    println!("CUDA_FSV_READBACK name={file_name} path={}", path.display());
}

pub fn max_rel_err(a: &[f32], b: &[f32]) -> f32 {
    parity_report(a, b).max_rel_err
}

pub fn assert_parity(cpu: &[f32], gpu: &[f32], op: &str, tol: f32) {
    assert_eq!(
        cpu.len(),
        gpu.len(),
        "PARITY FAIL op={op} len cpu={} gpu={}",
        cpu.len(),
        gpu.len()
    );
    let report = parity_report(cpu, gpu);
    println!(
        "PARITY op={op} rel_err={:.8e} rel_idx={} abs_err={:.8e} abs_idx={}",
        report.max_rel_err, report.worst_rel_idx, report.max_abs_err, report.worst_abs_idx
    );
    if report.max_rel_err > tol && report.max_abs_err > PARITY_ABS_TOL {
        panic!(
            "PARITY FAIL op={op} max_rel_err={:.2e} > tol={tol:.2e} and max_abs_err={:.2e} > abs_tol={PARITY_ABS_TOL:.2e}; rel_idx={} cpu={} gpu={}; abs_idx={} cpu={} gpu={}",
            report.max_rel_err,
            report.max_abs_err,
            report.worst_rel_idx,
            cpu[report.worst_rel_idx],
            gpu[report.worst_rel_idx],
            report.worst_abs_idx,
            cpu[report.worst_abs_idx],
            gpu[report.worst_abs_idx],
        );
    }
}

pub fn parity_report(a: &[f32], b: &[f32]) -> ParityReport {
    let mut report = ParityReport {
        worst_rel_idx: 0,
        max_rel_err: 0.0,
        worst_abs_idx: 0,
        max_abs_err: 0.0,
    };
    for (index, (left, right)) in a.iter().zip(b.iter()).enumerate() {
        let abs_err = (left - right).abs();
        let rel_err = abs_err / (right.abs() + 1e-8);
        if rel_err > report.max_rel_err {
            report.max_rel_err = rel_err;
            report.worst_rel_idx = index;
        }
        if abs_err > report.max_abs_err {
            report.max_abs_err = abs_err;
            report.worst_abs_idx = index;
        }
    }
    report
}
