use super::*;

#[test]
fn gen_row_is_deterministic_and_normalized() {
    let a = gen_row(42, 12345, 64);
    let b = gen_row(42, 12345, 64);
    assert_eq!(a, b, "same (seed,idx) -> same row");
    let norm = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-5, "unit norm, got {norm}");
    assert_ne!(gen_row(42, 1, 64), gen_row(42, 2, 64));
}

#[test]
fn balance_regions_splits_oversized_and_preserves_all_members() {
    let dim = 16;
    let sample: Vec<(u32, Vec<f32>)> = (0..400).map(|i| (i, gen_row(9, i as u64, dim))).collect();
    let initial = build_centroids(&sample, 2, 9);
    let buckets = vec![
        (0..500u64).collect::<Vec<_>>(),
        (500..540u64).collect::<Vec<_>>(),
    ];
    let cap = 100;
    let (cents, final_buckets) = balance_regions(&initial, buckets, 9, dim, cap);

    let total: usize = final_buckets.iter().map(Vec::len).sum();
    assert_eq!(total, 540, "all members preserved across the split");
    let mut all: Vec<u64> = final_buckets.iter().flatten().copied().collect();
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), 540, "no member duplicated or dropped");
    assert_eq!(cents.len(), final_buckets.len(), "centroid per region");
    assert!(
        final_buckets.len() >= 6,
        "oversized region split into >=5 parts"
    );
    let max_region = final_buckets.iter().map(Vec::len).max().unwrap();
    assert!(
        max_region <= cap,
        "max region must obey cap {cap}, got {max_region}"
    );
}

#[test]
fn balance_regions_recursively_enforces_cap() {
    let dim = 16;
    let sample: Vec<(u32, Vec<f32>)> = (0..800).map(|i| (i, gen_row(11, i as u64, dim))).collect();
    let initial = build_centroids(&sample, 1, 11);
    let buckets = vec![(0..900u64).collect::<Vec<_>>()];
    let cap = 37;
    let (cents, final_buckets) = balance_regions(&initial, buckets, 11, dim, cap);

    assert_eq!(cents.len(), final_buckets.len(), "centroid per region");
    assert_eq!(
        final_buckets.iter().map(Vec::len).sum::<usize>(),
        900,
        "all members preserved"
    );
    assert!(
        final_buckets.iter().all(|bucket| bucket.len() <= cap),
        "every final bucket must be <= cap"
    );
}

#[test]
fn partitioned_self_recall_and_region_restriction() {
    let dir = std::env::temp_dir().join(format!("calyx-part-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let p = PartitionBuildParams {
        n_cx: 5_000,
        dim: 32,
        n_regions: 16,
        seed: 7,
        sample: 5_000,
        chunk: 1_000,
        m_max: 16,
        ef_construction: 64,
    };
    let manifest = build_partitioned_vault(&dir, p).expect("build");
    let total: usize = manifest.regions.iter().map(|r| r.count).sum();
    assert_eq!(total, 5_000, "all cx partitioned exactly once");

    let search = PartitionedSearch::open(&dir).expect("open");
    let mut hits = 0;
    let n = 200;
    for s in 0..n {
        let idx = (s as u64 * 23) % p.n_cx;
        let q = gen_row(p.seed, idx, p.dim);
        let res = search.search(&q, 10, 4, 64).expect("search");
        if res.iter().any(|(c, _)| *c == idx) {
            hits += 1;
        }
    }
    let recall = hits as f32 / n as f32;
    assert!(recall >= 0.85, "self-recall@10 {recall} < 0.85");
    let _ = std::fs::remove_dir_all(&dir);
}
