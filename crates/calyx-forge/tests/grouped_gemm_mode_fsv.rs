#[cfg(feature = "cuda")]
use std::path::PathBuf;

#[cfg(feature = "cuda")]
use calyx_forge::{
    GemmProblem, GroupedGemmExecutionMode, build_grouped_gemm_plan, cpu::gemm_f32,
    execute_grouped_gemm, execute_grouped_gemm_strict, init_cuda, read_grouped_gemm_output,
};
#[cfg(feature = "cuda")]
use serde_json::json;

#[cfg(feature = "cuda")]
const SENTINEL: f32 = -316.0;

#[cfg(feature = "cuda")]
#[test]
#[ignore = "manual FSV for PH15 grouped GEMM execution mode"]
fn ph15_grouped_gemm_execution_mode_manual_fsv() {
    let root = fsv_root();
    std::fs::create_dir_all(&root).expect("create fsv root");
    let _guard = CUDA_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let ctx = init_cuda(0, false).expect("cuda context");
    let (problems, a, b, c) = sample_batch();

    let mut allowed_plan =
        build_grouped_gemm_plan(&ctx, problems.clone(), &a, &b, &c).expect("allowed plan");
    execute_grouped_gemm(&ctx, &mut allowed_plan).expect("allowed grouped execute");
    let allowed_output = read_grouped_gemm_output(&ctx, &allowed_plan).expect("allowed readback");
    let allowed_err = assert_outputs(&problems, &a, &b, &allowed_output);
    assert!(matches!(
        allowed_plan.execution_mode,
        GroupedGemmExecutionMode::GroupedBatched | GroupedGemmExecutionMode::SequentialFallback
    ));
    assert!(allowed_err <= 1e-4, "allowed_err={allowed_err}");

    let mut strict_plan =
        build_grouped_gemm_plan(&ctx, problems.clone(), &a, &b, &c).expect("strict plan");
    let strict = match execute_grouped_gemm_strict(&ctx, &mut strict_plan) {
        Ok(()) => {
            let out = read_grouped_gemm_output(&ctx, &strict_plan).expect("strict readback");
            let err = assert_outputs(&problems, &a, &b, &out);
            assert_eq!(
                strict_plan.execution_mode,
                GroupedGemmExecutionMode::GroupedBatched
            );
            assert!(err <= 1e-4, "strict_err={err}");
            json!({"ok": true, "mode": strict_plan.execution_mode.as_str(), "max_abs_err": err})
        }
        Err(err) => {
            assert_eq!(err.code(), "CALYX_FORGE_NUMERICAL_INVARIANT");
            json!({"ok": false, "mode": strict_plan.execution_mode.as_str(), "error": err.to_string()})
        }
    };

    let empty_c = vec![SENTINEL; 4];
    let mut empty_plan = build_grouped_gemm_plan(&ctx, vec![None, None], &[0.0], &[0.0], &empty_c)
        .expect("empty plan");
    execute_grouped_gemm(&ctx, &mut empty_plan).expect("empty execute");
    let empty_output = read_grouped_gemm_output(&ctx, &empty_plan).expect("empty readback");
    assert_eq!(
        empty_plan.execution_mode,
        GroupedGemmExecutionMode::NoActiveProblems
    );
    assert_eq!(empty_output, empty_c);

    let readback = json!({
        "allowed_mode": allowed_plan.execution_mode.as_str(),
        "allowed_max_abs_err": allowed_err,
        "allowed_output": allowed_output,
        "strict": strict,
        "empty_mode": empty_plan.execution_mode.as_str(),
        "empty_output": empty_output,
        "active_problem_count": problems.iter().flatten().count(),
    });
    let path = root.join("grouped-gemm-mode-readback.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!("PH15_GROUPED_GEMM_MODE_FSV_ROOT={}", root.display());
    println!("PH15_GROUPED_GEMM_MODE_READBACK={}", path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());
}

#[cfg(feature = "cuda")]
static CUDA_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(feature = "cuda")]
fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph15-grouped-gemm-mode-fsv"))
}

#[cfg(feature = "cuda")]
fn sample_batch() -> (Vec<Option<GemmProblem>>, Vec<f32>, Vec<f32>, Vec<f32>) {
    let mut problems = Vec::new();
    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut c = Vec::new();
    append_case(&mut problems, &mut a, &mut b, &mut c, (2, 2, 2), 3);
    append_case(&mut problems, &mut a, &mut b, &mut c, (1, 3, 2), 7);
    append_case(&mut problems, &mut a, &mut b, &mut c, (3, 1, 1), 11);
    (problems, a, b, c)
}

#[cfg(feature = "cuda")]
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
    b.extend(values(k * n, seed + 17, 0.03125));
    c.extend(vec![SENTINEL; m * n]);
    problems.push(Some(problem));
}

#[cfg(feature = "cuda")]
fn values(len: usize, seed: usize, scale: f32) -> Vec<f32> {
    (0..len)
        .map(|idx| ((idx + seed) % 19) as f32 - 9.0)
        .map(|value| value * scale)
        .collect()
}

#[cfg(feature = "cuda")]
fn assert_outputs(problems: &[Option<GemmProblem>], a: &[f32], b: &[f32], c: &[f32]) -> f32 {
    problems
        .iter()
        .flatten()
        .map(|problem| {
            let mut expected = vec![0.0; problem.m * problem.n];
            gemm_f32(
                &a[problem.a_offset..problem.a_offset + problem.m * problem.k],
                &b[problem.b_offset..problem.b_offset + problem.k * problem.n],
                problem.m,
                problem.k,
                problem.n,
                &mut expected,
            )
            .expect("cpu expected");
            max_abs_err(
                &c[problem.c_offset..problem.c_offset + problem.m * problem.n],
                &expected,
            )
        })
        .fold(0.0, f32::max)
}

#[cfg(feature = "cuda")]
fn max_abs_err(actual: &[f32], expected: &[f32]) -> f32 {
    actual
        .iter()
        .zip(expected.iter())
        .map(|(left, right)| (*left - *right).abs())
        .fold(0.0, f32::max)
}
