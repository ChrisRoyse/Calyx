use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use calyx_forge::{
    AdmissionController, BlockDeallocator, DevicePtr, GpuBlockRegistry, Result, VramBudgeter,
    VramProbe,
};
use criterion::{Criterion, black_box, criterion_group, criterion_main};

const GIB: usize = 1024 * 1024 * 1024;

struct StaticProbe;

impl VramProbe for StaticProbe {
    fn free_device_vram(&self) -> Result<usize> {
        Ok(64 * GIB)
    }
}

#[derive(Clone, Default)]
struct NoopDealloc;

impl BlockDeallocator for NoopDealloc {
    fn free(&self, _ptr: DevicePtr, _size: usize) -> Result<()> {
        Ok(())
    }
}

fn bench_admission_overhead(c: &mut Criterion) {
    let budgeter = VramBudgeter::with_soft_cap(8 * GIB, StaticProbe);
    let registry = GpuBlockRegistry::new(&budgeter, NoopDealloc, 16);
    let controller = AdmissionController::new(&budgeter, Arc::new(Mutex::new(registry)), 8, 1);
    let deadline = Instant::now() + Duration::from_secs(60);

    c.bench_function("bench_admission_overhead", |b| {
        b.iter(|| {
            black_box(controller.decide(black_box(1024), black_box(1), black_box(deadline)));
        });
    });
}

criterion_group!(benches, bench_admission_overhead);
criterion_main!(benches);
