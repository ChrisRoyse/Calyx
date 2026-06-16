use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use calyx_sextant::index::{DiskAnnSearchParams, build_synthetic_vault};
use criterion::{Criterion, criterion_group, criterion_main};

static RUN_ID: AtomicU64 = AtomicU64::new(0);

fn bench_diskann_1e6(c: &mut Criterion) {
    let n_cx = std::env::var("CALYX_BENCH_N_CX")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1_000_000);
    let dim = std::env::var("CALYX_BENCH_DIM")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(64);
    c.bench_function("bench_diskann_1e6", |b| {
        b.iter_custom(|iters| {
            let root = std::env::temp_dir()
                .join("calyx-sextant-bench-diskann")
                .join(format!("{}", RUN_ID.fetch_add(1, Ordering::Relaxed)));
            let vault =
                build_synthetic_vault(n_cx, dim, 1, 550, &root).expect("synthetic bench vault");
            let params = DiskAnnSearchParams {
                beamwidth: 64,
                ef_search: 128,
                rescore_k: 128,
                rescore_from_raw: false,
            };
            let start = Instant::now();
            for idx in 0..iters as usize {
                let query = &vault.rows[idx % vault.rows.len()].1;
                let hits = vault
                    .diskann
                    .search_ids(query, 10, &params)
                    .expect("search");
                criterion::black_box(hits);
            }
            start.elapsed()
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench_diskann_1e6
}
criterion_main!(benches);
