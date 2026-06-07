use super::test_lock;
use crate::{Backend, CpuBackend, CudaBackend, ForgeError, Result};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;

fn rel_err(actual: f32, expected: f32) -> f32 {
    let denom = expected.abs().max(1.0);
    (actual - expected).abs() / denom
}

fn compare(actual: &[f32], expected: &[f32], tol: f32) -> f32 {
    let mut max_rel = 0.0f32;
    for (idx, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        let err = rel_err(*a, *e);
        max_rel = max_rel.max(err);
        assert!(
            err <= tol,
            "idx={idx} actual={a} expected={e} rel_err={err}"
        );
    }
    max_rel
}

fn unit(dim: usize, idx: usize) -> Vec<f32> {
    let mut value = vec![0.0; dim];
    value[idx] = 1.0;
    value
}

fn seeded_values(len: usize, seed: u32) -> Vec<f32> {
    let mut state = seed;
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let centered = ((state >> 8) % 2001) as f32 - 1000.0;
            centered / 1000.0
        })
        .collect()
}

fn make_non_zero_rows(values: &mut [f32], dim: usize) {
    if dim == 0 {
        return;
    }
    for row in 0..values.len() / dim {
        values[row * dim] += 1.0;
    }
}

#[test]
fn gpu_cosine_orthogonal_and_identical() -> Result<()> {
    let _guard = test_lock();
    let backend = CudaBackend::new()?;
    let query = unit(128, 0);
    let mut candidates = unit(128, 1);
    candidates.extend_from_slice(&unit(128, 0));
    let mut out = vec![99.0; 2];

    backend.cosine(&query, &candidates, 128, &mut out)?;

    println!(
        "CUDA_COSINE orthogonal={:.8} identical={:.8}",
        out[0], out[1]
    );
    assert!(out[0].abs() <= 1e-5);
    assert!((out[1] - 1.0).abs() <= 1e-5);
    Ok(())
}

#[test]
fn gpu_l2_and_dot_known_values() -> Result<()> {
    let _guard = test_lock();
    let backend = CudaBackend::new()?;
    let mut l2 = vec![0.0];
    backend.l2(&[0.0, 0.0], &[3.0, 4.0], 2, &mut l2)?;
    let mut dot = vec![0.0];
    backend.dot(&[2.0, 3.0], &[5.0, 7.0], 2, &mut dot)?;

    println!("CUDA_L2_PYTHAGOREAN {:.6}", l2[0]);
    println!("CUDA_DOT_KNOWN {:.6}", dot[0]);
    assert!((l2[0] - 25.0).abs() <= 1e-4);
    assert!((dot[0] - 31.0).abs() <= 1e-5);
    Ok(())
}

#[test]
fn gpu_distance_edges_and_zero_norm_fail_closed() -> Result<()> {
    let _guard = test_lock();
    let backend = CudaBackend::new()?;
    let mut single = vec![0.0];
    backend.dot(&[3.0], &[4.0], 1, &mut single)?;
    println!("CUDA_DISTANCE_EDGE n_cands=1 dim=1 dot={:.6}", single[0]);
    assert!((single[0] - 12.0).abs() <= 1e-5);

    let dim = 1536;
    let query = vec![1.0; dim];
    let candidates = vec![2.0; dim];
    backend.cosine(&query, &candidates, dim, &mut single)?;
    println!("CUDA_DISTANCE_EDGE dim=1536 cosine={:.8}", single[0]);
    assert!((single[0] - 1.0).abs() <= 1e-5);

    let err = backend
        .cosine(&[1.0, 0.0], &[0.0, 0.0], 2, &mut single)
        .expect_err("zero-norm candidate must fail closed");
    println!("CUDA_DISTANCE_ZERO_NORM {err}");
    assert!(matches!(err, ForgeError::NumericalInvariant { .. }));

    let err = crate::init_cuda(99, false).expect_err("bad CUDA device must fail closed");
    println!("CUDA_DISTANCE_BAD_CONTEXT {err}");
    assert!(matches!(err, ForgeError::DeviceUnavailable { .. }));
    Ok(())
}

#[test]
fn gpu_cosine_seed42_matches_cpu_for_100_candidates() -> Result<()> {
    let _guard = test_lock();
    let dim = 128;
    let n_cands = 100;
    let mut query = seeded_values(dim, 42);
    query[0] += 1.0;
    let mut candidates = seeded_values(dim * n_cands, 43);
    make_non_zero_rows(&mut candidates, dim);
    let mut cpu_out = vec![0.0; n_cands];
    let mut gpu_out = vec![0.0; n_cands];

    CpuBackend::new().cosine(&query, &candidates, dim, &mut cpu_out)?;
    CudaBackend::new()?.cosine(&query, &candidates, dim, &mut gpu_out)?;

    let max_rel = compare(&gpu_out, &cpu_out, 1e-3);
    println!(
        "CUDA_COSINE_PROPTEST seed=42 dim={dim} candidates={n_cands} max_rel_err={max_rel:.8}"
    );
    Ok(())
}

fn cosine_case() -> impl Strategy<Value = (Vec<f32>, Vec<f32>, usize)> {
    (1usize..=16).prop_flat_map(|n_cands| {
        (
            proptest::collection::vec(-1.0f32..1.0, 128),
            proptest::collection::vec(-1.0f32..1.0, 128 * n_cands),
            Just(n_cands),
        )
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    #[test]
    fn gpu_cosine_matches_cpu_proptest((mut query, mut candidates, n_cands) in cosine_case()) {
        let _guard = test_lock();
        query[0] += 1.0;
        make_non_zero_rows(&mut candidates, 128);
        let mut cpu_out = vec![0.0; n_cands];
        let mut gpu_out = vec![0.0; n_cands];
        CpuBackend::new()
            .cosine(&query, &candidates, 128, &mut cpu_out)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        CudaBackend::new()
            .map_err(|err| TestCaseError::fail(err.to_string()))?
            .cosine(&query, &candidates, 128, &mut gpu_out)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        for (actual, expected) in gpu_out.iter().zip(cpu_out.iter()) {
            prop_assert!(rel_err(*actual, *expected) <= 1e-3);
        }
    }
}
