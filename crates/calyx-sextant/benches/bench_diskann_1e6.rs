use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use calyx_sextant::index::distance::unit_l2_cosine_distance;
use calyx_sextant::index::{DiskAnnSearchParams, build_synthetic_vault, l2_normalize, l2_sq};
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

fn bench_distance_l2_512(c: &mut Criterion) {
    let raw_a = bench_vec(7);
    let raw_b = bench_vec(29);
    let a = l2_normalize(&raw_a);
    let b = l2_normalize(&raw_b);
    c.bench_function("distance_old_cosine_scalar_512", |bench| {
        bench.iter(|| criterion::black_box(cosine_distance_scalar(&raw_a, &raw_b)))
    });
    c.bench_function("distance_unit_l2_kernel_512", |bench| {
        bench.iter(|| criterion::black_box(unit_l2_cosine_distance(&a, &b)))
    });
    c.bench_function("distance_l2_sq_scalar_512", |bench| {
        bench.iter(|| criterion::black_box(l2_sq_scalar(&a, &b)))
    });
    c.bench_function("distance_l2_sq_kernel_512", |bench| {
        bench.iter(|| criterion::black_box(l2_sq(&a, &b)))
    });
}

fn bench_vec(seed: u32) -> Vec<f32> {
    (0..512)
        .map(|i| {
            let n = seed
                .wrapping_mul(1_664_525)
                .wrapping_add((i as u32).wrapping_mul(1_013_904_223));
            ((n % 10_000) as f32 / 5_000.0) - 1.0
        })
        .collect()
}

fn l2_sq_scalar(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(left, right)| {
            let d = left - right;
            d * d
        })
        .sum()
}

fn cosine_distance_scalar(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0;
    let mut an = 0.0;
    let mut bn = 0.0;
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        an += x * x;
        bn += y * y;
    }
    if an == 0.0 || bn == 0.0 {
        1.0
    } else {
        (1.0 - dot / (an.sqrt() * bn.sqrt())).max(0.0)
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench_diskann_1e6, bench_distance_l2_512
}
criterion_main!(benches);
