use calyx_hazard_soak::soak::{DEFAULT_SOAK_SEED, run_integrated_soak_at};
use criterion::{Criterion, criterion_group, criterion_main};

fn bench_hazard_soak_throughput(c: &mut Criterion) {
    if !cfg!(target_os = "linux") {
        return;
    }
    c.bench_function("ph59_integrated_soak_1e4_ops", |b| {
        b.iter(|| {
            let root = std::env::temp_dir().join("calyx-ph59-bench-hazard-soak");
            let report = run_integrated_soak_at(&root, 10_000, DEFAULT_SOAK_SEED)
                .expect("1e4-op hazard soak benchmark");
            assert!(report.op_count >= 10_000);
        });
    });
}

criterion_group!(benches, bench_hazard_soak_throughput);
criterion_main!(benches);
