use super::super::assignment::{
    AssignmentRouting, AssignmentSink, BoundedAssignmentConfig, read_ids,
    stream_assign_to_ids_bounded,
};
use crate::index::SpannCentroidIndex;
use crate::index::partitioned::{
    PartitionBuildParams, PartitionedSearch, VectorSource, build_partitioned_vault,
    partitioned_manifest_db_exists,
};

#[test]
fn partitioned_open_rejects_corrupt_root_graph() {
    let dir = std::env::temp_dir().join(format!("calyx-part-root-corrupt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let p = params(31);
    let manifest = build_partitioned_vault(&dir, p).expect("build");
    corrupt_format_version(&dir.join(&manifest.root_graph_rel));

    let error = match PartitionedSearch::open(&dir) {
        Ok(_) => panic!("corrupt root graph opened"),
        Err(error) => error,
    };

    assert_eq!(error.code, crate::error::CALYX_INDEX_CORRUPT);
    assert!(error.message.contains("root graph"));
    assert!(error.message.contains(&manifest.root_graph_rel));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn partitioned_open_rejects_corrupt_unprobed_region_graph() {
    let dir =
        std::env::temp_dir().join(format!("calyx-part-region-corrupt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let p = params(37);
    let manifest = build_partitioned_vault(&dir, p).expect("build");
    let meta = manifest.regions.last().expect("region");
    corrupt_format_version(&dir.join(&meta.graph_rel));

    let error = match PartitionedSearch::open(&dir) {
        Ok(_) => panic!("corrupt region graph opened"),
        Err(error) => error,
    };

    assert_eq!(error.code, crate::error::CALYX_INDEX_CORRUPT);
    assert!(error.message.contains("region"));
    assert!(error.message.contains(&meta.graph_rel));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn partitioned_manifest_is_db_native_and_required() {
    let dir = std::env::temp_dir().join(format!("calyx-part-db-manifest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let p = params(41);
    let built = build_partitioned_vault(&dir, p).expect("build");

    assert!(partitioned_manifest_db_exists(&dir).expect("manifest db exists"));
    assert!(
        !dir.join("partitioned-manifest.json").exists(),
        "partitioned manifest authority must not be persisted as JSON"
    );
    let search = PartitionedSearch::open(&dir).expect("open from db manifest");
    assert_eq!(search.manifest().n_cx, built.n_cx);
    assert_eq!(search.manifest().regions.len(), built.regions.len());

    std::fs::remove_dir_all(dir.join("cf")).expect("remove manifest db rows");
    let error = match PartitionedSearch::open(&dir) {
        Ok(_) => panic!("missing DB manifest opened"),
        Err(error) => error,
    };
    assert_eq!(error.code, crate::error::CALYX_INDEX_MANIFEST_DB_MISSING);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bounded_assignment_cap_is_hard_stored_region_cap() {
    let dir = std::env::temp_dir().join(format!("calyx-part-hard-cap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let centroids = SpannCentroidIndex::from_parts(
        2,
        vec![vec![0.0, 0.0], vec![10.0, 0.0]],
        Vec::new(),
        Vec::new(),
    )
    .expect("centroids");
    let source = StaticSource {
        rows: vec![vec![5.0, 0.0]; 4],
    };

    let (regions, _) = stream_assign_to_ids_bounded(
        &dir,
        AssignmentSink::Final,
        &centroids,
        &source,
        2,
        BoundedAssignmentConfig {
            cap: 2,
            routing_probe: 2,
            routing: AssignmentRouting::Exact,
            boundary_epsilon: 3.0,
            max_replication: 2,
            apply_rng_rule: false,
            rng_factor: 1.0,
        },
    )
    .expect("bounded assignment");

    assert_eq!(regions.iter().map(|region| region.count).sum::<usize>(), 4);
    assert!(regions.iter().all(|region| region.count <= 2));
    for region in &regions {
        assert_eq!(
            read_ids(&dir.join(&region.ids_rel)).unwrap().len(),
            region.count
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn query_aware_pruning_bounds_touched_regions_and_rejects_bad_epsilon() {
    use crate::index::partitioned::{PartitionedSearchOptions, gen_row};

    let dir = std::env::temp_dir().join(format!("calyx-part-prune-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let p = params(43);
    let manifest = build_partitioned_vault(&dir, p).expect("build");
    assert!(manifest.n_regions >= 2, "need multiple regions to prune");
    let search = PartitionedSearch::open(&dir).expect("open");
    let query = gen_row(p.seed, 17, p.dim);
    let ceiling = manifest.n_regions.min(4);

    let unpruned = search
        .search_with_readback_opts(
            &query,
            5,
            PartitionedSearchOptions {
                n_probe: ceiling,
                region_beam: 32,
                pruning_epsilon: None,
            },
        )
        .expect("unpruned search");
    assert_eq!(unpruned.touched_regions.len(), ceiling);

    // A huge epsilon prunes nothing: identical to the fixed-probe search.
    let wide = search
        .search_with_readback_opts(
            &query,
            5,
            PartitionedSearchOptions {
                n_probe: ceiling,
                region_beam: 32,
                pruning_epsilon: Some(1.0e6),
            },
        )
        .expect("wide search");
    assert_eq!(wide.touched_regions.len(), ceiling);
    assert_eq!(wide.hits, unpruned.hits);

    // Epsilon 0 keeps only regions tied with the nearest centroid — at least
    // one and strictly fewer than the ceiling for a generic query.
    let tight = search
        .search_with_readback_opts(
            &query,
            5,
            PartitionedSearchOptions {
                n_probe: ceiling,
                region_beam: 32,
                pruning_epsilon: Some(0.0),
            },
        )
        .expect("tight search");
    assert!(!tight.touched_regions.is_empty());
    assert!(tight.touched_regions.len() < ceiling);
    assert!(!tight.hits.is_empty());

    for bad in [-0.5_f32, f32::NAN, f32::INFINITY] {
        let error = search
            .search_with_readback_opts(
                &query,
                5,
                PartitionedSearchOptions {
                    n_probe: ceiling,
                    region_beam: 32,
                    pruning_epsilon: Some(bad),
                },
            )
            .expect_err("bad epsilon must fail closed");
        assert_eq!(error.code, crate::error::CALYX_INDEX_INVALID_PARAMS);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn balance_cap_drives_hierarchical_region_granularity_and_rejects_zero() {
    let dir = std::env::temp_dir().join(format!("calyx-part-balcap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut p = params(47);
    p.n_cx = 512;
    p.sample = 512;
    p.n_regions = 2;
    p.balance_cap = Some(64);
    let manifest = build_partitioned_vault(&dir, p).expect("build");
    assert_eq!(manifest.region_balance_cap, 64);
    assert!(
        manifest.n_regions > 2,
        "balance cap 64 over 512 rows must split the 2 initial regions, got {}",
        manifest.n_regions
    );
    let _ = std::fs::remove_dir_all(&dir);

    let bad_dir = std::env::temp_dir().join(format!("calyx-part-balcap0-{}", std::process::id()));
    let mut bad = params(47);
    bad.balance_cap = Some(0);
    let error = build_partitioned_vault(&bad_dir, bad).expect_err("balance_cap 0 must fail");
    assert_eq!(error.code, crate::error::CALYX_INDEX_INVALID_PARAMS);
}

#[test]
fn rng_rule_skips_redundant_replica_but_keeps_boundary_replica() {
    // Row at [3,0]: primary centroid [1,0] (d_sq 4), replica candidate [0,0]
    // (d_sq 9). The centroids are 1 apart (l2_sq 1 < 9), so the RNG rule must
    // skip the replica; with the rule off the loose epsilon keeps it, and an
    // rng_factor >= 9 relaxes the squared-scale comparison enough to keep it.
    let centroids = SpannCentroidIndex::from_parts(
        2,
        vec![vec![0.0, 0.0], vec![1.0, 0.0]],
        Vec::new(),
        Vec::new(),
    )
    .expect("centroids");
    let source = StaticSource {
        rows: vec![vec![3.0, 0.0]],
    };
    let mut config = BoundedAssignmentConfig {
        cap: 4,
        routing_probe: 2,
        routing: AssignmentRouting::Exact,
        boundary_epsilon: 4.0,
        max_replication: 2,
        apply_rng_rule: true,
        rng_factor: 1.0,
    };

    let dir = std::env::temp_dir().join(format!("calyx-part-rng-on-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (regions, stats) =
        stream_assign_to_ids_bounded(&dir, AssignmentSink::Final, &centroids, &source, 1, config)
            .expect("rng-on assignment");
    assert_eq!(
        regions.iter().map(|region| region.count).sum::<usize>(),
        1,
        "RNG rule must skip the redundant replica"
    );
    assert_eq!(
        stats.rng_skipped, 1,
        "skip must be attributed to the RNG rule"
    );
    assert_eq!(stats.replicas_stored, 0);
    assert_eq!(stats.replica_histogram, vec![1]);
    let _ = std::fs::remove_dir_all(&dir);

    let dir = std::env::temp_dir().join(format!("calyx-part-rng-off-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    config.apply_rng_rule = false;
    let (regions, stats) =
        stream_assign_to_ids_bounded(&dir, AssignmentSink::Final, &centroids, &source, 1, config)
            .expect("rng-off assignment");
    assert_eq!(
        regions.iter().map(|region| region.count).sum::<usize>(),
        2,
        "without the RNG rule the loose epsilon keeps the replica"
    );
    assert_eq!(stats.rng_skipped, 0);
    assert_eq!(stats.replicas_stored, 1);
    assert_eq!(stats.replica_histogram, vec![0, 1]);
    let _ = std::fs::remove_dir_all(&dir);

    // SPTAG RNGFactor parity: candidate kept iff factor * s_sq >= d_sq, here
    // factor * 1 >= 9. Factor 8.9 still skips; factor 9.0 keeps.
    for (factor, expected_total) in [(8.9f32, 1usize), (9.0, 2)] {
        let dir = std::env::temp_dir().join(format!(
            "calyx-part-rng-factor-{factor}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        config.apply_rng_rule = true;
        config.rng_factor = factor;
        let (regions, _) = stream_assign_to_ids_bounded(
            &dir,
            AssignmentSink::Final,
            &centroids,
            &source,
            1,
            config,
        )
        .expect("rng-factor assignment");
        assert_eq!(
            regions.iter().map(|region| region.count).sum::<usize>(),
            expected_total,
            "rng_factor {factor} must store {expected_total} copies"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    let dir = std::env::temp_dir().join(format!("calyx-part-rng-bad-{}", std::process::id()));
    config.rng_factor = 0.0;
    let error =
        stream_assign_to_ids_bounded(&dir, AssignmentSink::Final, &centroids, &source, 1, config)
            .expect_err("rng_factor 0 must fail");
    assert_eq!(error.code, crate::error::CALYX_INDEX_INVALID_PARAMS);
}

fn params(seed: u64) -> PartitionBuildParams {
    PartitionBuildParams {
        n_cx: 128,
        dim: 16,
        n_regions: 4,
        seed,
        sample: 128,
        chunk: 64,
        m_max: 8,
        ef_construction: 32,
        region_build_parallelism: 2,
        final_assignment_probe: crate::index::DEFAULT_FINAL_ASSIGNMENT_PROBE,
        final_assignment_cap: None,
        balance_cap: None,
        assignment_boundary_epsilon: 0.10,
        assignment_max_replication: 2,
        assignment_rng_rule: true,
        assignment_rng_factor: 1.0,
    }
}

fn corrupt_format_version(path: &std::path::Path) {
    use std::io::{Seek, SeekFrom, Write};

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open graph for corruption");
    file.seek(SeekFrom::Start(8)).expect("seek format version");
    file.write_all(&99_u32.to_le_bytes())
        .expect("write bad format version");
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
