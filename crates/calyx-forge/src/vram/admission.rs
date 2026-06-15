//! Admission control for large Forge VRAM dispatches.
//!
//! The controller is the coordination layer over the T01 budgeter and T02 LRU
//! registry: try immediate execution, evict and retry, split into smaller
//! batches, queue bounded work with a deadline, or fail closed with
//! `CALYX_FORGE_VRAM_BUDGET`.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::vram::{
    BlockDeallocator, GpuBlockRegistry, RESERVED_HEADROOM_BYTES, VRAM_BUDGET_REMEDIATION,
    VramBudgeter, VramGuard, VramProbe,
};
use crate::{ForgeError, Result};

pub const LENS_VRAM_BUDGET_REMEDIATION: &str = "Lower lens precision, move the lens to CPU, evict cold GPU lenses, or raise CALYX_FORGE_VRAM_BUDGET";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LensAdmissionPlacement {
    Cpu,
    Gpu,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LensAdmissionRequest {
    pub lens_vram_bytes: usize,
    pub tei_reserved_bytes: usize,
    pub allow_cpu_fallback: bool,
}

pub struct LensAdmission<'b, P: VramProbe> {
    pub placement: LensAdmissionPlacement,
    pub requested_vram_bytes: usize,
    pub available_vram_bytes: usize,
    pub guard: Option<VramGuard<'b, P>>,
}

pub fn admit_lens<'b, P: VramProbe>(
    budgeter: &'b VramBudgeter<P>,
    request: LensAdmissionRequest,
) -> Result<LensAdmission<'b, P>> {
    if request.lens_vram_bytes == 0 {
        return Ok(LensAdmission {
            placement: LensAdmissionPlacement::Gpu,
            requested_vram_bytes: 0,
            available_vram_bytes: available_after_tei(budgeter, request.tei_reserved_bytes),
            guard: None,
        });
    }

    let available = available_after_tei(budgeter, request.tei_reserved_bytes);
    if request.lens_vram_bytes <= available {
        return Ok(LensAdmission {
            placement: LensAdmissionPlacement::Gpu,
            requested_vram_bytes: request.lens_vram_bytes,
            available_vram_bytes: available,
            guard: Some(budgeter.reserve(request.lens_vram_bytes)?),
        });
    }

    if request.allow_cpu_fallback {
        return Ok(LensAdmission {
            placement: LensAdmissionPlacement::Cpu,
            requested_vram_bytes: request.lens_vram_bytes,
            available_vram_bytes: available,
            guard: None,
        });
    }

    Err(ForgeError::LensVramBudget {
        detail: format!(
            "lens_vram_bytes={} available_after_tei={} tei_reserved_bytes={}",
            request.lens_vram_bytes, available, request.tei_reserved_bytes
        ),
        remediation: LENS_VRAM_BUDGET_REMEDIATION.to_string(),
    })
}

fn available_after_tei<P: VramProbe>(
    budgeter: &VramBudgeter<P>,
    tei_reserved_bytes: usize,
) -> usize {
    let stats = budgeter.stats();
    let soft_available = stats
        .soft_cap_bytes
        .saturating_sub(stats.allocated_bytes)
        .saturating_sub(tei_reserved_bytes);
    let device_available = stats
        .device_free_bytes
        .saturating_sub(RESERVED_HEADROOM_BYTES)
        .saturating_sub(tei_reserved_bytes);
    soft_available.min(device_available)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmitDecision {
    Split { sub_batch_size: usize },
    Queue { deadline: Instant },
    Fail,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueuedDispatch {
    pub requested_bytes: usize,
    pub batch_size: usize,
    pub deadline: Instant,
    pub enqueued_at: Instant,
}

pub trait AdmissionOutput: Sized {
    fn merge(parts: Vec<Self>) -> Self;
}

impl AdmissionOutput for () {
    fn merge(_parts: Vec<Self>) -> Self {}
}

impl<T> AdmissionOutput for Vec<T> {
    fn merge(parts: Vec<Self>) -> Self {
        parts.into_iter().flatten().collect()
    }
}

pub struct AdmissionController<'b, P: VramProbe, D: BlockDeallocator> {
    budgeter: &'b VramBudgeter<P>,
    registry: Arc<Mutex<GpuBlockRegistry<'b, P, D>>>,
    queue: Mutex<VecDeque<QueuedDispatch>>,
    queue_cap: usize,
    split_min_batch: usize,
}

impl<'b, P: VramProbe, D: BlockDeallocator> AdmissionController<'b, P, D> {
    pub fn new(
        budgeter: &'b VramBudgeter<P>,
        registry: Arc<Mutex<GpuBlockRegistry<'b, P, D>>>,
        queue_cap: usize,
        split_min_batch: usize,
    ) -> Self {
        Self {
            budgeter,
            registry,
            queue: Mutex::new(VecDeque::with_capacity(queue_cap)),
            queue_cap,
            split_min_batch: split_min_batch.max(1),
        }
    }

    pub fn decide(
        &self,
        requested_bytes: usize,
        batch_size: usize,
        deadline: Instant,
    ) -> AdmitDecision {
        if requested_bytes == 0 {
            return self.split(batch_size);
        }
        if batch_size == 0 {
            return self.fail();
        }
        if deadline <= Instant::now() {
            return self.fail();
        }
        if self.budgeter.can_allocate(requested_bytes).is_ok() {
            return self.split(batch_size);
        }
        if self.evict_then_can_allocate(requested_bytes) {
            return self.split(batch_size);
        }
        if let Some(sub_batch_size) = self.next_split(batch_size) {
            return self.split(sub_batch_size);
        }
        self.queue_or_fail(requested_bytes, batch_size, deadline)
    }

    pub fn run_with_admission<F, R>(
        &self,
        bytes: usize,
        batch: usize,
        deadline: Instant,
        mut f: F,
    ) -> Result<R>
    where
        F: FnMut(usize, usize) -> Result<R>,
        R: AdmissionOutput,
    {
        self.run_range(bytes, batch, 0, deadline, &mut f)
    }

    pub fn queue_len(&self) -> usize {
        self.queue.lock().map(|queue| queue.len()).unwrap_or(0)
    }

    pub fn queued_snapshot(&self) -> Vec<QueuedDispatch> {
        self.queue
            .lock()
            .map(|queue| queue.iter().copied().collect())
            .unwrap_or_default()
    }

    fn run_range<F, R>(
        &self,
        bytes: usize,
        batch: usize,
        offset: usize,
        deadline: Instant,
        f: &mut F,
    ) -> Result<R>
    where
        F: FnMut(usize, usize) -> Result<R>,
        R: AdmissionOutput,
    {
        match self.decide(bytes, batch, deadline) {
            AdmitDecision::Split { sub_batch_size } if sub_batch_size >= batch => {
                let _guard = self.budgeter.reserve(bytes)?;
                f(offset, batch)
            }
            AdmitDecision::Split { sub_batch_size } => {
                let left_batch = sub_batch_size;
                let right_batch = batch - left_batch;
                let left_bytes = proportional_bytes(bytes, batch, left_batch);
                let right_bytes = bytes.saturating_sub(left_bytes);
                let left = self.run_range(left_bytes, left_batch, offset, deadline, f)?;
                let right =
                    self.run_range(right_bytes, right_batch, offset + left_batch, deadline, f)?;
                Ok(R::merge(vec![left, right]))
            }
            AdmitDecision::Queue { .. } => Err(self.budget_error(
                bytes,
                batch,
                "dispatch queued; synchronous run_with_admission has no completed result",
            )),
            AdmitDecision::Fail => Err(self.budget_error(bytes, batch, "admission failed closed")),
        }
    }

    fn evict_then_can_allocate(&self, requested_bytes: usize) -> bool {
        let Ok(mut registry) = self.registry.lock() else {
            return false;
        };
        registry.evict_until(requested_bytes).is_ok()
            && self.budgeter.can_allocate(requested_bytes).is_ok()
    }

    fn next_split(&self, batch_size: usize) -> Option<usize> {
        if batch_size <= self.split_min_batch {
            return None;
        }
        let sub_batch_size = (batch_size / 2).max(self.split_min_batch);
        (sub_batch_size < batch_size).then_some(sub_batch_size)
    }

    fn queue_or_fail(
        &self,
        requested_bytes: usize,
        batch_size: usize,
        deadline: Instant,
    ) -> AdmitDecision {
        let Ok(mut queue) = self.queue.lock() else {
            return self.fail();
        };
        if queue.len() >= self.queue_cap {
            return self.fail();
        }
        queue.push_back(QueuedDispatch {
            requested_bytes,
            batch_size,
            deadline,
            enqueued_at: Instant::now(),
        });
        self.budgeter.record_admission_queued();
        AdmitDecision::Queue { deadline }
    }

    fn split(&self, sub_batch_size: usize) -> AdmitDecision {
        self.budgeter.record_admission_split();
        AdmitDecision::Split { sub_batch_size }
    }

    fn fail(&self) -> AdmitDecision {
        self.budgeter.record_admission_failed();
        AdmitDecision::Fail
    }

    fn budget_error(&self, requested_bytes: usize, batch_size: usize, reason: &str) -> ForgeError {
        let stats = self.budgeter.stats();
        let soft_available = stats.soft_cap_bytes.saturating_sub(stats.allocated_bytes);
        let device_available = stats
            .device_free_bytes
            .saturating_sub(RESERVED_HEADROOM_BYTES);
        ForgeError::VramBudget {
            detail: format!(
                "{reason}: requested_bytes={requested_bytes} available_bytes={} budget_bytes={} allocated_bytes={} device_free_bytes={} batch_size={batch_size}",
                soft_available.min(device_available),
                stats.soft_cap_bytes,
                stats.allocated_bytes,
                stats.device_free_bytes
            ),
            remediation: VRAM_BUDGET_REMEDIATION.to_string(),
        }
    }
}

fn proportional_bytes(total_bytes: usize, total_batch: usize, sub_batch: usize) -> usize {
    if total_batch == 0 {
        return 0;
    }
    let per_item = total_bytes / total_batch;
    let remainder = total_bytes % total_batch;
    per_item
        .saturating_mul(sub_batch)
        .saturating_add(remainder.min(sub_batch))
}

#[cfg(test)]
#[path = "admission_tests.rs"]
mod tests;
