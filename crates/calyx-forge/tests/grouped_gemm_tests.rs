#[cfg(feature = "cuda")]
use std::sync::Mutex;

#[cfg(feature = "cuda")]
use calyx_forge::{
    GemmProblem,
    cpu::gemm_f32,
    cuda::{
        build_grouped_gemm_plan, build_ragged_batch_from_slabs, execute_grouped_gemm,
        extract_ragged_results, gemm_cublas, gemm_mxfp4_fp32_accum, init_cuda,
        read_grouped_gemm_output,
    },
    encode_mxfp4,
};
#[cfg(feature = "cuda")]
use rand::{Rng, SeedableRng};
#[cfg(feature = "cuda")]
use rand_chacha::ChaCha8Rng;

#[cfg(feature = "cuda")]
mod grouped_gemm_tests {
    use super::*;

    #[cfg(feature = "cuda")]
    const SEED: u64 = 0xCA1A_0000_0000_0015;
    #[cfg(feature = "cuda")]
    const SENTINEL: f32 = -919.0;
    #[cfg(feature = "cuda")]
    static CUDA_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    fn grouped_equals_per_loop() {
        #[cfg(feature = "cuda")]
        {
            let _guard = CUDA_LOCK.lock().unwrap_or_else(|err| err.into_inner());
            let ctx = init_cuda(0, false).expect("cuda context");
            let mut rng = rng();
            let mut problems = Vec::new();
            let mut a = Vec::new();
            let mut b = Vec::new();
            let mut c = Vec::new();
            for dims in [(2, 3, 2), (1, 5, 3), (4, 2, 1), (3, 3, 3), (2, 4, 5)] {
                append_case(&mut problems, &mut a, &mut b, &mut c, dims, &mut rng);
            }

            let grouped = run_grouped(&ctx, problems.clone(), &a, &b, &c);
            let mut per_problem = Vec::new();
            let mut max_err = 0.0_f32;
            for problem in problems.iter().flatten() {
                let expected = cublas_single(&ctx, *problem, &a, &b);
                let actual = output_for(*problem, &grouped);
                let err = assert_abs_bound(actual, &expected, 1e-4, "grouped_gemm");
                per_problem.push(err);
                max_err = max_err.max(err);
            }
            println!(
                "grouped_equals_per_loop PASSED problems=5 per_problem_max_err={per_problem:?} max_err={max_err:.3e}"
            );
        }
    }

    #[test]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    fn grouped_gemm_n_invariant() {
        #[cfg(feature = "cuda")]
        {
            let _guard = CUDA_LOCK.lock().unwrap_or_else(|err| err.into_inner());
            let ctx = init_cuda(0, false).expect("cuda context");
            let mut rng = rng();
            let mut problems3 = Vec::new();
            let mut a3 = Vec::new();
            let mut b3 = Vec::new();
            let mut c3 = Vec::new();
            for dims in [(2, 2, 2), (3, 2, 1), (1, 4, 3)] {
                append_case(&mut problems3, &mut a3, &mut b3, &mut c3, dims, &mut rng);
            }
            let out3 = run_grouped(&ctx, problems3.clone(), &a3, &b3, &c3);

            let mut problems5 = problems3.clone();
            let mut a5 = a3.clone();
            let mut b5 = b3.clone();
            let mut c5 = c3.clone();
            append_identity_case(&mut problems5, &mut a5, &mut b5, &mut c5, 2, 2, &mut rng);
            append_identity_case(&mut problems5, &mut a5, &mut b5, &mut c5, 3, 1, &mut rng);
            let out5 = run_grouped(&ctx, problems5.clone(), &a5, &b5, &c5);

            let mut max_delta = 0.0_f32;
            for problem in problems3.iter().flatten() {
                max_delta = max_delta.max(max_abs_err(
                    output_for(*problem, &out3),
                    output_for(*problem, &out5),
                ));
            }
            assert!(max_delta < 1e-5, "n_invariant_max_delta={max_delta}");
            println!("grouped_gemm_n_invariant PASSED n_invariant_max_delta={max_delta:.3e}");
        }
    }

    #[test]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    fn mxfp4_within_bound() {
        #[cfg(feature = "cuda")]
        {
            let _guard = CUDA_LOCK.lock().unwrap_or_else(|err| err.into_inner());
            let ctx = init_cuda(0, false).expect("cuda context");
            let m = 4;
            let k = 4;
            let n = 4;
            let a = exactish_values(m * k);
            let b = identity(k);
            let a_blocks = encode_mxfp4(&a).expect("encode fp4 A");
            let b_blocks = encode_mxfp4(&b).expect("encode fp4 B");
            let stream = ctx.inner().default_stream();
            let mut out_dev = stream.alloc_zeros(m * n).expect("fp4 output alloc");
            gemm_mxfp4_fp32_accum(&ctx, &a_blocks, &b_blocks, m, k, n, &mut out_dev)
                .expect("fp4 gemm");
            let fp4 = stream.clone_dtoh(&out_dev).expect("fp4 output readback");
            let mut expected = vec![0.0; m * n];
            gemm_f32(&a, &b, m, k, n, &mut expected).expect("f32 gemm");
            let max_rel = max_rel_bound(&fp4, &expected);
            assert!(max_rel <= 0.05, "fp4_within_bound={max_rel}");
            println!("mxfp4_within_bound PASSED fp4_within_bound={max_rel:.3e}");
        }
    }

    #[test]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    fn partial_bundle_correct() {
        #[cfg(feature = "cuda")]
        {
            let _guard = CUDA_LOCK.lock().unwrap_or_else(|err| err.into_inner());
            let ctx = init_cuda(0, false).expect("cuda context");
            let mut rng = rng();
            let mut a = Vec::new();
            let mut b = Vec::new();
            let mut c = Vec::new();
            let mut rows = Vec::new();
            for cx in 0..4 {
                let mut row = Vec::new();
                for slot in 0..3 {
                    if cx == 2 && slot == 1 {
                        row.push(None);
                    } else {
                        row.push(Some(make_case(&mut a, &mut b, &mut c, (2, 2, 2), &mut rng)));
                    }
                }
                rows.push(row);
            }
            let mut batch = build_ragged_batch_from_slabs(&ctx, rows.clone(), &a, &b, &c)
                .expect("ragged batch");
            execute_grouped_gemm(&ctx, &mut batch.plan).expect("ragged grouped gemm");
            let results = extract_ragged_results(&batch);
            assert!(results[2][1].is_none(), "absent slot must stay None");
            let mut max_err = 0.0_f32;
            for (row, result_row) in rows.iter().zip(results.iter()) {
                for (problem, actual) in row.iter().zip(result_row.iter()) {
                    match (problem, actual) {
                        (Some(problem), Some(actual)) => {
                            let expected = cublas_single(&ctx, *problem, &a, &b);
                            max_err = max_err.max(assert_abs_bound(
                                actual,
                                &expected,
                                1e-4,
                                "partial_bundle",
                            ));
                        }
                        (None, None) => {}
                        _ => panic!("partial bundle Option shape changed"),
                    }
                }
            }
            println!("partial_bundle_correct PASSED absent_slot=None max_err={max_err:.3e}");
        }
    }

    #[test]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    fn grouped_gemm_edges_n1_n32_and_mixed_fp4_f32() {
        #[cfg(feature = "cuda")]
        {
            let _guard = CUDA_LOCK.lock().unwrap_or_else(|err| err.into_inner());
            let ctx = init_cuda(0, false).expect("cuda context");
            let n1_err = grouped_vs_loop_for_n(&ctx, 1);
            let n32_err = grouped_vs_loop_for_n(&ctx, 32);
            let fp4_rel = fp4_identity_rel_err(&ctx);
            assert!(n1_err <= 1e-4, "n1_err={n1_err}");
            assert!(n32_err <= 1e-4, "n32_err={n32_err}");
            assert!(fp4_rel <= 0.05, "fp4_rel={fp4_rel}");
            println!(
                "grouped_gemm_edges PASSED n1_err={n1_err:.3e} n32_err={n32_err:.3e} mixed_fp4_f32 fp4_within={fp4_rel:.3e}"
            );
        }
    }

    #[cfg(feature = "cuda")]
    fn rng() -> ChaCha8Rng {
        ChaCha8Rng::seed_from_u64(SEED)
    }

    #[cfg(feature = "cuda")]
    fn append_case(
        problems: &mut Vec<Option<GemmProblem>>,
        a: &mut Vec<f32>,
        b: &mut Vec<f32>,
        c: &mut Vec<f32>,
        dims: (usize, usize, usize),
        rng: &mut ChaCha8Rng,
    ) -> GemmProblem {
        let problem = make_case(a, b, c, dims, rng);
        problems.push(Some(problem));
        problem
    }

    #[cfg(feature = "cuda")]
    fn make_case(
        a: &mut Vec<f32>,
        b: &mut Vec<f32>,
        c: &mut Vec<f32>,
        dims: (usize, usize, usize),
        rng: &mut ChaCha8Rng,
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
        a.extend(random_values(rng, m * k));
        b.extend(random_values(rng, k * n));
        c.extend(vec![SENTINEL; m * n]);
        problem
    }

    #[cfg(feature = "cuda")]
    fn append_identity_case(
        problems: &mut Vec<Option<GemmProblem>>,
        a: &mut Vec<f32>,
        b: &mut Vec<f32>,
        c: &mut Vec<f32>,
        size: usize,
        cols: usize,
        rng: &mut ChaCha8Rng,
    ) -> GemmProblem {
        let problem = GemmProblem {
            m: size,
            k: size,
            n: cols,
            a_offset: a.len(),
            b_offset: b.len(),
            c_offset: c.len(),
        };
        a.extend(identity(size));
        b.extend(random_values(rng, size * cols));
        c.extend(vec![SENTINEL; size * cols]);
        problems.push(Some(problem));
        problem
    }

    #[cfg(feature = "cuda")]
    fn run_grouped(
        ctx: &calyx_forge::CudaContext,
        problems: Vec<Option<GemmProblem>>,
        a: &[f32],
        b: &[f32],
        c: &[f32],
    ) -> Vec<f32> {
        let mut plan = build_grouped_gemm_plan(ctx, problems, a, b, c).expect("grouped plan");
        execute_grouped_gemm(ctx, &mut plan).expect("grouped execute");
        read_grouped_gemm_output(ctx, &plan).expect("grouped output readback")
    }

    #[cfg(feature = "cuda")]
    fn cublas_single(
        ctx: &calyx_forge::CudaContext,
        problem: GemmProblem,
        a: &[f32],
        b: &[f32],
    ) -> Vec<f32> {
        let stream = ctx.inner().default_stream();
        let a_part = &a[problem.a_offset..problem.a_offset + problem.m * problem.k];
        let b_part = &b[problem.b_offset..problem.b_offset + problem.k * problem.n];
        let a_dev = stream.clone_htod(a_part).expect("single A copy");
        let b_dev = stream.clone_htod(b_part).expect("single B copy");
        let mut out_dev = stream
            .alloc_zeros(problem.m * problem.n)
            .expect("single output alloc");
        gemm_cublas(
            ctx,
            &a_dev,
            &b_dev,
            problem.m,
            problem.k,
            problem.n,
            &mut out_dev,
        )
        .expect("single cublas gemm");
        stream.synchronize().expect("single gemm sync");
        stream.clone_dtoh(&out_dev).expect("single output readback")
    }

    #[cfg(feature = "cuda")]
    fn grouped_vs_loop_for_n(ctx: &calyx_forge::CudaContext, count: usize) -> f32 {
        let mut rng = rng();
        let mut problems = Vec::new();
        let mut a = Vec::new();
        let mut b = Vec::new();
        let mut c = Vec::new();
        for idx in 0..count {
            let dim = 1 + (idx % 4);
            append_case(
                &mut problems,
                &mut a,
                &mut b,
                &mut c,
                (dim, dim, dim),
                &mut rng,
            );
        }
        let grouped = run_grouped(ctx, problems.clone(), &a, &b, &c);
        problems
            .iter()
            .flatten()
            .map(|problem| {
                let expected = cublas_single(ctx, *problem, &a, &b);
                max_abs_err(output_for(*problem, &grouped), &expected)
            })
            .fold(0.0, f32::max)
    }

    #[cfg(feature = "cuda")]
    fn fp4_identity_rel_err(ctx: &calyx_forge::CudaContext) -> f32 {
        let m = 4;
        let k = 4;
        let n = 4;
        let a = exactish_values(m * k);
        let b = identity(k);
        let stream = ctx.inner().default_stream();
        let mut out_dev = stream.alloc_zeros(m * n).expect("fp4 mixed alloc");
        gemm_mxfp4_fp32_accum(
            ctx,
            &encode_mxfp4(&a).expect("fp4 mixed A"),
            &encode_mxfp4(&b).expect("fp4 mixed B"),
            m,
            k,
            n,
            &mut out_dev,
        )
        .expect("fp4 mixed gemm");
        let fp4 = stream.clone_dtoh(&out_dev).expect("fp4 mixed read");
        let mut expected = vec![0.0; m * n];
        gemm_f32(&a, &b, m, k, n, &mut expected).expect("mixed f32 gemm");
        max_rel_bound(&fp4, &expected)
    }

    #[cfg(feature = "cuda")]
    fn output_for(problem: GemmProblem, out: &[f32]) -> &[f32] {
        &out[problem.c_offset..problem.c_offset + problem.m * problem.n]
    }

    #[cfg(feature = "cuda")]
    fn assert_abs_bound(actual: &[f32], expected: &[f32], tol: f32, op: &str) -> f32 {
        let err = max_abs_err(actual, expected);
        assert!(err <= tol, "{op} max_err={err:.3e} tol={tol:.3e}");
        err
    }

    #[cfg(feature = "cuda")]
    fn max_abs_err(actual: &[f32], expected: &[f32]) -> f32 {
        actual
            .iter()
            .zip(expected.iter())
            .map(|(left, right)| (*left - *right).abs())
            .fold(0.0, f32::max)
    }

    #[cfg(feature = "cuda")]
    fn max_rel_bound(actual: &[f32], expected: &[f32]) -> f32 {
        actual
            .iter()
            .zip(expected.iter())
            .map(|(left, right)| (*left - *right).abs() / right.abs().max(1.0))
            .fold(0.0, f32::max)
    }

    #[cfg(feature = "cuda")]
    fn random_values(rng: &mut ChaCha8Rng, len: usize) -> Vec<f32> {
        (0..len)
            .map(|_| rng.gen_range(-0.75_f32..0.75_f32))
            .collect()
    }

    #[cfg(feature = "cuda")]
    fn exactish_values(len: usize) -> Vec<f32> {
        (0..len)
            .map(|idx| ((idx % 15) as f32 - 7.0) / 7.0)
            .collect()
    }

    #[cfg(feature = "cuda")]
    fn identity(size: usize) -> Vec<f32> {
        let mut out = vec![0.0; size * size];
        for idx in 0..size {
            out[col_major(idx, idx, size)] = 1.0;
        }
        out
    }

    #[cfg(feature = "cuda")]
    fn col_major(row: usize, col: usize, rows: usize) -> usize {
        col * rows + row
    }
}
