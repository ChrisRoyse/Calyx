use super::grouped_gemm::{
    ABSENT_SENTINEL, AbsentSlotSentinel, GemmProblem, GroupedGemmPlan,
    build_grouped_gemm_plan_with_metadata, read_grouped_gemm_output,
};
use crate::{CudaContext, ForgeError, Result};

pub struct RaggedBatch {
    pub n_constellations: usize,
    pub n_slots: usize,
    pub plan: GroupedGemmPlan,
    ctx: CudaContext,
}

struct FlatRagged {
    n_constellations: usize,
    n_slots: usize,
    problems: Vec<Option<GemmProblem>>,
    slot_ids: Vec<Option<usize>>,
}

pub fn build_ragged_batch(
    ctx: &CudaContext,
    problems: Vec<Vec<Option<GemmProblem>>>,
) -> Result<RaggedBatch> {
    let flat = flatten_ragged(problems)?;
    let (a_len, b_len, c_len) = required_slab_lens(&flat.problems)?;
    let a_host = vec![0.0; a_len.max(1)];
    let b_host = vec![0.0; b_len.max(1)];
    let c_init = vec![0.0; c_len.max(1)];
    build_ragged_batch_from_parts(ctx, flat, &a_host, &b_host, &c_init)
}

pub fn build_ragged_batch_from_slabs(
    ctx: &CudaContext,
    problems: Vec<Vec<Option<GemmProblem>>>,
    a_host: &[f32],
    b_host: &[f32],
    c_init: &[f32],
) -> Result<RaggedBatch> {
    let flat = flatten_ragged(problems)?;
    build_ragged_batch_from_parts(ctx, flat, a_host, b_host, c_init)
}

pub fn extract_ragged_results(batch: &RaggedBatch) -> Vec<Vec<Option<Vec<f32>>>> {
    try_extract_ragged_results(batch).expect("read ragged grouped GEMM output")
}

pub fn try_extract_ragged_results(batch: &RaggedBatch) -> Result<Vec<Vec<Option<Vec<f32>>>>> {
    let out = read_grouped_gemm_output(&batch.ctx, &batch.plan)?;
    let mut rows = Vec::with_capacity(batch.n_constellations);
    for cx in 0..batch.n_constellations {
        let mut row = Vec::with_capacity(batch.n_slots);
        for slot in 0..batch.n_slots {
            let idx = cx * batch.n_slots + slot;
            row.push(batch.plan.problems[idx].map(|problem| {
                let start = problem.c_offset;
                out[start..start + problem.m * problem.n].to_vec()
            }));
        }
        rows.push(row);
    }
    Ok(rows)
}

fn build_ragged_batch_from_parts(
    ctx: &CudaContext,
    flat: FlatRagged,
    a_host: &[f32],
    b_host: &[f32],
    c_init: &[f32],
) -> Result<RaggedBatch> {
    let FlatRagged {
        n_constellations,
        n_slots,
        problems,
        slot_ids,
    } = flat;
    let mut c_with_sentinels = c_init.to_vec();
    let mut sentinels = Vec::new();
    for (flat_idx, problem) in problems.iter().enumerate() {
        if problem.is_none() {
            let c_offset = c_with_sentinels.len();
            c_with_sentinels.push(ABSENT_SENTINEL);
            sentinels.push(AbsentSlotSentinel {
                flat_idx,
                c_offset,
                len: 1,
            });
        }
    }
    if c_with_sentinels.is_empty() {
        c_with_sentinels.push(0.0);
    }
    let a_fallback;
    let a_input = if a_host.is_empty() {
        a_fallback = vec![0.0];
        &a_fallback
    } else {
        a_host
    };
    let b_fallback;
    let b_input = if b_host.is_empty() {
        b_fallback = vec![0.0];
        &b_fallback
    } else {
        b_host
    };
    let plan = build_grouped_gemm_plan_with_metadata(
        ctx,
        problems,
        slot_ids,
        sentinels,
        a_input,
        b_input,
        &c_with_sentinels,
    )?;
    Ok(RaggedBatch {
        n_constellations,
        n_slots,
        plan,
        ctx: ctx.clone(),
    })
}

fn flatten_ragged(problems: Vec<Vec<Option<GemmProblem>>>) -> Result<FlatRagged> {
    let n_constellations = problems.len();
    let n_slots = problems.first().map_or(0, Vec::len);
    let mut flat = Vec::with_capacity(n_constellations * n_slots);
    let mut slot_ids = Vec::with_capacity(n_constellations * n_slots);
    for (cx, row) in problems.into_iter().enumerate() {
        if row.len() != n_slots {
            return Err(ForgeError::ShapeMismatch {
                expected: vec![n_slots],
                got: vec![cx, row.len()],
                remediation: "Ragged grouped GEMM rows must have a consistent slot count"
                    .to_string(),
            });
        }
        for (slot, problem) in row.into_iter().enumerate() {
            slot_ids.push(problem.as_ref().map(|_| slot));
            flat.push(problem);
        }
    }
    Ok(FlatRagged {
        n_constellations,
        n_slots,
        problems: flat,
        slot_ids,
    })
}

fn required_slab_lens(problems: &[Option<GemmProblem>]) -> Result<(usize, usize, usize)> {
    let mut a_len = 0;
    let mut b_len = 0;
    let mut c_len = 0;
    for problem in problems.iter().flatten() {
        let a_need = matrix_len(problem.m, problem.k, "ragged A")?;
        let b_need = matrix_len(problem.k, problem.n, "ragged B")?;
        let c_need = matrix_len(problem.m, problem.n, "ragged C")?;
        a_len = a_len.max(checked_end(problem.a_offset, a_need, "ragged A")?);
        b_len = b_len.max(checked_end(problem.b_offset, b_need, "ragged B")?);
        c_len = c_len.max(checked_end(problem.c_offset, c_need, "ragged C")?);
    }
    Ok((a_len, b_len, c_len))
}

fn matrix_len(rows: usize, cols: usize, name: &str) -> Result<usize> {
    rows.checked_mul(cols)
        .ok_or_else(|| ForgeError::ShapeMismatch {
            expected: vec![rows, cols],
            got: vec![usize::MAX],
            remediation: format!("{name} shape overflows usize"),
        })
}

fn checked_end(offset: usize, len: usize, name: &str) -> Result<usize> {
    offset
        .checked_add(len)
        .ok_or_else(|| ForgeError::ShapeMismatch {
            expected: vec![usize::MAX],
            got: vec![offset, len],
            remediation: format!("{name} offset+length overflows usize"),
        })
}
