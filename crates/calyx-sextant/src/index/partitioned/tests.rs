use super::assignment::{
    AssignmentRouting, AssignmentSink, BoundedAssignmentConfig, read_ids,
    stream_assign_to_ids_bounded, stream_assign_to_ids_with_routing,
};
use super::balance::balance_regions;
use super::*;

mod validation;

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
fn bounded_final_assignment_caps_regions_and_preserves_ids() {
    let dir = std::env::temp_dir().join(format!("calyx-part-bounded-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let centroids = SpannCentroidIndex::from_parts(
        2,
        vec![vec![1.0, 0.0], vec![-1.0, 0.0]],
        Vec::new(),
        Vec::new(),
    )
    .expect("centroids");
    let mut rows = Vec::new();
    for _ in 0..10 {
        rows.push(vec![1.0, 0.0]);
    }
    for _ in 0..2 {
        rows.push(vec![-1.0, 0.0]);
    }
    let source = StaticSource { rows };
    let regions = stream_assign_to_ids_bounded(
        &dir,
        AssignmentSink::Final,
        &centroids,
        &source,
        4,
        BoundedAssignmentConfig {
            cap: 6,
            routing_probe: 2,
            routing: AssignmentRouting::Hnsw,
            boundary_epsilon: 0.0,
            max_replication: 1,
        },
    )
    .expect("bounded assignment");

    assert_eq!(regions.iter().map(|r| r.count).sum::<usize>(), 12);
    assert!(
        regions.iter().all(|region| region.count <= 6),
        "bounded assignment must enforce the cap"
    );
    let mut ids = Vec::new();
    for region in &regions {
        ids.extend(read_ids(&dir.join(&region.ids_rel)).expect("ids"));
    }
    ids.sort_unstable();
    assert_eq!(ids, (0..12).collect::<Vec<_>>());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bounded_assignment_duplicates_boundary_rows_to_adjacent_ids_files() {
    let dir = std::env::temp_dir().join(format!("calyx-part-boundary-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let centroids = SpannCentroidIndex::from_parts(
        2,
        vec![vec![0.0, 0.0], vec![10.0, 0.0]],
        Vec::new(),
        Vec::new(),
    )
    .expect("centroids");
    let source = StaticSource {
        rows: vec![vec![5.0, 0.0]],
    };

    let regions = stream_assign_to_ids_bounded(
        &dir,
        AssignmentSink::Final,
        &centroids,
        &source,
        1,
        BoundedAssignmentConfig {
            cap: 2,
            routing_probe: 2,
            routing: AssignmentRouting::Exact,
            boundary_epsilon: 3.0,
            max_replication: 2,
        },
    )
    .expect("bounded replicated assignment");

    assert_eq!(regions.iter().map(|r| r.count).sum::<usize>(), 2);
    assert_eq!(
        read_ids(&dir.join("idx/region_00000.ids")).unwrap(),
        vec![0]
    );
    assert_eq!(
        read_ids(&dir.join("idx/region_00001.ids")).unwrap(),
        vec![0]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn assignment_writes_nofile_scale_region_count_without_stale_ids() {
    let dir = std::env::temp_dir().join(format!("calyx-part-fd-scale-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let region_count = 1_200usize;
    let rows: Vec<Vec<f32>> = (0..region_count)
        .map(|idx| vec![idx as f32 * 10.0, 0.0])
        .collect();
    let centroids =
        SpannCentroidIndex::from_parts(2, rows.clone(), Vec::new(), Vec::new()).expect("centroids");
    let source = StaticSource { rows };

    let stale = dir.join("idx/assign-initial/region_00007.ids");
    std::fs::create_dir_all(stale.parent().expect("parent")).expect("mkdir");
    let mut stale_bytes = Vec::new();
    stale_bytes.extend_from_slice(&9_999u64.to_le_bytes());
    stale_bytes.extend_from_slice(&8_888u64.to_le_bytes());
    std::fs::write(&stale, stale_bytes).expect("stale ids");

    let provisional = stream_assign_to_ids_with_routing(
        &dir,
        AssignmentSink::Provisional,
        &centroids,
        &source,
        37,
        AssignmentRouting::Exact,
    )
    .expect("provisional assignment");
    assert_eq!(provisional.len(), region_count);
    assert_eq!(
        read_ids(&dir.join("idx/assign-initial/region_00007.ids")).expect("ids"),
        vec![7],
        "assignment must clear stale bytes before append-mode chunk writes"
    );

    let final_regions = stream_assign_to_ids_bounded(
        &dir,
        AssignmentSink::Final,
        &centroids,
        &source,
        41,
        BoundedAssignmentConfig {
            cap: 1,
            routing_probe: 1,
            routing: AssignmentRouting::Exact,
            boundary_epsilon: 0.0,
            max_replication: 1,
        },
    )
    .expect("bounded assignment");
    assert_eq!(final_regions.len(), region_count);
    assert_eq!(
        read_ids(&dir.join("idx/region_01199.ids")).expect("ids"),
        vec![1_199]
    );
    let _ = std::fs::remove_dir_all(&dir);
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
        region_build_parallelism: 2,
        final_assignment_probe: DEFAULT_FINAL_ASSIGNMENT_PROBE,
        final_assignment_cap: None,
    };
    let manifest = build_partitioned_vault(&dir, p).expect("build");
    assert_eq!(manifest.region_build_parallelism, 2);
    let total: usize = manifest.regions.iter().map(|r| r.count).sum();
    assert!(total >= 5_000, "all cx persisted at least once");
    assert!(total <= 10_000, "replication is bounded to 2x");
    assert_eq!(manifest.stored_region_members, total);

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

    // TRUE recall@10 vs brute-force L2 over the whole dataset — the real gate
    // (#711). Self-recall is a weaker bar that can pass while true recall fails, so
    // tests and FSV must measure this directly against ground truth.
    let mut found = 0usize;
    let mut want = 0usize;
    for s in 0..n {
        let idx = (s as u64 * 41) % p.n_cx;
        let q = gen_row(p.seed, idx, p.dim);
        let truth = brute_force_topk(&q, p.seed, p.n_cx, p.dim, 10);
        let got: std::collections::BTreeSet<u64> = search
            .search(&q, 10, 8, 64)
            .expect("search")
            .into_iter()
            .map(|(c, _)| c)
            .collect();
        found += truth.iter().filter(|t| got.contains(t)).count();
        want += truth.len();
    }
    let true_recall = found as f32 / want as f32;
    assert!(true_recall >= 0.85, "true recall@10 {true_recall} < 0.85");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn search_readback_reports_only_touched_region_graphs() {
    let dir = std::env::temp_dir().join(format!("calyx-part-readback-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let p = PartitionBuildParams {
        n_cx: 512,
        dim: 24,
        n_regions: 8,
        seed: 19,
        sample: 512,
        chunk: 128,
        m_max: 12,
        ef_construction: 48,
        region_build_parallelism: 2,
        final_assignment_probe: DEFAULT_FINAL_ASSIGNMENT_PROBE,
        final_assignment_cap: None,
    };
    let manifest = build_partitioned_vault(&dir, p).expect("build");
    assert_eq!(manifest.region_build_parallelism, 2);
    let search = PartitionedSearch::open(&dir).expect("open");
    let readback = search
        .search_with_readback(&gen_row(p.seed, 17, p.dim), 5, 3, 32)
        .expect("search readback");

    assert!(!readback.hits.is_empty());
    assert!(!readback.touched_regions.is_empty());
    assert!(readback.touched_regions.len() <= 3);
    for region in &readback.touched_regions {
        let meta = manifest
            .regions
            .iter()
            .find(|meta| meta.id == *region)
            .expect("region in manifest");
        assert!(dir.join(&meta.graph_rel).is_file());
        assert!(dir.join(&meta.ids_rel).is_file());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn region_build_parallelism_is_effective_cap_and_zero_rejected() {
    let dir = std::env::temp_dir().join(format!("calyx-part-cap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut p = PartitionBuildParams {
        n_cx: 256,
        dim: 16,
        n_regions: 4,
        seed: 23,
        sample: 256,
        chunk: 64,
        m_max: 8,
        ef_construction: 32,
        region_build_parallelism: 64,
        final_assignment_probe: 4,
        final_assignment_cap: Some(128),
    };

    let manifest = build_partitioned_vault(&dir, p).expect("build");
    assert_eq!(
        manifest.region_build_parallelism,
        manifest.regions.len().max(1),
        "cap larger than region count is reduced to actual buildable regions"
    );
    let total: usize = manifest.regions.iter().map(|r| r.count).sum();
    assert!(total >= p.n_cx as usize);
    assert!(total <= p.n_cx as usize * 2);
    assert_eq!(manifest.stored_region_members, total);
    assert_eq!(manifest.final_assignment_probe, 4);
    assert_eq!(manifest.final_assignment_cap, Some(128));
    let raw_sidecars = manifest
        .regions
        .iter()
        .filter(|meta| dir.join(&meta.graph_rel).with_extension("raw").exists())
        .count();
    assert_eq!(
        raw_sidecars, 0,
        "partitioned search does not rescore from raw sidecars"
    );
    assert!(
        !dir.join(&manifest.root_graph_rel)
            .with_extension("raw")
            .exists()
    );

    let _ = std::fs::remove_dir_all(&dir);
    p.region_build_parallelism = 0;
    let err = build_partitioned_vault(&dir, p).unwrap_err();
    assert_eq!(err.code, crate::error::CALYX_INDEX_INVALID_PARAMS);
    assert!(err.message.contains("region_build_parallelism"));
    let _ = std::fs::remove_dir_all(&dir);

    p.region_build_parallelism = 2;
    p.final_assignment_probe = 0;
    let err = build_partitioned_vault(&dir, p).unwrap_err();
    assert_eq!(err.code, crate::error::CALYX_INDEX_INVALID_PARAMS);
    assert!(err.message.contains("final_assignment_probe"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn raw_l2_centroid_routing_uses_exact_l2_not_cosine() {
    let centroids = SpannCentroidIndex::from_parts(
        2,
        vec![vec![100.0, 0.0], vec![9.0, 1.0]],
        Vec::new(),
        Vec::new(),
    )
    .expect("centroids");

    let raw_l2 = centroids.nearest_centroids_exact_l2(&[10.0, 0.0], 1);
    assert_eq!(raw_l2, vec![1], "[9,1] is nearest by raw L2");
}

#[test]
fn raw_l2_graph_centroid_routing_preserves_raw_l2_order() {
    let centroids = SpannCentroidIndex::from_parts(
        2,
        (0..80).map(|idx| vec![idx as f32 * 10.0, 0.0]).collect(),
        Vec::new(),
        Vec::new(),
    )
    .expect("centroids");

    let routed = centroids.nearest_centroids_raw_l2_graph(&[410.0, 0.0], 1);
    assert_eq!(
        routed,
        vec![41],
        "raw-L2 graph routes to raw-nearest centroid"
    );
}

#[test]
fn raw_l2_graph_assignment_writes_nearest_raw_region_ids() {
    let dir = std::env::temp_dir().join(format!("calyx-part-raw-l2-graph-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let centroids = SpannCentroidIndex::from_parts(
        2,
        (0..80).map(|idx| vec![idx as f32 * 10.0, 0.0]).collect(),
        Vec::new(),
        Vec::new(),
    )
    .expect("centroids");
    let source = StaticSource {
        rows: vec![vec![410.0, 0.0]],
    };

    let regions = stream_assign_to_ids_with_routing(
        &dir,
        AssignmentSink::Final,
        &centroids,
        &source,
        1,
        AssignmentRouting::RawL2Graph,
    )
    .expect("raw l2 graph assignment");

    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].id, 41);
    assert_eq!(
        read_ids(&dir.join(&regions[0].ids_rel)).expect("ids"),
        vec![0]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Exact L2 top-k over the deterministic dataset — ground truth for recall.
fn brute_force_topk(query: &[f32], seed: u64, n_cx: u64, dim: usize, k: usize) -> Vec<u64> {
    let mut scored: Vec<(u64, f32)> = (0..n_cx)
        .map(|idx| {
            let row = gen_row(seed, idx, dim);
            let d: f32 = row.iter().zip(query).map(|(a, b)| (a - b) * (a - b)).sum();
            (idx, d)
        })
        .collect();
    scored.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    scored.into_iter().take(k).map(|(idx, _)| idx).collect()
}

struct StaticSource {
    rows: Vec<Vec<f32>>,
}

impl VectorSource for StaticSource {
    fn dim(&self) -> usize {
        self.rows[0].len()
    }

    fn len(&self) -> u64 {
        self.rows.len() as u64
    }

    fn row(&self, idx: u64) -> Vec<f32> {
        self.rows[idx as usize].clone()
    }
}
