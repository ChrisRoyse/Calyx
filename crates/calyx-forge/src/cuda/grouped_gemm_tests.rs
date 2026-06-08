use super::grouped_gemm::{
    GemmProblem, GroupedGemmExecutionMode, build_grouped_gemm_plan, execute_grouped_gemm,
    execute_grouped_gemm_strict, read_grouped_gemm_output,
};
use super::ragged_gemm::{
    build_ragged_batch, build_ragged_batch_from_slabs, extract_ragged_results,
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
) -> GemmProblem {
    let problem = make_case(a, b, c, dims, seed);
    problems.push(Some(problem));
    problem
}

fn make_case(
    a: &mut Vec<f32>,
    b: &mut Vec<f32>,
    c: &mut Vec<f32>,
    dims: (usize, usize, usize),
    seed: usize,
) -> GemmProblem {
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
    problem
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

fn assert_ragged_outputs(
    problems: &[Vec<Option<GemmProblem>>],
    results: &[Vec<Option<Vec<f32>>>],
    a: &[f32],
    b: &[f32],
) -> Result<f32> {
    let mut max = 0.0_f32;
    for (problem_row, result_row) in problems.iter().zip(results.iter()) {
        for (problem, result) in problem_row.iter().zip(result_row.iter()) {
            match (problem, result) {
                (Some(problem), Some(actual)) => {
                    let expected = expected_for(*problem, a, b)?;
                    max = max.max(max_err(actual, &expected));
                }
                (None, None) => {}
                _ => panic!("ragged result shape must preserve Option slots"),
            }
        }
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
    execute_grouped_gemm_strict(&ctx, &mut plan)?;
    assert_eq!(
        plan.execution_mode,
        GroupedGemmExecutionMode::GroupedBatched
    );
    let out = read_grouped_gemm_output(&ctx, &plan)?;
    let err = assert_outputs(&problems, &a, &b, &out)?;
    assert!(err <= 1e-5, "max_err={err}");
    println!(
        "grouped_gemm_one PASSED mode={} max_err={err:.3e}",
        plan.execution_mode.as_str()
    );
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
    assert!(matches!(
        plan.execution_mode,
        GroupedGemmExecutionMode::GroupedBatched | GroupedGemmExecutionMode::SequentialFallback
    ));
    let out = read_grouped_gemm_output(&ctx, &plan)?;
    let err = assert_outputs(&problems, &a, &b, &out)?;
    assert!(err <= 1e-4, "max_err={err}");
    println!(
        "grouped_equals_per_loop PASSED grouped=3 per_loop=3 mode={} max_err={err:.3e}",
        plan.execution_mode.as_str()
    );
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
    assert_eq!(
        plan.execution_mode,
        GroupedGemmExecutionMode::NoActiveProblems
    );
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

#[test]
fn ragged_absent_slot_no_zero_fill() -> Result<()> {
    let _guard = crate::cuda::test_lock();
    let ctx = crate::init_cuda(0, false)?;
    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut c = Vec::new();
    let p00 = make_case(&mut a, &mut b, &mut c, (2, 2, 2), 21);
    let p02 = make_case(&mut a, &mut b, &mut c, (1, 3, 2), 22);
    let p10 = make_case(&mut a, &mut b, &mut c, (2, 1, 3), 23);
    let p11 = make_case(&mut a, &mut b, &mut c, (3, 2, 1), 24);
    let p12 = make_case(&mut a, &mut b, &mut c, (2, 2, 2), 25);
    let problems = vec![
        vec![Some(p00), None, Some(p02)],
        vec![Some(p10), Some(p11), Some(p12)],
    ];
    let mut batch = build_ragged_batch_from_slabs(&ctx, problems.clone(), &a, &b, &c)?;
    execute_grouped_gemm(&ctx, &mut batch.plan)?;
    let results = extract_ragged_results(&batch);
    assert!(results[0][1].is_none());
    let err = assert_ragged_outputs(&problems, &results, &a, &b)?;
    assert!(err <= 1e-4, "max_err={err}");
    println!(
        "ragged_absent_slot_no_zero_fill PASSED slot[0][1]={:?} slot[0][0]={:?} max_err={err:.3e}",
        results[0][1], results[0][0]
    );
    Ok(())
}

#[test]
fn ragged_all_absent_returns_none() -> Result<()> {
    let _guard = crate::cuda::test_lock();
    let ctx = crate::init_cuda(0, false)?;
    let mut batch = build_ragged_batch(&ctx, vec![vec![None, None], vec![None, None]])?;
    execute_grouped_gemm(&ctx, &mut batch.plan)?;
    let results = extract_ragged_results(&batch);
    assert!(results.iter().flatten().all(Option::is_none));
    println!("ragged_all_absent PASSED all_none=true results={results:?}");
    Ok(())
}

#[test]
fn ragged_all_present_matches_cpu() -> Result<()> {
    let _guard = crate::cuda::test_lock();
    let ctx = crate::init_cuda(0, false)?;
    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut c = Vec::new();
    let problems = vec![
        vec![
            Some(make_case(&mut a, &mut b, &mut c, (2, 2, 2), 31)),
            Some(make_case(&mut a, &mut b, &mut c, (3, 1, 2), 32)),
        ],
        vec![
            Some(make_case(&mut a, &mut b, &mut c, (1, 4, 3), 33)),
            Some(make_case(&mut a, &mut b, &mut c, (2, 3, 1), 34)),
        ],
    ];
    let mut batch = build_ragged_batch_from_slabs(&ctx, problems.clone(), &a, &b, &c)?;
    execute_grouped_gemm(&ctx, &mut batch.plan)?;
    let results = extract_ragged_results(&batch);
    let err = assert_ragged_outputs(&problems, &results, &a, &b)?;
    assert!(err <= 1e-4, "max_err={err}");
    println!("ragged_all_present PASSED present=4 max_err={err:.3e}");
    Ok(())
}

#[test]
fn ragged_edges_cover_first_absent_and_large_present() -> Result<()> {
    let _guard = crate::cuda::test_lock();
    let ctx = crate::init_cuda(0, false)?;
    let mut all_absent = build_ragged_batch(&ctx, vec![vec![None, None, None]])?;
    execute_grouped_gemm(&ctx, &mut all_absent.plan)?;
    assert!(
        extract_ragged_results(&all_absent)
            .iter()
            .flatten()
            .all(Option::is_none)
    );

    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut c = Vec::new();
    let mut rows = Vec::new();
    for idx in 0..100 {
        rows.push(vec![Some(make_case(
            &mut a,
            &mut b,
            &mut c,
            (1, 1, 1),
            idx,
        ))]);
    }
    let mut large = build_ragged_batch_from_slabs(&ctx, rows.clone(), &a, &b, &c)?;
    execute_grouped_gemm(&ctx, &mut large.plan)?;
    let large_results = extract_ragged_results(&large);
    let err = assert_ragged_outputs(&rows, &large_results, &a, &b)?;
    assert!(err <= 1e-4, "max_err={err}");

    let first_present = make_case(&mut a, &mut b, &mut c, (1, 2, 1), 141);
    let first_absent = vec![vec![None, Some(first_present)]];
    let mut first = build_ragged_batch_from_slabs(&ctx, first_absent, &a, &b, &c)?;
    execute_grouped_gemm(&ctx, &mut first.plan)?;
    let first_results = extract_ragged_results(&first);
    assert!(first_results[0][0].is_none());
    println!(
        "ragged_edges PASSED cx1_all_absent=true cx100_all_present=100 first_slot={:?} max_err={err:.3e}",
        first_results[0][0]
    );
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

    #[test]
    fn ragged_square_proptest(mask in proptest::collection::vec(any::<bool>(), 16..=16)) {
        let _guard = crate::cuda::test_lock();
        let ctx = crate::init_cuda(0, false)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        let mut a = Vec::new();
        let mut b = Vec::new();
        let mut c = Vec::new();
        let mut rows = Vec::new();
        for cx in 0..4 {
            let mut row = Vec::new();
            for slot in 0..4 {
                let idx = cx * 4 + slot;
                if mask[idx] {
                    row.push(Some(make_case(&mut a, &mut b, &mut c, (2, 2, 2), 42 + idx)));
                } else {
                    row.push(None);
                }
            }
            rows.push(row);
        }
        if a.is_empty() {
            a.push(0.0);
            b.push(0.0);
        }
        let mut batch = build_ragged_batch_from_slabs(&ctx, rows.clone(), &a, &b, &c)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        execute_grouped_gemm(&ctx, &mut batch.plan)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        let results = extract_ragged_results(&batch);
        let err = assert_ragged_outputs(&rows, &results, &a, &b)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        prop_assert!(err <= 1e-4, "max_err={err}");
        prop_assert!(rows.iter().flatten().zip(results.iter().flatten()).all(
            |(problem, result)| problem.is_some() == result.is_some()
        ));
        println!("ragged_square_proptest PASSED seed=42 max_err={err:.3e}");
    }
}
