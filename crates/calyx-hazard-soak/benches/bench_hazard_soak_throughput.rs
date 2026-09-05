use calyx_hazard_soak::soak::{DEFAULT_SOAK_SEED, run_integrated_soak_at};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::fs;
use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const BENCH_OPS: u64 = 10_000;

fn bench_hazard_soak_throughput(c: &mut Criterion) {
    if !cfg!(target_os = "linux") {
        return;
    }

    let counter = AtomicU64::new(0);
    let mut group = c.benchmark_group("bench_hazard_soak_throughput");
    group.throughput(Throughput::Elements(BENCH_OPS));
    group.bench_function(
        BenchmarkId::new("ph59_integrated_soak_ops", BENCH_OPS),
        |b| {
            b.iter_custom(|iterations| {
                let mut measured = Duration::ZERO;
                for _ in 0..iterations {
                    let root = bench_root(counter.fetch_add(1, Ordering::Relaxed));
                    let _ = fs::remove_dir_all(&root);
                    let started = Instant::now();
                    let report = run_integrated_soak_at(&root, BENCH_OPS, DEFAULT_SOAK_SEED)
                        .expect("1e4-op hazard soak benchmark");
                    measured += started.elapsed();
                    assert_eq!(report.op_count, BENCH_OPS);
                    std::hint::black_box(report.wal_records_flushed);
                    fs::remove_dir_all(&root).expect("remove 1e4-op hazard soak benchmark root");
                }
                measured
            });
        },
    );
    group.finish();
}

fn bench_root(iteration: u64) -> PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-ph59-bench-hazard-soak-{}-{iteration}",
        process::id()
    ))
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(10))
        .sample_size(10);
    targets = bench_hazard_soak_throughput
}
criterion_main!(benches);
