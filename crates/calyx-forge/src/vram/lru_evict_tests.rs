//! FSV for the GPU-block LRU eviction registry (PH57 · T02).
//!
//! Source of Truth: the registry's [`GpuBlockStats`] (resident_blocks /
//! resident_bytes / evictions_total) and the real [`VramBudgeter`]'s
//! `allocated_bytes()` — both read back *independently* after each action, not
//! trusted from return values. The deallocator is a real recorder, so we also
//! assert exactly which device pointers were physically freed. No mocked logic:
//! every reservation is a genuine budgeter guard.

use std::sync::{Arc, Mutex};

use super::*;
use crate::vram::{VramBudgeter, VramProbe};
use crate::{ForgeError, Result};

const MIB: usize = 1024 * 1024;
const GIB: usize = 1024 * 1024 * 1024;
const CODE: &str = "CALYX_FORGE_VRAM_BUDGET";

/// Deterministic free-VRAM probe (the T01 hardware boundary). Abundant free
/// VRAM so device-headroom never gates these soft-cap/eviction tests.
struct StaticProbe {
    free: usize,
}
impl VramProbe for StaticProbe {
    fn free_device_vram(&self) -> Result<usize> {
        Ok(self.free)
    }
}

/// Records every physical free as `(device_addr, size_bytes)`. Shared via `Arc`
/// so a test can read back exactly what was freed after eviction.
#[derive(Clone)]
struct RecordingDealloc {
    freed: Arc<Mutex<Vec<(u64, usize)>>>,
}
impl RecordingDealloc {
    fn new() -> Self {
        Self {
            freed: Arc::new(Mutex::new(Vec::new())),
        }
    }
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

/// Stands in for a `cudaFree` that returns `cudaErrorInvalidValue`. A real
/// hardware-boundary failure, not a mock of the eviction logic.
struct FailingDealloc;
impl BlockDeallocator for FailingDealloc {
    fn free(&self, _ptr: DevicePtr, _size: usize) -> Result<()> {
        Err(ForgeError::DeviceUnavailable {
            device: "test-gpu".into(),
            detail: "simulated cudaFree cudaErrorInvalidValue".into(),
            remediation: "n/a".into(),
        })
    }
}

/// Reserve `size` against the real budgeter and insert the block. The device
/// address is derived from `id` so we can assert exactly which block was freed.
fn ins<'b, P: VramProbe, D: BlockDeallocator>(
    reg: &mut GpuBlockRegistry<'b, P, D>,
    budgeter: &'b VramBudgeter<P>,
    id: u64,
    size: usize,
    kind: BlockKind,
) {
    let guard = budgeter.reserve(size).expect("budget reservation");
    reg.insert(BlockId(id), DevicePtr(0x1000 + id), size, kind, guard);
}

#[test]
fn insert_three_then_evict_until_evicts_lru_block() {
    // Synthetic known-I/O: A=1GiB, B=512MiB, C=256MiB under a 2GiB soft cap.
    let budgeter = VramBudgeter::with_soft_cap(2 * GIB, StaticProbe { free: 64 * GIB });
    let dealloc = RecordingDealloc::new();
    let mut reg = GpuBlockRegistry::new(&budgeter, dealloc.clone(), 16);

    ins(&mut reg, &budgeter, 1, GIB, BlockKind::General); // A (LRU)
    ins(&mut reg, &budgeter, 2, 512 * MIB, BlockKind::General); // B
    ins(&mut reg, &budgeter, 3, 256 * MIB, BlockKind::General); // C (MRU)

    // BEFORE: read the SoT independently.
    let before = reg.stats();
    println!(
        "BEFORE: {before:?} allocated={}",
        budgeter.allocated_bytes()
    );
    assert_eq!(before.resident_blocks, 3);
    assert_eq!(before.resident_bytes, 1792 * MIB); // 1024+512+256
    assert_eq!(budgeter.allocated_bytes(), 1792 * MIB);

    // TRIGGER: make room for D=512MiB. 1792+512=2304 > 2048 cap → evict LRU (A).
    reg.evict_until(512 * MIB).expect("eviction frees enough");
    let g_d = budgeter.reserve(512 * MIB).expect("D fits after eviction");
    reg.insert(
        BlockId(4),
        DevicePtr(0x2000),
        512 * MIB,
        BlockKind::General,
        g_d,
    );

    // AFTER: independent read of the SoT + the deallocator record.
    let after = reg.stats();
    println!(
        "AFTER: {after:?} allocated={} freed={:?}",
        budgeter.allocated_bytes(),
        dealloc.freed()
    );
    assert_eq!(after.evictions_total, 1, "exactly one eviction (A)");
    assert_eq!(after.resident_bytes, 1280 * MIB); // B+C+D = 512+256+512
    assert_eq!(after.resident_blocks, 3);
    assert_eq!(budgeter.allocated_bytes(), 1280 * MIB);
    // A (device addr 0x1001, 1GiB) is the block physically freed.
    assert_eq!(dealloc.freed(), vec![(0x1001, GIB)]);
    // A is gone; B/C/D resident.
    assert!(reg.get(&BlockId(1)).is_none());
    assert_eq!(reg.get(&BlockId(4)), Some(DevicePtr(0x2000)));
}

#[test]
fn touch_promotes_block_changing_eviction_victim() {
    let budgeter = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 64 * GIB });
    let dealloc = RecordingDealloc::new();
    let mut reg = GpuBlockRegistry::new(&budgeter, dealloc.clone(), 16);

    ins(&mut reg, &budgeter, 1, 100 * MIB, BlockKind::General); // A (LRU)
    ins(&mut reg, &budgeter, 2, 100 * MIB, BlockKind::General); // B
    ins(&mut reg, &budgeter, 3, 100 * MIB, BlockKind::General); // C (MRU)

    // Without touch the victim would be A. Promote A → victim becomes B.
    reg.touch(&BlockId(1));
    let freed = reg.evict_lru().expect("evict one");
    println!("evicted bytes={freed} freed={:?}", dealloc.freed());

    assert_eq!(freed, 100 * MIB);
    assert_eq!(dealloc.freed(), vec![(0x1002, 100 * MIB)]); // B freed, not A
    assert!(reg.get(&BlockId(2)).is_none()); // B gone
    assert_eq!(reg.get(&BlockId(1)), Some(DevicePtr(0x1001))); // A survived
}

#[test]
fn evict_until_fails_closed_when_registry_cannot_free_enough() {
    // soft_cap = 100MiB; registry holds exactly one 100MiB block; a 200MiB
    // request cannot be satisfied even after the registry is emptied.
    let budgeter = VramBudgeter::with_soft_cap(100 * MIB, StaticProbe { free: 64 * GIB });
    let dealloc = RecordingDealloc::new();
    let mut reg = GpuBlockRegistry::new(&budgeter, dealloc.clone(), 16);
    ins(&mut reg, &budgeter, 1, 100 * MIB, BlockKind::General);

    println!(
        "BEFORE: allocated={} stats={:?}",
        budgeter.allocated_bytes(),
        reg.stats()
    );
    let err = reg
        .evict_until(200 * MIB)
        .expect_err("200MiB cannot fit under a 100MiB cap");
    println!(
        "AFTER fail: code={} allocated={} stats={:?}",
        err.code(),
        budgeter.allocated_bytes(),
        reg.stats()
    );

    assert_eq!(err.code(), CODE);
    // The one block WAS evicted before failing closed (registry emptied).
    assert_eq!(reg.stats().resident_blocks, 0);
    assert_eq!(reg.stats().evictions_total, 1);
    assert_eq!(budgeter.allocated_bytes(), 0);
    assert_eq!(dealloc.freed(), vec![(0x1001, 100 * MIB)]);
}

#[test]
fn evict_lru_on_empty_returns_none_without_panic() {
    let budgeter = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 64 * GIB });
    let mut reg = GpuBlockRegistry::new(&budgeter, RecordingDealloc::new(), 16);
    assert_eq!(reg.evict_lru(), None);
    assert_eq!(reg.stats().resident_blocks, 0);
    assert_eq!(reg.stats().evictions_total, 0);
}

#[test]
fn frontier_cap_evicts_oldest_frontier_before_general() {
    // max_frontier_blocks = 2. Insert F1, F2 (frontier), G1 (general), then F3
    // (frontier) → F1 (oldest frontier) is evicted; G1 is untouched.
    let budgeter = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 64 * GIB });
    let dealloc = RecordingDealloc::new();
    let mut reg = GpuBlockRegistry::new(&budgeter, dealloc.clone(), 2);

    ins(&mut reg, &budgeter, 1, 10 * MIB, BlockKind::Frontier); // F1 (oldest frontier)
    ins(&mut reg, &budgeter, 2, 10 * MIB, BlockKind::Frontier); // F2
    ins(&mut reg, &budgeter, 3, 10 * MIB, BlockKind::General); // G1
    println!(
        "BEFORE F3: frontier={} stats={:?}",
        reg.frontier_count(),
        reg.stats()
    );

    ins(&mut reg, &budgeter, 4, 10 * MIB, BlockKind::Frontier); // F3 → evicts F1
    println!(
        "AFTER F3: frontier={} freed={:?}",
        reg.frontier_count(),
        dealloc.freed()
    );

    assert_eq!(dealloc.freed(), vec![(0x1001, 10 * MIB)]); // F1 freed
    assert!(reg.get(&BlockId(1)).is_none()); // F1 gone
    assert_eq!(reg.frontier_count(), 2); // F2 + F3
    assert_eq!(reg.get(&BlockId(3)), Some(DevicePtr(0x1003))); // G1 untouched
    assert_eq!(reg.stats().resident_blocks, 3); // F2, G1, F3
}

#[test]
fn zero_size_block_does_not_count_against_budget() {
    let budgeter = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 64 * GIB });
    let dealloc = RecordingDealloc::new();
    let mut reg = GpuBlockRegistry::new(&budgeter, dealloc.clone(), 16);

    ins(&mut reg, &budgeter, 1, 0, BlockKind::General);
    assert_eq!(reg.stats().resident_bytes, 0);
    assert_eq!(reg.stats().resident_blocks, 1);
    assert_eq!(budgeter.allocated_bytes(), 0);

    // Eviction of a zero-size block frees 0 bytes (no-op on size), still counts.
    assert_eq!(reg.evict_lru(), Some(0));
    assert_eq!(reg.stats().evictions_total, 1);
    assert_eq!(dealloc.freed(), vec![(0x1001, 0)]);
}

#[test]
fn dealloc_failure_logs_but_still_reclaims_budget() {
    // Fail-closed proof: a cudaFree failure must NOT panic and must NOT leak the
    // budget reservation — the mapping is gone from Forge's registry regardless.
    let budgeter = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 64 * GIB });
    let mut reg = GpuBlockRegistry::new(&budgeter, FailingDealloc, 16);
    ins(&mut reg, &budgeter, 1, 100 * MIB, BlockKind::General);
    assert_eq!(budgeter.allocated_bytes(), 100 * MIB);

    let freed = reg
        .evict_lru()
        .expect("eviction proceeds despite dealloc error");
    println!(
        "evicted={freed} allocated_after={}",
        budgeter.allocated_bytes()
    );

    assert_eq!(freed, 100 * MIB);
    assert_eq!(reg.stats().resident_blocks, 0); // block removed
    assert_eq!(reg.stats().evictions_total, 1);
    assert_eq!(budgeter.allocated_bytes(), 0); // budget reclaimed, not leaked
}

#[test]
fn reinsert_existing_id_evicts_prior_block_no_leak() {
    // Re-inserting an id must evict the prior block (release its guard) — no
    // silent guard/mapping leak.
    let budgeter = VramBudgeter::with_soft_cap(GIB, StaticProbe { free: 64 * GIB });
    let dealloc = RecordingDealloc::new();
    let mut reg = GpuBlockRegistry::new(&budgeter, dealloc.clone(), 16);

    ins(&mut reg, &budgeter, 1, 100 * MIB, BlockKind::General);
    assert_eq!(budgeter.allocated_bytes(), 100 * MIB);
    // Re-insert id=1 with a fresh 50MiB reservation.
    let g = budgeter.reserve(50 * MIB).expect("reserve");
    reg.insert(
        BlockId(1),
        DevicePtr(0x9999),
        50 * MIB,
        BlockKind::General,
        g,
    );

    assert_eq!(reg.stats().resident_blocks, 1);
    assert_eq!(reg.stats().resident_bytes, 50 * MIB);
    assert_eq!(budgeter.allocated_bytes(), 50 * MIB); // prior 100MiB released
    assert_eq!(dealloc.freed(), vec![(0x1001, 100 * MIB)]); // prior block freed
    assert_eq!(reg.get(&BlockId(1)), Some(DevicePtr(0x9999)));
}

proptest::proptest! {
    /// Invariant: after any sequence of admit-then-insert / get operations, the
    /// registry's resident_bytes never exceeds the soft cap (and never exceeds
    /// the budgeter's reserved total, which the budgeter itself caps).
    #[test]
    fn resident_bytes_never_exceeds_soft_cap(
        soft_cap in 1usize..=4096,
        ops in proptest::collection::vec((0u64..8, 1usize..=512, proptest::bool::ANY), 1..40),
    ) {
        let budgeter = VramBudgeter::with_soft_cap(soft_cap, StaticProbe { free: usize::MAX });
        let dealloc = RecordingDealloc::new();
        let mut reg = GpuBlockRegistry::new(&budgeter, dealloc, 4);

        for (id, size, is_get) in ops {
            if is_get {
                reg.get(&BlockId(id));
            } else {
                // Admit: evict to make room, then reserve a real guard.
                let _ = reg.evict_until(size);
                if let Ok(guard) = budgeter.reserve(size) {
                    reg.insert(BlockId(id), DevicePtr(0x1000 + id), size, BlockKind::General, guard);
                }
            }
            let stats = reg.stats();
            proptest::prop_assert!(stats.resident_bytes <= soft_cap);
            proptest::prop_assert!(budgeter.allocated_bytes() <= soft_cap);
            proptest::prop_assert!(stats.resident_bytes <= budgeter.allocated_bytes());
        }
    }
}
