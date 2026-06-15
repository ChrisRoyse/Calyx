use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::vram::{BlockId, BlockKind, DevicePtr, GpuBlockRegistry};

const MIB: usize = 1024 * 1024;
const GIB: usize = 1024 * 1024 * 1024;
const CODE: &str = "CALYX_FORGE_VRAM_BUDGET";

struct StaticProbe {
    free: usize,
}

impl VramProbe for StaticProbe {
    fn free_device_vram(&self) -> Result<usize> {
        Ok(self.free)
    }
}

#[derive(Clone, Default)]
struct RecordingDealloc {
    freed: Arc<Mutex<Vec<(u64, usize)>>>,
}

impl RecordingDealloc {
    fn freed(&self) -> Vec<(u64, usize)> {
        self.freed.lock().unwrap().clone()
    }
}

impl BlockDeallocator for RecordingDealloc {
    fn free(&self, ptr: DevicePtr, size_bytes: usize) -> Result<()> {
        self.freed.lock().unwrap().push((ptr.0, size_bytes));
        Ok(())
    }
}

fn controller<'b>(
    budgeter: &'b VramBudgeter<StaticProbe>,
    dealloc: RecordingDealloc,
    queue_cap: usize,
    split_min_batch: usize,
) -> AdmissionController<'b, StaticProbe, RecordingDealloc> {
    let registry = GpuBlockRegistry::new(budgeter, dealloc, 16);
    AdmissionController::new(
        budgeter,
        Arc::new(Mutex::new(registry)),
        queue_cap,
        split_min_batch,
    )
}

#[test]
fn fits_returns_full_batch_and_counts_split() {
    let budgeter = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 64 * GIB });
    let ctl = controller(&budgeter, RecordingDealloc::default(), 2, 2);

    let decision = ctl.decide(512 * MIB, 8, Instant::now() + Duration::from_secs(1));

    assert_eq!(decision, AdmitDecision::Split { sub_batch_size: 8 });
    let stats = budgeter.stats();
    assert_eq!(stats.splits_total, 1);
    assert_eq!(stats.queued_total, 0);
    assert_eq!(stats.failed_total, 0);
}

#[test]
fn eviction_allows_full_batch() {
    let budgeter = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 64 * GIB });
    let dealloc = RecordingDealloc::default();
    let registry = Arc::new(Mutex::new(GpuBlockRegistry::new(
        &budgeter,
        dealloc.clone(),
        16,
    )));
    let guard = budgeter.reserve(700 * MIB).expect("reserve resident block");
    registry.lock().unwrap().insert(
        BlockId(7),
        DevicePtr(0x7000),
        700 * MIB,
        BlockKind::General,
        guard,
    );
    let ctl = AdmissionController::new(&budgeter, Arc::clone(&registry), 2, 2);

    let decision = ctl.decide(512 * MIB, 8, Instant::now() + Duration::from_secs(1));

    assert_eq!(decision, AdmitDecision::Split { sub_batch_size: 8 });
    assert_eq!(dealloc.freed(), vec![(0x7000, 700 * MIB)]);
    assert_eq!(registry.lock().unwrap().stats().evictions_total, 1);
    assert_eq!(budgeter.allocated_bytes(), 0);
}

#[test]
fn oversized_request_splits_and_runner_assembles_all_items() {
    let budgeter = VramBudgeter::with_soft_cap(2 * MIB, StaticProbe { free: 64 * GIB });
    let ctl = controller(&budgeter, RecordingDealloc::default(), 0, 2);
    let mut calls = Vec::new();

    let out = ctl
        .run_with_admission(
            8 * MIB,
            8,
            Instant::now() + Duration::from_secs(1),
            |offset, len| {
                calls.push((offset, len));
                Ok((offset..offset + len).collect::<Vec<_>>())
            },
        )
        .expect("split run succeeds");

    assert_eq!(out, (0..8).collect::<Vec<_>>());
    assert_eq!(calls, vec![(0, 2), (2, 2), (4, 2), (6, 2)]);
    assert_eq!(budgeter.allocated_bytes(), 0);
    assert_eq!(budgeter.stats().splits_total, 7);
}

#[test]
fn queue_when_min_split_and_capacity() {
    let budgeter = VramBudgeter::with_soft_cap(MIB, StaticProbe { free: 64 * GIB });
    let ctl = controller(&budgeter, RecordingDealloc::default(), 1, 2);
    let deadline = Instant::now() + Duration::from_secs(1);

    let decision = ctl.decide(2 * MIB, 2, deadline);

    assert_eq!(decision, AdmitDecision::Queue { deadline });
    assert_eq!(ctl.queue_len(), 1);
    assert_eq!(ctl.queued_snapshot()[0].requested_bytes, 2 * MIB);
    assert_eq!(budgeter.stats().queued_total, 1);
}

#[test]
fn queue_full_fails_closed_from_runner() {
    let budgeter = VramBudgeter::with_soft_cap(MIB, StaticProbe { free: 64 * GIB });
    let ctl = controller(&budgeter, RecordingDealloc::default(), 1, 2);
    let deadline = Instant::now() + Duration::from_secs(1);

    assert_eq!(
        ctl.decide(2 * MIB, 2, deadline),
        AdmitDecision::Queue { deadline }
    );
    let err = ctl
        .run_with_admission(
            2 * MIB,
            2,
            deadline,
            |_offset, _len| Ok(Vec::<usize>::new()),
        )
        .expect_err("queue is full, so sync runner fails closed");

    assert_eq!(err.code(), CODE);
    let stats = budgeter.stats();
    assert_eq!(stats.queued_total, 1);
    assert_eq!(stats.failed_total, 1);
}

#[test]
fn past_deadline_fails_immediately() {
    let budgeter = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 64 * GIB });
    let ctl = controller(&budgeter, RecordingDealloc::default(), 1, 2);

    let decision = ctl.decide(512 * MIB, 8, Instant::now() - Duration::from_millis(1));

    assert_eq!(decision, AdmitDecision::Fail);
    assert_eq!(ctl.queue_len(), 0);
    assert_eq!(budgeter.stats().failed_total, 1);
}

#[test]
fn zero_bytes_admitted_even_with_past_deadline() {
    let budgeter = VramBudgeter::with_soft_cap(0, StaticProbe { free: 0 });
    let ctl = controller(&budgeter, RecordingDealloc::default(), 0, 2);

    let decision = ctl.decide(0, 8, Instant::now() - Duration::from_secs(1));

    assert_eq!(decision, AdmitDecision::Split { sub_batch_size: 8 });
    assert_eq!(budgeter.stats().splits_total, 1);
}

#[test]
fn nonzero_bytes_with_empty_batch_fails_closed() {
    let budgeter = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 64 * GIB });
    let ctl = controller(&budgeter, RecordingDealloc::default(), 1, 2);

    let decision = ctl.decide(MIB, 0, Instant::now() + Duration::from_secs(1));

    assert_eq!(decision, AdmitDecision::Fail);
    assert_eq!(ctl.queue_len(), 0);
    assert_eq!(budgeter.stats().failed_total, 1);
}

#[test]
fn lens_admission_reserves_until_post_tei_cap_then_fallback_or_refuse() {
    let budgeter = VramBudgeter::with_soft_cap(10 * MIB, StaticProbe { free: 64 * GIB });
    let request = LensAdmissionRequest {
        lens_vram_bytes: 2 * MIB,
        tei_reserved_bytes: 2 * MIB,
        allow_cpu_fallback: false,
    };
    let mut guards = Vec::new();

    for _ in 0..4 {
        let admission = admit_lens(&budgeter, request).expect("GPU lens fits post-TEI cap");
        assert_eq!(admission.placement, LensAdmissionPlacement::Gpu);
        assert_eq!(admission.requested_vram_bytes, 2 * MIB);
        guards.push(admission.guard.expect("GPU admission owns guard"));
    }
    assert_eq!(budgeter.allocated_bytes(), 8 * MIB);

    let fallback = admit_lens(
        &budgeter,
        LensAdmissionRequest {
            allow_cpu_fallback: true,
            ..request
        },
    )
    .expect("CPU fallback chosen after VRAM cap");
    assert_eq!(fallback.placement, LensAdmissionPlacement::Cpu);
    assert!(fallback.guard.is_none());

    let err = match admit_lens(&budgeter, request) {
        Ok(_) => panic!("no-fallback GPU lens should refuse"),
        Err(err) => err,
    };
    assert_eq!(err.code(), "CALYX_VRAM_BUDGET_EXCEEDED");
    assert!(err.remediation().contains("Lower lens precision"));

    drop(guards);
    assert_eq!(budgeter.allocated_bytes(), 0);
}

#[test]
fn zero_vram_lens_admits_without_reservation() {
    let budgeter = VramBudgeter::with_soft_cap(0, StaticProbe { free: 0 });

    let admission = admit_lens(
        &budgeter,
        LensAdmissionRequest {
            lens_vram_bytes: 0,
            tei_reserved_bytes: 8 * MIB,
            allow_cpu_fallback: false,
        },
    )
    .expect("zero VRAM lens admits");

    assert_eq!(admission.placement, LensAdmissionPlacement::Gpu);
    assert_eq!(admission.requested_vram_bytes, 0);
    assert!(admission.guard.is_none());
    assert_eq!(budgeter.allocated_bytes(), 0);
}

proptest::proptest! {
    #[test]
    fn decisions_are_total_and_counted(
        requests in proptest::collection::vec((0usize..8192, 0usize..32), 1..64),
    ) {
        let budgeter = VramBudgeter::with_soft_cap(4096, StaticProbe { free: usize::MAX });
        let ctl = controller(&budgeter, RecordingDealloc::default(), 128, 2);

        for (bytes, batch) in requests.iter().copied() {
            match ctl.decide(bytes, batch, Instant::now() + Duration::from_secs(1)) {
                AdmitDecision::Split { .. } | AdmitDecision::Queue { .. } | AdmitDecision::Fail => {}
            }
        }

        let stats = budgeter.stats();
        proptest::prop_assert_eq!(
            stats.splits_total + stats.queued_total + stats.failed_total,
            requests.len() as u64
        );
    }
}
