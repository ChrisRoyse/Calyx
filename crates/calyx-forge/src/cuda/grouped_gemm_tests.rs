use super::grouped_gemm::{
    GemmProblem, build_grouped_gemm_plan, execute_grouped_gemm, read_grouped_gemm_output,
};
use crate::cpu::gemm_f32;
use crate::{ForgeError, Result};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;

const SENTINEL: f32 = -777.0;

fn append_case(
    problems: &mut Vec<Option<GemmProblem>>,
    a: &mut Vec<f32>,
    b: &mut Vec<f32>,
    c: &mut Vec<f32>,
    dims: (usize, usize, usize),
    seed: usize,
) {
    let (m, k, n) = dims;
    let problem = GemmProblem {
        m,
        k,
        n,
        a_offset: a.len(),
        b_offset: b.len(),
        c_offset: c.len(),
    };
    a.extend(values(m * k, seed, 0.0625));
    b.extend(values(k * n, seed + 11, 0.03125));
    c.extend(vec![SENTINEL; m * n]);
    problems.push(Some(problem));
}

fn values(len: usize, seed: usize, scale: f32) -> Vec<f32> {
    (0..len)
        .map(|idx| ((idx + seed) % 17) as f32 - 8.0)
        .map(|value| value * scale)
        .collect()
}

fn expected_for(problem: GemmProblem, a: &[f32], b: &[f32]) -> Result<Vec<f32>> {
    let mut out = vec![0.0; problem.m * problem.n];
    gemm_f32(
        &a[problem.a_offset..problem.a_offset + problem.m * problem.k],
        &b[problem.b_offset..problem.b_offset + problem.k * problem.n],
        problem.m,
        problem.k,
        problem.n,
        &mut out,
    )?;
    Ok(out)
}

fn assert_outputs(
    problems: &[Option<GemmProblem>],
    a: &[f32],
    b: &[f32],
    c: &[f32],
) -> Result<f32> {
    let mut max = 0.0_f32;
    for problem in problems.iter().flatten() {
        let expected = expected_for(*problem, a, b)?;
        let start = problem.c_offset;
        let end = start + problem.m * problem.n;
        max = max.max(max_err(&c[start..end], &expected));
    }
    Ok(max)
}

fn max_err(actual: &[f32], expected: &[f32]) -> f32 {
    actual
        .iter()
        .zip(expected.iter())
        .map(|(left, right)| (*left - *right).abs())
        .fold(0.0, f32::max)
}

#[test]
fn grouped_gemm_one_matches_single_gemm() -> Result<()> {
    let _guard = crate::cuda::test_lock();
    let ctx = crate::init_cuda(0, false)?;
    let mut problems = Vec::new();
    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut c = Vec::new();
    append_case(&mut problems, &mut a, &mut b, &mut c, (2, 2, 2), 1);
    let mut plan = build_grouped_gemm_plan(&ctx, problems.clone(), &a, &b, &c)?;
    execute_grouped_gemm(&ctx, &mut plan)?;
    let out = read_grouped_gemm_output(&ctx, &plan)?;
    let err = assert_outputs(&problems, &a, &b, &out)?;
    assert!(err <= 1e-5, "max_err={err}");
    println!("grouped_gemm_one PASSED max_err={err:.3e}");
    Ok(())
}

#[test]
fn grouped_equals_per_loop() -> Result<()> {
    let _guard = crate::cuda::test_lock();
    let ctx = crate::init_cuda(0, false)?;
    let mut problems = Vec::new();
    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut c = Vec::new();
    append_case(&mut problems, &mut a, &mut b, &mut c, (2, 2, 2), 3);
    append_case(&mut problems, &mut a, &mut b, &mut c, (4, 3, 2), 7);
    append_case(&mut problems, &mut a, &mut b, &mut c, (1, 5, 3), 13);
    let mut plan = build_grouped_gemm_plan(&ctx, problems.clone(), &a, &b, &c)?;
    execute_grouped_gemm(&ctx, &mut plan)?;
    let out = read_grouped_gemm_output(&ctx, &plan)?;
    let err = assert_outputs(&problems, &a, &b, &out)?;
    assert!(err <= 1e-4, "max_err={err}");
    println!("grouped_equals_per_loop PASSED grouped=3 per_loop=3 max_err={err:.3e}");
    Ok(())
}

#[test]
fn grouped_absent_slots_do_not_modify_gap() -> Result<()> {
    let _guard = crate::cuda::test_lock();
    let ctx = crate::init_cuda(0, false)?;
    let mut problems = Vec::new();
    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut c = Vec::new();
    append_case(&mut problems, &mut a, &mut b, &mut c, (2, 2, 2), 5);
    problems.push(None);
    c.extend(vec![SENTINEL; 4]);
    let gap = c.len() - 4..c.len();
    append_case(&mut problems, &mut a, &mut b, &mut c, (1, 3, 2), 9);
    let mut plan = build_grouped_gemm_plan(&ctx, problems.clone(), &a, &b, &c)?;
    execute_grouped_gemm(&ctx, &mut plan)?;
    let out = read_grouped_gemm_output(&ctx, &plan)?;
    assert!(out[gap.clone()].iter().all(|value| *value == SENTINEL));
    let err = assert_outputs(&problems, &a, &b, &out)?;
    assert!(err <= 1e-4, "max_err={err}");
    println!(
        "grouped_absent_slot PASSED max_err={err:.3e} gap_values={:?}",
        &out[gap]
    );
    Ok(())
}

#[test]
fn grouped_all_none_and_shape_mismatch_edges() -> Result<()> {
    let _guard = crate::cuda::test_lock();
    let ctx = crate::init_cuda(0, false)?;
    let c = vec![SENTINEL; 3];
    let mut plan = build_grouped_gemm_plan(&ctx, vec![None, None], &[0.0], &[0.0], &c)?;
    execute_grouped_gemm(&ctx, &mut plan)?;
    let out = read_grouped_gemm_output(&ctx, &plan)?;
    assert_eq!(out, c);

    let bad = GemmProblem {
        m: 4,
        k: 4,
        n: 4,
        a_offset: 0,
        b_offset: 0,
        c_offset: 0,
    };
    let err =
        match build_grouped_gemm_plan(&ctx, vec![Some(bad)], &[1.0; 3], &[1.0; 16], &[0.0; 16]) {
            Ok(_) => panic!("short A slab must fail closed"),
            Err(err) => err,
        };
    println!("grouped_edges PASSED all_none=true {err}");
    assert!(matches!(err, ForgeError::ShapeMismatch { .. }));
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4))]

    #[test]
    fn grouped_square_proptest(dims in proptest::collection::vec(2usize..=16, 1..=8)) {
        let _guard = crate::cuda::test_lock();
        let ctx = crate::init_cuda(0, false)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        let mut problems = Vec::new();
        let mut a = Vec::new();
        let mut b = Vec::new();
        let mut c = Vec::new();
        for (idx, dim) in dims.iter().enumerate() {
            append_case(&mut problems, &mut a, &mut b, &mut c, (*dim, *dim, *dim), idx);
        }
        let mut plan = build_grouped_gemm_plan(&ctx, problems.clone(), &a, &b, &c)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        execute_grouped_gemm(&ctx, &mut plan)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        let out = read_grouped_gemm_output(&ctx, &plan)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        let err = assert_outputs(&problems, &a, &b, &out)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        prop_assert!(err <= 1e-4, "max_err={err}");
    }
}
