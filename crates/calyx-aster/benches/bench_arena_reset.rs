use calyx_core::Arena;
use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};

const CAPACITIES: [usize; 4] = [
    4 * 1024,
    4 * 1024 * 1024,
    32 * 1024 * 1024,
    128 * 1024 * 1024,
];

fn bench_arena_reset(c: &mut Criterion) {
    let mut group = c.benchmark_group("bench_arena_reset");
    for capacity in CAPACITIES {
        group.bench_with_input(
            BenchmarkId::new("capacity", capacity),
            &capacity,
            |b, &cap| {
                let mut arena = Arena::new(cap).unwrap();
                b.iter(|| {
                    let ptr = arena.alloc(64, 8).unwrap();
                    black_box(ptr);
                    arena.reset();
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_arena_reset);
criterion_main!(benches);
