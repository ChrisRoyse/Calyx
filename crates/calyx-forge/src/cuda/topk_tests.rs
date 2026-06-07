use super::test_lock;
use crate::{Backend, CpuBackend, CudaBackend, ForgeError, Result};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;

fn seeded_scores(len: usize, seed: u32) -> Vec<f32> {
    let mut state = seed;
    (0..len)
        .map(|idx| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (((state >> 8) % 2001) as f32 - 1000.0) / 1000.0 + (idx % 7) as f32 * 0.00001
        })
        .collect()
}

fn assert_sorted(result: &[(usize, f32)]) {
    for pair in result.windows(2) {
        let (left_index, left_score) = pair[0];
        let (right_index, right_score) = pair[1];
        assert!(
            left_score > right_score
                || ((left_score - right_score).abs() <= 1e-5 && left_index < right_index),
            "left=({left_index},{left_score}) right=({right_index},{right_score})"
        );
    }
}

#[test]
fn topk_tie_break_gpu_matches_cpu() -> Result<()> {
    let _guard = test_lock();
    let scores = [0.1, 0.9, 0.5, 0.9];
    let result = CudaBackend::new()?.topk(&scores, 2)?;
    let cpu = CpuBackend::new().topk(&scores, 2)?;
    println!("CUDA_TOPK_TIE {:?}", result);
    assert_eq!(result, vec![(1, 0.9), (3, 0.9)]);
    assert_eq!(result, cpu);
    Ok(())
}

#[test]
fn topk_k_ge_n_returns_all_sorted() -> Result<()> {
    let _guard = test_lock();
    let scores = [0.1, 0.9, 0.5, 0.9];
    let result = CudaBackend::new()?.topk(&scores, 8)?;
    println!("CUDA_TOPK_ALL {:?}", result);
    assert_eq!(result, vec![(1, 0.9), (3, 0.9), (2, 0.5), (0, 0.1)]);
    Ok(())
}

#[test]
fn topk_edges_equal_single_and_empty() -> Result<()> {
    let _guard = test_lock();
    let backend = CudaBackend::new()?;
    let equal = backend.topk(&[1.0, 1.0, 1.0, 1.0], 3)?;
    let single = backend.topk(&[42.0], 1)?;
    let empty = backend.topk(&[2.0, 1.0], 0)?;
    println!("CUDA_TOPK_EQUAL {:?}", equal);
    println!("CUDA_TOPK_SINGLE {:?}", single);
    println!("CUDA_TOPK_EMPTY {:?}", empty);
    assert_eq!(equal, vec![(0, 1.0), (1, 1.0), (2, 1.0)]);
    assert_eq!(single, vec![(0, 42.0)]);
    assert!(empty.is_empty());
    Ok(())
}

#[test]
fn topk_seed42_matches_cpu() -> Result<()> {
    let _guard = test_lock();
    let scores = seeded_scores(512, 42);
    let gpu = CudaBackend::new()?.topk(&scores, 8)?;
    let cpu = CpuBackend::new().topk(&scores, 8)?;
    println!("CUDA_TOPK_PROPTEST seed=42 indices={:?}", gpu);
    assert_eq!(gpu, cpu);
    assert_sorted(&gpu);
    Ok(())
}

#[test]
fn topk_nan_fails_closed() -> Result<()> {
    let _guard = test_lock();
    let err = CudaBackend::new()?
        .topk(&[1.0, f32::NAN, 0.5], 2)
        .expect_err("NaN score must fail closed");
    println!("CUDA_TOPK_NAN {err}");
    assert!(matches!(err, ForgeError::NumericalInvariant { .. }));
    Ok(())
}

fn score_case() -> impl Strategy<Value = (Vec<f32>, usize)> {
    (16usize..=512).prop_flat_map(|len| (proptest::collection::vec(-10.0f32..10.0, len), Just(len)))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    #[test]
    fn gpu_topk_matches_cpu_and_sorted((scores, len) in score_case()) {
        let _guard = test_lock();
        let k = 8.min(len);
        let gpu = CudaBackend::new()
            .map_err(|err| TestCaseError::fail(err.to_string()))?
            .topk(&scores, k)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        let cpu = CpuBackend::new()
            .topk(&scores, k)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        prop_assert_eq!(gpu.iter().map(|(idx, _)| *idx).collect::<Vec<_>>(),
            cpu.iter().map(|(idx, _)| *idx).collect::<Vec<_>>());
        for pair in gpu.windows(2) {
            let (left_index, left_score) = pair[0];
            let (right_index, right_score) = pair[1];
            prop_assert!(left_score > right_score
                || ((left_score - right_score).abs() <= 1e-5 && left_index < right_index));
        }
    }
}
