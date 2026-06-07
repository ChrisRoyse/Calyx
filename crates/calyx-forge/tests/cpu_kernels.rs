use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use calyx_forge::{Backend, CpuBackend};
use proptest::prelude::*;
use serde::Deserialize;

const GOLDEN_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden");
const GEMM_TOL: f32 = 1e-4;
const COSINE_TOL: f32 = 1e-5;
static PANIC_HOOK_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Deserialize)]
struct GoldenManifest {
    seed: String,
    seed_version: u32,
    numpy_version: String,
    n_vecs: usize,
    dim: usize,
    gemm_m: usize,
    gemm_k: usize,
    gemm_n: usize,
    topk: usize,
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(GOLDEN_DIR).join(format!("{name}.bin"))
}

fn load_golden_f32(name: &str) -> Vec<f32> {
    read_golden_f32(name).unwrap_or_else(|err| panic!("{name}: {err}"))
}

fn read_golden_f32(name: &str) -> Result<Vec<f32>, String> {
    let path = golden_path(name);
    let bytes = fs::read(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    decode_golden_f32(name, &bytes)
}

fn decode_golden_f32(name: &str, bytes: &[u8]) -> Result<Vec<f32>, String> {
    if !bytes.len().is_multiple_of(4) {
        return Err(format!("{name}: unexpected EOF in f32 little-endian bytes"));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

fn load_manifest() -> GoldenManifest {
    let path = PathBuf::from(GOLDEN_DIR).join("golden_manifest.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn max_abs_diff(actual: &[f32], expected: &[f32]) -> (usize, f32) {
    actual
        .iter()
        .zip(expected.iter())
        .enumerate()
        .map(|(index, (actual, expected))| (index, (actual - expected).abs()))
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .unwrap_or((0, 0.0))
}

fn assert_with_worst(op: &str, actual: &[f32], expected: &[f32], tolerance: f32) {
    let manifest = load_manifest();
    assert_eq!(actual.len(), expected.len());
    let (index, worst) = max_abs_diff(actual, expected);
    println!("GOLDEN_{op} worst={worst:.8} index={index}");
    if worst > tolerance {
        panic!(
            "CALYX_FORGE_GOLDEN_MISMATCH op={op} seed={} numpy_version={} worst={} index={} expected={} actual={}",
            manifest.seed, manifest.numpy_version, worst, index, expected[index], actual[index]
        );
    }
}

#[test]
fn golden_manifest_deserializes() {
    let manifest = load_manifest();
    assert_eq!(manifest.seed, "0xCALYX12");
    assert_eq!(manifest.seed_version, 1);
    assert_eq!(manifest.n_vecs, 64);
    assert_eq!(manifest.dim, 128);
}

#[test]
fn golden_gemm_a_length_exact() {
    assert_eq!(load_golden_f32("gemm_A").len(), 128 * 64);
}

#[test]
fn golden_gemm_matches_numpy() {
    let manifest = load_manifest();
    let cpu = CpuBackend::new();
    let a = load_golden_f32("gemm_A");
    let b = load_golden_f32("gemm_B");
    let expected = load_golden_f32("gemm_C_ref");
    let mut actual = vec![0.0; manifest.gemm_m * manifest.gemm_n];

    cpu.gemm(
        &a,
        &b,
        manifest.gemm_m,
        manifest.gemm_k,
        manifest.gemm_n,
        &mut actual,
    )
    .expect("golden gemm dispatch");

    assert_with_worst("GEMM", &actual, &expected, GEMM_TOL);
}

#[test]
fn golden_cosine_matches_numpy() {
    let manifest = load_manifest();
    let cpu = CpuBackend::new();
    let vectors = load_golden_f32("vectors_128d");
    let expected = load_golden_f32("cosine_ref");
    let query = &vectors[..manifest.dim];
    let candidates = &vectors[manifest.dim..];
    let mut actual = vec![0.0; manifest.n_vecs - 1];

    cpu.cosine(query, candidates, manifest.dim, &mut actual)
        .expect("golden cosine dispatch");

    assert_with_worst("COSINE", &actual, &expected, COSINE_TOL);
}

#[test]
fn golden_topk_matches_numpy() {
    let manifest = load_manifest();
    let cpu = CpuBackend::new();
    let scores = load_golden_f32("cosine_ref");
    let expected = load_golden_f32("topk_ref");
    let actual = cpu
        .topk(&scores, manifest.topk)
        .expect("golden topk dispatch");
    let actual_indices: Vec<usize> = actual.iter().map(|(index, _)| *index).collect();
    let expected_indices: Vec<usize> = expected.iter().map(|index| *index as usize).collect();
    println!("GOLDEN_TOPK actual={actual_indices:?} expected={expected_indices:?}");
    if actual_indices != expected_indices {
        panic!(
            "CALYX_FORGE_GOLDEN_MISMATCH op=TOPK seed={} numpy_version={} expected={expected_indices:?} actual={actual_indices:?}",
            manifest.seed, manifest.numpy_version
        );
    }
}

#[test]
fn golden_missing_file_names_path() {
    let _guard = PANIC_HOOK_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let panic = std::panic::catch_unwind(|| load_golden_f32("missing_fixture"));
    std::panic::set_hook(hook);
    let panic = panic.expect_err("missing fixture must panic with filename");
    let message = panic_message(panic);
    assert!(message.contains("missing_fixture"), "{message}");
}

#[test]
fn golden_truncated_bytes_report_unexpected_eof() {
    let err = decode_golden_f32("truncated_fixture", &[0, 1, 2])
        .expect_err("truncated f32 bytes must fail");
    assert!(err.contains("unexpected EOF"), "{err}");
}

#[test]
fn golden_cosine_self_sanity_bound() {
    let manifest = load_manifest();
    let cpu = CpuBackend::new();
    let vectors = load_golden_f32("vectors_128d");
    let query = &vectors[..manifest.dim];
    let mut out = vec![0.0];
    cpu.cosine(query, query, manifest.dim, &mut out)
        .expect("self cosine dispatch");
    println!("GOLDEN_COSINE_SELF {:.8}", out[0]);
    assert!(out[0] >= 0.9999);
}

proptest! {
    #[test]
    fn golden_values_are_finite(name in prop::sample::select(vec![
        "vectors_128d",
        "gemm_A",
        "gemm_B",
        "gemm_C_ref",
        "cosine_ref",
        "topk_ref",
    ])) {
        let values = load_golden_f32(name);
        prop_assert!(values.iter().all(|value| value.is_finite()));
    }
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = panic.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = panic.downcast_ref::<&'static str>() {
        return (*message).to_string();
    }
    "<non-string panic>".to_string()
}
