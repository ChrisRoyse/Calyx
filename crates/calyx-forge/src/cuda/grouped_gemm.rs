use cudarc::cublas::{CudaBlas, result::CublasError, sys};
use cudarc::driver::{CudaSlice, DevicePtr, DevicePtrMut};

use crate::cpu::check_finite;
use crate::{CudaContext, ForgeError, Result};

const GROUPED_REMEDIATION: &str =
    "Validate grouped GEMM slab offsets, dimensions, and cuBLAS grouped support";
const DEVICE_REMEDIATION: &str = "Check CUDA/cuBLAS grouped GEMM support and CUDA GPU availability";
pub(crate) const ABSENT_SENTINEL: f32 = f32::NAN;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GemmProblem {
    pub m: usize,
    pub k: usize,
    pub n: usize,
    pub a_offset: usize,
    pub b_offset: usize,
    pub c_offset: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActiveGemmProblem {
    pub slot_idx: usize,
    pub problem: GemmProblem,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AbsentSlotSentinel {
    pub flat_idx: usize,
    pub c_offset: usize,
    pub len: usize,
}

pub struct GroupedGemmPlan {
    pub problems: Vec<Option<GemmProblem>>,
    pub slot_ids: Vec<Option<usize>>,
    pub absent_sentinel_ranges: Vec<AbsentSlotSentinel>,
    pub active: Vec<ActiveGemmProblem>,
    pub execution_mode: GroupedGemmExecutionMode,
    pub a_slab: CudaSlice<f32>,
    pub b_slab: CudaSlice<f32>,
    pub c_slab: CudaSlice<f32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupedGemmExecutionMode {
    NotRun,
    NoActiveProblems,
    GroupedBatched,
    SequentialFallback,
}

impl GroupedGemmExecutionMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotRun => "not_run",
            Self::NoActiveProblems => "no_active_problems",
            Self::GroupedBatched => "grouped_batched",
            Self::SequentialFallback => "sequential_fallback",
        }
    }
}

pub fn build_grouped_gemm_plan(
    ctx: &CudaContext,
    problems: Vec<Option<GemmProblem>>,
    a_host: &[f32],
    b_host: &[f32],
    c_init: &[f32],
) -> Result<GroupedGemmPlan> {
    let slot_ids = default_slot_ids(&problems);
    build_grouped_gemm_plan_with_metadata(
        ctx,
        problems,
        slot_ids,
        Vec::new(),
        a_host,
        b_host,
        c_init,
    )
}

pub(crate) fn build_grouped_gemm_plan_with_metadata(
    ctx: &CudaContext,
    problems: Vec<Option<GemmProblem>>,
    slot_ids: Vec<Option<usize>>,
    absent_sentinel_ranges: Vec<AbsentSlotSentinel>,
    a_host: &[f32],
    b_host: &[f32],
    c_init: &[f32],
) -> Result<GroupedGemmPlan> {
    check_finite(a_host, "grouped_gemm A slab")?;
    check_finite(b_host, "grouped_gemm B slab")?;
    if absent_sentinel_ranges.is_empty() {
        check_finite(c_init, "grouped_gemm C slab")?;
    }
    let active = sorted_active(&problems, a_host.len(), b_host.len(), c_init.len())?;
    let stream = ctx.inner().default_stream();
    Ok(GroupedGemmPlan {
        problems,
        slot_ids,
        absent_sentinel_ranges,
        active,
        execution_mode: GroupedGemmExecutionMode::NotRun,
        a_slab: stream
            .clone_htod(a_host)
            .map_err(|err| device_unavailable(ctx, format!("copy grouped A slab failed: {err}")))?,
        b_slab: stream
            .clone_htod(b_host)
            .map_err(|err| device_unavailable(ctx, format!("copy grouped B slab failed: {err}")))?,
        c_slab: stream
            .clone_htod(c_init)
            .map_err(|err| device_unavailable(ctx, format!("copy grouped C slab failed: {err}")))?,
    })
}

pub fn execute_grouped_gemm(ctx: &CudaContext, plan: &mut GroupedGemmPlan) -> Result<()> {
    execute_grouped_gemm_with_policy(ctx, plan, FallbackPolicy::AllowSequential)
}

pub fn execute_grouped_gemm_strict(ctx: &CudaContext, plan: &mut GroupedGemmPlan) -> Result<()> {
    execute_grouped_gemm_with_policy(ctx, plan, FallbackPolicy::FailIfGroupedUnsupported)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FallbackPolicy {
    AllowSequential,
    FailIfGroupedUnsupported,
}

fn execute_grouped_gemm_with_policy(
    ctx: &CudaContext,
    plan: &mut GroupedGemmPlan,
    policy: FallbackPolicy,
) -> Result<()> {
    plan.execution_mode = GroupedGemmExecutionMode::NotRun;
    if plan.active.is_empty() {
        plan.execution_mode = GroupedGemmExecutionMode::NoActiveProblems;
        return check_device_output(ctx, plan);
    }
    validate_active_again(plan)?;
    let stream = ctx.inner().default_stream();
    let blas = CudaBlas::new(stream.clone())
        .map_err(|err| device_unavailable(ctx, format!("cuBLAS grouped handle failed: {err}")))?;
    {
        let (a_base, _a_guard) = plan.a_slab.device_ptr(&stream);
        let (b_base, _b_guard) = plan.b_slab.device_ptr(&stream);
        let (c_base, _c_guard) = plan.c_slab.device_ptr_mut(&stream);
        let launch = LaunchData::new(&plan.active, a_base, b_base, c_base)?;
        let group_count = to_i32(launch.group_sizes.len(), "group_count")?;
        let grouped = launch_grouped(*blas.handle(), &launch, group_count);
        if let Err(err) = grouped {
            if err.0 != sys::cublasStatus_t::CUBLAS_STATUS_NOT_SUPPORTED {
                return Err(cublas_error(format!(
                    "cublasSgemmGroupedBatched failed: {err}"
                )));
            }
            if policy == FallbackPolicy::FailIfGroupedUnsupported {
                return Err(grouped_unsupported_error(err));
            }
            launch_sequential(*blas.handle(), &launch)?;
            plan.execution_mode = GroupedGemmExecutionMode::SequentialFallback;
        } else {
            plan.execution_mode = GroupedGemmExecutionMode::GroupedBatched;
        }
        stream
            .synchronize()
            .map_err(|err| device_unavailable(ctx, format!("grouped GEMM sync failed: {err}")))?;
    }
    check_device_output(ctx, plan)
}

pub fn read_grouped_gemm_output(ctx: &CudaContext, plan: &GroupedGemmPlan) -> Result<Vec<f32>> {
    ctx.inner()
        .default_stream()
        .clone_dtoh(&plan.c_slab)
        .map_err(|err| device_unavailable(ctx, format!("read grouped C slab failed: {err}")))
}

fn default_slot_ids(problems: &[Option<GemmProblem>]) -> Vec<Option<usize>> {
    problems
        .iter()
        .enumerate()
        .map(|(idx, problem)| problem.as_ref().map(|_| idx))
        .collect()
}

fn sorted_active(
    problems: &[Option<GemmProblem>],
    a_len: usize,
    b_len: usize,
    c_len: usize,
) -> Result<Vec<ActiveGemmProblem>> {
    let mut active = Vec::new();
    for (slot_idx, problem) in problems.iter().enumerate() {
        if let Some(problem) = problem {
            validate_problem(problem, a_len, b_len, c_len)?;
            active.push(ActiveGemmProblem {
                slot_idx,
                problem: *problem,
            });
        }
    }
    active.sort_by_key(|item| {
        (
            item.problem.k,
            item.problem.n,
            item.problem.m,
            item.slot_idx,
        )
    });
    Ok(active)
}

fn validate_problem(problem: &GemmProblem, a_len: usize, b_len: usize, c_len: usize) -> Result<()> {
    let a_need = matrix_len(problem.m, problem.k, "grouped A")?;
    let b_need = matrix_len(problem.k, problem.n, "grouped B")?;
    let c_need = matrix_len(problem.m, problem.n, "grouped C")?;
    checked_range(problem.a_offset, a_need, a_len, "grouped A slab")?;
    checked_range(problem.b_offset, b_need, b_len, "grouped B slab")?;
    checked_range(problem.c_offset, c_need, c_len, "grouped C slab")?;
    to_i32(problem.m, "m")?;
    to_i32(problem.k, "k")?;
    to_i32(problem.n, "n")?;
    Ok(())
}

fn validate_active_again(plan: &GroupedGemmPlan) -> Result<()> {
    for item in &plan.active {
        validate_problem(
            &item.problem,
            plan.a_slab.len(),
            plan.b_slab.len(),
            plan.c_slab.len(),
        )?;
    }
    Ok(())
}

struct LaunchData {
    trans: Vec<sys::cublasOperation_t>,
    ms: Vec<i32>,
    ns: Vec<i32>,
    ks: Vec<i32>,
    alphas: Vec<f32>,
    betas: Vec<f32>,
    ldas: Vec<i32>,
    ldbs: Vec<i32>,
    ldcs: Vec<i32>,
    group_sizes: Vec<i32>,
    a_ptrs: Vec<*const f32>,
    b_ptrs: Vec<*const f32>,
    c_ptrs: Vec<*mut f32>,
    active: Vec<GemmProblem>,
}

impl LaunchData {
    fn new(active: &[ActiveGemmProblem], a_base: u64, b_base: u64, c_base: u64) -> Result<Self> {
        let mut data = Self::empty();
        let mut current_dims = None;
        for item in active {
            let problem = item.problem;
            let dims = (problem.m, problem.k, problem.n);
            if current_dims != Some(dims) {
                data.push_group(problem)?;
                current_dims = Some(dims);
            }
            *data.group_sizes.last_mut().expect("group exists") += 1;
            data.a_ptrs.push(offset_ptr(a_base, problem.a_offset));
            data.b_ptrs.push(offset_ptr(b_base, problem.b_offset));
            data.c_ptrs.push(offset_mut_ptr(c_base, problem.c_offset));
            data.active.push(problem);
        }
        Ok(data)
    }

    fn empty() -> Self {
        Self {
            trans: Vec::new(),
            ms: Vec::new(),
            ns: Vec::new(),
            ks: Vec::new(),
            alphas: Vec::new(),
            betas: Vec::new(),
            ldas: Vec::new(),
            ldbs: Vec::new(),
            ldcs: Vec::new(),
            group_sizes: Vec::new(),
            a_ptrs: Vec::new(),
            b_ptrs: Vec::new(),
            c_ptrs: Vec::new(),
            active: Vec::new(),
        }
    }

    fn push_group(&mut self, problem: GemmProblem) -> Result<()> {
        self.trans.push(sys::cublasOperation_t::CUBLAS_OP_N);
        self.ms.push(to_i32(problem.m, "m")?);
        self.ns.push(to_i32(problem.n, "n")?);
        self.ks.push(to_i32(problem.k, "k")?);
        self.alphas.push(1.0);
        self.betas.push(0.0);
        self.ldas.push(to_i32(problem.m, "lda")?);
        self.ldbs.push(to_i32(problem.k, "ldb")?);
        self.ldcs.push(to_i32(problem.m, "ldc")?);
        self.group_sizes.push(0);
        Ok(())
    }
}

fn launch_grouped(
    handle: sys::cublasHandle_t,
    data: &LaunchData,
    group_count: i32,
) -> std::result::Result<(), CublasError> {
    unsafe {
        sys::cublasSgemmGroupedBatched(
            handle,
            data.trans.as_ptr(),
            data.trans.as_ptr(),
            data.ms.as_ptr(),
            data.ns.as_ptr(),
            data.ks.as_ptr(),
            data.alphas.as_ptr(),
            data.a_ptrs.as_ptr(),
            data.ldas.as_ptr(),
            data.b_ptrs.as_ptr(),
            data.ldbs.as_ptr(),
            data.betas.as_ptr(),
            data.c_ptrs.as_ptr(),
            data.ldcs.as_ptr(),
            group_count,
            data.group_sizes.as_ptr(),
        )
        .result()
    }
}

fn launch_sequential(handle: sys::cublasHandle_t, data: &LaunchData) -> Result<()> {
    for (idx, problem) in data.active.iter().enumerate() {
        unsafe {
            sys::cublasSgemm_v2(
                handle,
                sys::cublasOperation_t::CUBLAS_OP_N,
                sys::cublasOperation_t::CUBLAS_OP_N,
                to_i32(problem.m, "m")?,
                to_i32(problem.n, "n")?,
                to_i32(problem.k, "k")?,
                &1.0,
                data.a_ptrs[idx],
                to_i32(problem.m, "lda")?,
                data.b_ptrs[idx],
                to_i32(problem.k, "ldb")?,
                &0.0,
                data.c_ptrs[idx],
                to_i32(problem.m, "ldc")?,
            )
            .result()
        }
        .map_err(|err| cublas_error(format!("sequential cublasSgemm_v2 failed: {err}")))?;
    }
    Ok(())
}

fn check_device_output(ctx: &CudaContext, plan: &GroupedGemmPlan) -> Result<()> {
    let values = read_device(ctx, &plan.c_slab)?;
    for item in &plan.active {
        let problem = item.problem;
        let start = problem.c_offset;
        let end = start + problem.m * problem.n;
        for (rel_idx, value) in values[start..end].iter().enumerate() {
            if !value.is_finite() {
                return Err(ForgeError::NumericalInvariant {
                    op: "execute_grouped_gemm".to_string(),
                    detail: format!(
                        "non-finite active output at slot {} index {}: {}",
                        item.slot_idx, rel_idx, value
                    ),
                    remediation: GROUPED_REMEDIATION.to_string(),
                });
            }
        }
    }
    check_absent_sentinels(&values, &plan.absent_sentinel_ranges)
}

fn check_absent_sentinels(values: &[f32], sentinels: &[AbsentSlotSentinel]) -> Result<()> {
    for sentinel in sentinels {
        let end = checked_end(sentinel.c_offset, sentinel.len, "absent sentinel C slab")?;
        if end > values.len() {
            return Err(ForgeError::ShapeMismatch {
                expected: vec![values.len()],
                got: vec![sentinel.c_offset, sentinel.len],
                remediation: GROUPED_REMEDIATION.to_string(),
            });
        }
        for (idx, value) in values[sentinel.c_offset..end].iter().enumerate() {
            if value.to_bits() != ABSENT_SENTINEL.to_bits() {
                return Err(ForgeError::NumericalInvariant {
                    op: "execute_grouped_gemm".to_string(),
                    detail: format!(
                        "absent slot {} output was written at relative index {}",
                        sentinel.flat_idx, idx
                    ),
                    remediation: GROUPED_REMEDIATION.to_string(),
                });
            }
        }
    }
    Ok(())
}

fn read_device(ctx: &CudaContext, out: &CudaSlice<f32>) -> Result<Vec<f32>> {
    ctx.inner()
        .default_stream()
        .clone_dtoh(out)
        .map_err(|err| device_unavailable(ctx, format!("read grouped output failed: {err}")))
}

fn offset_ptr(base: u64, offset: usize) -> *const f32 {
    (base + byte_offset(offset)) as *const f32
}

fn offset_mut_ptr(base: u64, offset: usize) -> *mut f32 {
    (base + byte_offset(offset)) as *mut f32
}

fn byte_offset(offset: usize) -> u64 {
    (offset * std::mem::size_of::<f32>()) as u64
}

fn matrix_len(rows: usize, cols: usize, name: &str) -> Result<usize> {
    rows.checked_mul(cols)
        .ok_or_else(|| ForgeError::ShapeMismatch {
            expected: vec![rows, cols],
            got: vec![usize::MAX],
            remediation: format!("{name} shape overflows usize"),
        })
}

fn checked_range(offset: usize, len: usize, total: usize, name: &str) -> Result<()> {
    let end = checked_end(offset, len, name)?;
    if end <= total {
        Ok(())
    } else {
        Err(ForgeError::ShapeMismatch {
            expected: vec![total],
            got: vec![offset, len],
            remediation: format!("{name} offset+length exceeds slab length"),
        })
    }
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

fn to_i32(value: usize, name: &str) -> Result<i32> {
    i32::try_from(value).map_err(|_| ForgeError::ShapeMismatch {
        expected: vec![i32::MAX as usize],
        got: vec![value],
        remediation: format!("grouped GEMM {name} exceeds cuBLAS i32 limit"),
    })
}

fn cublas_error(detail: String) -> ForgeError {
    ForgeError::NumericalInvariant {
        op: "execute_grouped_gemm".to_string(),
        detail,
        remediation: GROUPED_REMEDIATION.to_string(),
    }
}

fn grouped_unsupported_error(err: CublasError) -> ForgeError {
    ForgeError::NumericalInvariant {
        op: "execute_grouped_gemm_strict".to_string(),
        detail: format!(
            "cublasSgemmGroupedBatched unsupported and strict grouped launch requested: {err}"
        ),
        remediation: DEVICE_REMEDIATION.to_string(),
    }
}

fn device_unavailable(ctx: &CudaContext, detail: String) -> ForgeError {
    ForgeError::DeviceUnavailable {
        device: format!("cuda:{}", ctx.device_idx()),
        detail,
        remediation: DEVICE_REMEDIATION.to_string(),
    }
}
