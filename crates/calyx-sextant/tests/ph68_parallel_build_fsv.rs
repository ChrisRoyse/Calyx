//! PH68 #712 — FSV for the parallel (ParlayANN prefix-doubling) Vamana build.
//!
//! Two guarantees, both verified against the on-disk graph file (SoT) and
//! brute-force ground truth — not return values alone:
//!   1. DETERMINISM across thread counts: building the same corpus inside a
//!      1-thread vs 8-thread rayon pool yields a BYTE-IDENTICAL graph file.
//!   2. RECALL preserved: recall@10 of the parallel-built graph vs brute-force
//!      cosine ground truth stays at/above a high bar on a planted corpus.

use std::path::{Path, PathBuf};

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use calyx_core::{CxId, SlotId};
use calyx_sextant::index::{DiskAnnBuildParams, DiskAnnSearch, DiskAnnSearchParams};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use sextant_support::cx_usize_be as cx;
use sha2::{Digest, Sha256};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("calyx-ph68-712").join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

/// Planted clustered corpus: each vector gets a strong +4.0 spike at dim
/// `idx % clusters`, so its true nearest neighbours are the same-residue nodes.
fn corpus(n: usize, dim: usize, clusters: usize, seed: u64) -> Vec<(CxId, Vec<f32>)> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    (0..n)
        .map(|idx| {
            let mut v: Vec<f32> = (0..dim).map(|_| rng.gen_range(-1.0..1.0)).collect();
            v[idx % clusters] += 4.0;
            (cx(idx), v)
        })
        .collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let (mut dot, mut aa, mut bb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        aa += x * x;
        bb += y * y;
    }
    if aa == 0.0 || bb == 0.0 {
        0.0
    } else {
        dot / (aa.sqrt() * bb.sqrt())
    }
}

fn brute_top_k(rows: &[(CxId, Vec<f32>)], q: &[f32], k: usize) -> Vec<u32> {
    let mut scored: Vec<(u32, f32)> = rows
        .iter()
        .enumerate()
        .map(|(i, (_, v))| (i as u32, cosine(q, v)))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.into_iter().take(k).map(|(i, _)| i).collect()
}

fn build_at(path: &Path, rows: &[(CxId, Vec<f32>)], threads: usize) {
    let params = DiskAnnBuildParams {
        dim: rows[0].1.len(),
        m_max: 24,
        ef_construction: 96,
        alpha: 1.2,
    };
    let sp = DiskAnnSearchParams {
        beamwidth: 48,
        ef_search: 96,
        rescore_k: 16,
        rescore_from_raw: false,
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .expect("pool");
    pool.install(|| {
        DiskAnnSearch::build(SlotId::new(0), path.to_path_buf(), rows, params, None, sp)
            .expect("parallel build");
    });
}

fn sha256_file(path: &Path) -> String {
    let bytes = std::fs::read(path).expect("read graph file");
    let mut h = Sha256::new();
    h.update(&bytes);
    format!("{:x} ({} bytes)", h.finalize(), bytes.len())
}

#[test]
fn fsv_determinism_across_thread_counts_and_recall() {
    let rows = corpus(3000, 64, 16, 712);
    println!("\n=== PH68 #712 parallel-build FSV (n=3000 dim=64 clusters=16) ===");

    // --- Guarantee 1: determinism across thread counts ---
    let d1 = scratch("t1");
    let p1 = d1.join("idx/slot_00.ann/graph.cda");
    let d8 = scratch("t8");
    let p8 = d8.join("idx/slot_00.ann/graph.cda");
    build_at(&p1, &rows, 1);
    build_at(&p8, &rows, 8);
    let s1 = sha256_file(&p1);
    let s8 = sha256_file(&p8);
    println!("[SoT] graph @ 1 thread : {s1}");
    println!("[SoT] graph @ 8 threads: {s8}");
    println!("[SoT] file p1 = {}", p1.display());
    assert_eq!(
        s1, s8,
        "parallel build MUST be byte-identical across thread counts"
    );

    // --- Guarantee 2: recall@10 preserved vs brute-force ground truth ---
    let index = {
        let params = DiskAnnBuildParams {
            dim: 64,
            m_max: 24,
            ef_construction: 96,
            alpha: 1.2,
        };
        let sp = DiskAnnSearchParams {
            beamwidth: 48,
            ef_search: 96,
            rescore_k: 16,
            rescore_from_raw: false,
        };
        let d = scratch("recall");
        DiskAnnSearch::build(
            SlotId::new(0),
            d.join("idx/slot_00.ann/graph.cda"),
            &rows,
            params,
            None,
            sp,
        )
        .expect("build")
    };
    let sp = DiskAnnSearchParams {
        beamwidth: 48,
        ef_search: 96,
        rescore_k: 16,
        rescore_from_raw: false,
    };
    let queries = [0usize, 5, 17, 123, 999, 2500];
    let (mut hit, mut total) = (0usize, 0usize);
    for &qi in &queries {
        let q = &rows[qi].1;
        let truth: std::collections::BTreeSet<u32> =
            brute_top_k(&rows, q, 10).into_iter().collect();
        let got: Vec<(u32, f32)> = index.search_ids(q, 10, &sp).expect("search");
        let got_ids: std::collections::BTreeSet<u32> = got.iter().map(|(id, _)| *id).collect();
        let overlap = truth.intersection(&got_ids).count();
        println!(
            "[recall] q={qi}: truth-top10 ∩ got-top10 = {overlap}/10  (got rank0={:?})",
            got.first()
        );
        hit += overlap;
        total += 10;
        assert!(
            got.windows(2).all(|w| w[0].1 <= w[1].1),
            "distances must be non-decreasing"
        );
    }
    let recall = hit as f32 / total as f32;
    println!(
        "[recall] overall recall@10 = {:.3} over {} queries",
        recall,
        queries.len()
    );
    assert!(
        recall >= 0.90,
        "parallel-built graph recall@10 must stay >= 0.90, got {recall}"
    );
}

#[test]
#[ignore = "timing demo — run explicitly with --ignored in a manual verification run"]
fn fsv_parallel_build_speedup() {
    let rows = corpus(20000, 128, 32, 99);
    let d1 = scratch("spd1");
    let d8 = scratch("spd8");
    let t0 = std::time::Instant::now();
    build_at(&d1.join("idx/slot_00.ann/graph.cda"), &rows, 1);
    let serial = t0.elapsed();
    let t1 = std::time::Instant::now();
    build_at(&d8.join("idx/slot_00.ann/graph.cda"), &rows, 8);
    let par = t1.elapsed();
    println!(
        "[speedup] n=20000 dim=128: 1-thread={:?}  8-thread={:?}  speedup={:.2}x",
        serial,
        par,
        serial.as_secs_f64() / par.as_secs_f64()
    );
    assert!(par < serial, "8 threads must beat 1 thread");
}
