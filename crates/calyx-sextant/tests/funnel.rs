//! PH68 T05 - Kernel-first 3-hop funnel tests (issue #549).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use calyx_core::SlotId;
use calyx_sextant::index::{
    DiskAnnBuildParams, DiskAnnSearch, DiskAnnSearchParams, FinalCxSearch, FunnelParams,
    KernelFirstSearch, KernelRegion, KernelRegionAnn, RegionPartitions,
};
use proptest::prelude::*;
use serde::Serialize;
use sextant_support::cx_u32_be as cx;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("calyx-funnel-t05")
        .join(format!("{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn build_params(dim: usize) -> DiskAnnBuildParams {
    DiskAnnBuildParams {
        dim,
        m_max: 12,
        ef_construction: 64,
        alpha: 1.2,
    }
}

fn search_params(n: usize) -> DiskAnnSearchParams {
    DiskAnnSearchParams {
        beamwidth: n.max(16),
        ef_search: n.max(16),
        rescore_k: n.max(16),
        rescore_from_raw: false,
    }
}

fn region_vector(region: u32, dim: usize) -> Vec<f32> {
    let mut vector = (0..dim)
        .map(|i| {
            let wave = ((region + 1) as f32 * (i as f32 + 1.7)).sin() * 0.08;
            let residue = ((region.wrapping_mul(31) + i as u32 * 17) % 23) as f32 / 230.0;
            wave + residue
        })
        .collect::<Vec<_>>();
    vector[region as usize % dim] += 1.0;
    vector[(region as usize / dim) % dim] += 0.35;
    normalize(&mut vector);
    vector
}

fn cx_vector(local: u32, per_region: u32, dim: usize) -> Vec<f32> {
    let region = local / per_region;
    let offset = (local % per_region) as f32 / per_region as f32;
    let mut vector = region_vector(region, dim);
    vector[(region as usize + 2) % dim] = offset * 0.01;
    vector
}

struct Fixture {
    root: PathBuf,
    query: Vec<f32>,
    final_rows: Vec<(u32, Vec<f32>)>,
    search: KernelFirstSearch,
}

fn fixture(tag: &str, n_regions: u32, per_region: u32) -> Fixture {
    let root = scratch(tag);
    fixture_at(root, n_regions, per_region)
}

fn fixture_at(root: PathBuf, n_regions: u32, per_region: u32) -> Fixture {
    let dim = 8;
    let region_rows = (0..n_regions)
        .map(|id| (cx(id), region_vector(id, dim)))
        .collect::<Vec<_>>();
    let region_ann = DiskAnnSearch::build(
        SlotId::new(90),
        root.join("idx/regions.ann/graph.cda"),
        &region_rows,
        build_params(dim),
        None,
        search_params(n_regions as usize),
    )
    .expect("region diskann");
    let final_rows = (0..n_regions * per_region)
        .map(|id| (id, cx_vector(id, per_region, dim)))
        .collect::<Vec<_>>();
    let final_diskann_rows = final_rows
        .iter()
        .map(|(id, vector)| (cx(*id), vector.clone()))
        .collect::<Vec<_>>();
    let final_ann = DiskAnnSearch::build(
        SlotId::new(91),
        root.join("idx/slot_00.ann/graph.cda"),
        &final_diskann_rows,
        build_params(dim),
        None,
        search_params(final_rows.len()),
    )
    .expect("cx diskann");
    let kernel = KernelRegionAnn::new(vec![
        KernelRegion {
            id: 0,
            vector: region_vector(0, dim),
        },
        KernelRegion {
            id: 3,
            vector: region_vector(3, dim),
        },
        KernelRegion {
            id: 6,
            vector: region_vector(6, dim),
        },
    ])
    .expect("kernel ann");
    let partitions = RegionPartitions::new((0..n_regions * per_region).map(|id| {
        let region = id / per_region;
        (id, region)
    }));
    let search = KernelFirstSearch::new(
        u64::from(n_regions * per_region),
        Some(kernel),
        region_ann,
        FinalCxSearch::DiskAnn(final_ann),
        partitions,
    )
    .with_min_vault_size(u64::from(n_regions * per_region));
    Fixture {
        root,
        query: region_vector(3, dim),
        final_rows,
        search,
    }
}

fn params() -> FunnelParams {
    FunnelParams {
        n_kernel_probe: 3,
        n_region_beam: 16,
        n_cx_beam: 256,
        n_regions_to_expand: 4,
    }
}

#[test]
fn three_hop_funnel_returns_distinct_hits_with_paths() {
    let fx = fixture("three-hop", 10, 100);
    let hits = fx.search.search(&fx.query, 10, &params()).expect("search");

    assert_eq!(hits.len(), 10);
    assert!(hits.iter().all(|hit| hit.cx_id < 1000));
    let distinct: BTreeSet<_> = hits.iter().map(|hit| hit.cx_id).collect();
    assert_eq!(distinct.len(), hits.len());
    assert!(hits.iter().all(|hit| hit.path.kernel_region == 3));
    assert!(hits.iter().all(|hit| hit.path.region == 3));
}

#[test]
fn explain_path_proves_kernel_hop_was_used() {
    let fx = fixture("path", 10, 100);
    let hits = fx.search.search(&fx.query, 3, &params()).expect("search");

    for hit in hits {
        assert_eq!(hit.path.kernel_region, 3);
        assert_eq!(hit.path.region, hit.path.cx / 100);
        assert_eq!(hit.path.cx, hit.cx_id);
    }
}

#[test]
fn small_vault_guard_fails_closed() {
    let root = scratch("small");
    let fx = fixture_at(root, 10, 100);
    let guarded = fx.search.with_min_vault_size(10_000_000);
    let err = guarded
        .search(&fx.query, 10, &params())
        .expect_err("small vault guard");

    assert_eq!(err.code, "CALYX_INDEX_FUNNEL_VAULT_TOO_SMALL");
}

#[test]
fn planted_region_recall_at_10_is_at_least_point_9() {
    let fx = fixture("recall", 10, 100);
    let expected = brute_region_top(&fx.final_rows, &fx.query, 3, 100, 10);
    let hits = fx.search.search(&fx.query, 10, &params()).expect("search");
    let actual: BTreeSet<_> = hits.iter().map(|hit| hit.cx_id).collect();
    let overlap = expected.intersection(&actual).count();

    assert!(
        overlap >= 9,
        "overlap={overlap}, expected={expected:?}, actual={actual:?}"
    );
}

#[test]
fn zero_regions_to_expand_is_invalid_params() {
    let fx = fixture("bad-params", 10, 100);
    let mut p = params();
    p.n_regions_to_expand = 0;
    let err = fx
        .search
        .search(&fx.query, 10, &p)
        .expect_err("invalid params");

    assert_eq!(err.code, "CALYX_INDEX_INVALID_PARAMS");
}

#[test]
fn empty_kernel_index_is_unavailable() {
    let root = scratch("empty-kernel");
    let mut fx = fixture_at(root, 10, 100);
    let empty = KernelRegionAnn::empty(8).expect("empty kernel");
    fx.search = KernelFirstSearch::new(
        1000,
        Some(empty),
        extract_region_ann(&fx.root, 10),
        extract_final_ann(&fx.root, 1000),
        RegionPartitions::new((0..1000).map(|id| (id, id / 100))),
    )
    .with_min_vault_size(1000);
    let err = fx
        .search
        .search(&fx.query, 10, &params())
        .expect_err("empty kernel");

    assert_eq!(err.code, "CALYX_INDEX_KERNEL_UNAVAILABLE");
}

#[test]
#[cfg(unix)]
fn missing_region_graph_fails_closed_as_io() {
    let fx = fixture("missing-region", 10, 100);
    std::fs::remove_file(fx.root.join("idx/regions.ann/graph.cda")).expect("remove region graph");
    let err = fx
        .search
        .search(&fx.query, 10, &params())
        .expect_err("missing region graph");

    assert_eq!(err.code, "CALYX_INDEX_IO");
}

#[test]
#[ignore = "server-only FSV trigger writes funnel trace artifacts"]
fn fsv_issue549_writes_three_hop_trace() {
    let root =
        PathBuf::from(std::env::var("CALYX_FUNNEL_FSV_DIR").expect("set CALYX_FUNNEL_FSV_DIR"));
    assert!(
        root.to_string_lossy().contains("issue549"),
        "FSV root must be dedicated to issue549"
    );
    let fx = fixture_at(root.clone(), 100, 100);
    let p = FunnelParams {
        n_kernel_probe: 3,
        n_region_beam: 64,
        n_cx_beam: 1024,
        n_regions_to_expand: 8,
    };
    let hits = fx.search.search(&fx.query, 10, &p).expect("FSV search");
    let expected = brute_region_top(&fx.final_rows, &fx.query, 3, 100, 10);
    let overlap = hits
        .iter()
        .filter(|hit| expected.contains(&hit.cx_id))
        .count();
    let trace = TraceReport {
        trigger: "kernel_first_3hop",
        expected_region: 3,
        expected_recall_overlap: overlap,
        hits,
    };
    std::fs::write(
        root.join("funnel_trace.json"),
        serde_json::to_vec_pretty(&trace).expect("json"),
    )
    .expect("write trace");
    assert!(overlap >= 9);
}

#[test]
#[ignore = "server-only FSV trigger writes funnel edge artifacts"]
fn fsv_issue549_edges_write_before_after_artifacts() {
    let root =
        PathBuf::from(std::env::var("CALYX_FUNNEL_EDGE_DIR").expect("set CALYX_FUNNEL_EDGE_DIR"));
    assert_eq!(
        root.file_name().and_then(|name| name.to_str()),
        Some("edges")
    );
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("edge root");
    edge_small_vault(&root);
    edge_empty_kernel(&root);
    edge_invalid_params(&root);
    edge_missing_region_graph(&root);
}

fn edge_small_vault(root: &Path) {
    let fx = fixture_at(root.join("small_vault"), 10, 100);
    std::fs::write(root.join("small-before.txt"), graph_listing(&fx.root)).unwrap();
    let err = fx
        .search
        .with_min_vault_size(10_000_000)
        .search(&fx.query, 10, &params())
        .expect_err("small vault");
    std::fs::write(root.join("small-after.txt"), graph_listing(&fx.root)).unwrap();
    std::fs::write(root.join("small-result.txt"), err.code).unwrap();
}

fn edge_empty_kernel(root: &Path) {
    let fx = fixture_at(root.join("empty_kernel"), 10, 100);
    let search = KernelFirstSearch::new(
        1000,
        Some(KernelRegionAnn::empty(8).unwrap()),
        extract_region_ann(&fx.root, 10),
        extract_final_ann(&fx.root, 1000),
        RegionPartitions::new((0..1000).map(|id| (id, id / 100))),
    )
    .with_min_vault_size(1000);
    std::fs::write(root.join("kernel-before.txt"), graph_listing(&fx.root)).unwrap();
    let err = search
        .search(&fx.query, 10, &params())
        .expect_err("empty kernel");
    std::fs::write(root.join("kernel-after.txt"), graph_listing(&fx.root)).unwrap();
    std::fs::write(root.join("kernel-result.txt"), err.code).unwrap();
}

fn edge_invalid_params(root: &Path) {
    let fx = fixture_at(root.join("invalid_params"), 10, 100);
    let mut p = params();
    p.n_regions_to_expand = 0;
    std::fs::write(root.join("params-before.txt"), graph_listing(&fx.root)).unwrap();
    let err = fx.search.search(&fx.query, 10, &p).expect_err("bad params");
    std::fs::write(root.join("params-after.txt"), graph_listing(&fx.root)).unwrap();
    std::fs::write(root.join("params-result.txt"), err.code).unwrap();
}

fn edge_missing_region_graph(root: &Path) {
    let fx = fixture_at(root.join("missing_region"), 10, 100);
    let path = fx.root.join("idx/regions.ann/graph.cda");
    std::fs::write(root.join("region-before.txt"), file_state(&path)).unwrap();
    #[cfg(unix)]
    std::fs::remove_file(&path).unwrap();
    let err = fx
        .search
        .search(&fx.query, 10, &params())
        .expect_err("missing region");
    std::fs::write(root.join("region-after.txt"), path_state(&path)).unwrap();
    std::fs::write(root.join("region-result.txt"), err.code).unwrap();
}

fn extract_region_ann(root: &Path, n_regions: usize) -> DiskAnnSearch {
    let rows = (0..n_regions as u32)
        .map(|id| (cx(id), region_vector(id, 8)))
        .collect::<Vec<_>>();
    DiskAnnSearch::open(
        SlotId::new(90),
        root.join("idx/regions.ann/graph.cda"),
        rows.iter().map(|(id, _)| *id).collect(),
        None,
        search_params(n_regions),
    )
    .expect("open region ann")
}

fn extract_final_ann(root: &Path, n: usize) -> FinalCxSearch {
    let ids = (0..n as u32).map(cx).collect::<Vec<_>>();
    FinalCxSearch::DiskAnn(
        DiskAnnSearch::open(
            SlotId::new(91),
            root.join("idx/slot_00.ann/graph.cda"),
            ids,
            None,
            search_params(n),
        )
        .expect("open final ann"),
    )
}

fn brute_region_top(
    rows: &[(u32, Vec<f32>)],
    query: &[f32],
    region: u32,
    per_region: u32,
    k: usize,
) -> BTreeSet<u32> {
    let mut scores = rows
        .iter()
        .filter(|(id, _)| *id / per_region == region)
        .map(|(id, vector)| (*id, cosine(query, vector)))
        .collect::<Vec<_>>();
    scores.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    scores.into_iter().take(k).map(|(id, _)| id).collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let (dot, aa, bb) = a
        .iter()
        .zip(b)
        .fold((0.0_f32, 0.0_f32, 0.0_f32), |(dot, aa, bb), (x, y)| {
            (dot + x * y, aa + x * x, bb + y * y)
        });
    dot / (aa.sqrt() * bb.sqrt())
}

fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    for value in vector {
        *value /= norm;
    }
}

fn graph_listing(root: &Path) -> String {
    let mut rows = std::fs::read_dir(root.join("idx"))
        .expect("read idx")
        .flat_map(|entry| {
            let entry = entry.expect("entry");
            std::fs::read_dir(entry.path())
                .expect("child")
                .map(move |child| {
                    let child = child.expect("child");
                    format!(
                        "{}/{} {} bytes",
                        entry.file_name().to_string_lossy(),
                        child.file_name().to_string_lossy(),
                        child.metadata().expect("metadata").len()
                    )
                })
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows.join("\n")
}

fn file_state(path: &Path) -> String {
    let bytes = std::fs::read(path).expect("read");
    format!(
        "exists=true size={} sha256-input-bytes={}\n",
        bytes.len(),
        bytes.len()
    )
}

fn path_state(path: &Path) -> String {
    format!("exists={}\n", path.exists())
}

#[derive(Serialize)]
struct TraceReport {
    trigger: &'static str,
    expected_region: u32,
    expected_recall_overlap: usize,
    hits: Vec<calyx_sextant::FunnelHit>,
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    #[test]
    fn probe_count_returns_distinct_descending_hits(n_probe in 1_usize..=3) {
        let fx = fixture("prop", 10, 50);
        let mut p = params();
        p.n_kernel_probe = n_probe;
        p.n_cx_beam = 128;
        let hits = fx.search.search(&fx.query, 10, &p).expect("search");

        prop_assert_eq!(hits.len(), 10);
        let distinct: BTreeSet<_> = hits.iter().map(|hit| hit.cx_id).collect();
        prop_assert_eq!(distinct.len(), hits.len());
        prop_assert!(hits.windows(2).all(|pair| pair[0].score >= pair[1].score));
    }
}
