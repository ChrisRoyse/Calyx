//! PH68 T04 - Dual-DiskANN asymmetric slot tests (issue #548).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use calyx_core::SlotVector;
use calyx_sextant::index::diskann::graph::DISKANN_MAGIC;
use calyx_sextant::index::{
    Direction, DirectionalBoost, DiskAnnBuildParams, DiskAnnSearchParams, DualDiskAnnSearch,
    SextantIndex, build_diskann_graph, build_dual, build_dual_with_search, dual_graph_path,
    open_dual,
};
use proptest::prelude::*;
use sextant_support::cx_usize_be as cx;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("calyx-diskann-dual-t04")
        .join(format!("{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn params(dim: usize) -> DiskAnnBuildParams {
    DiskAnnBuildParams {
        dim,
        m_max: 16,
        ef_construction: 96,
        alpha: 1.2,
    }
}

fn search_params(n: usize) -> DiskAnnSearchParams {
    DiskAnnSearchParams {
        beamwidth: 64,
        ef_search: n.max(8),
        rescore_k: n.max(8),
        rescore_from_raw: false,
    }
}

fn query(dim: usize) -> Vec<f32> {
    let mut q = vec![0.0; dim];
    q[0] = 1.0;
    q
}

type LocalRows = Vec<(u32, Vec<f32>)>;

fn dual_rows(n: usize, dim: usize) -> (LocalRows, LocalRows) {
    let denom = (n.saturating_sub(1)).max(1) as f32;
    let mut forward = Vec::with_capacity(n);
    let mut reverse = Vec::with_capacity(n);
    for id in 0..n {
        let t = id as f32 / denom;
        let mut a = vec![0.0; dim];
        a[0] = 1.0;
        a[1] = t;
        let mut b = vec![0.0; dim];
        b[0] = t;
        b[1] = 1.0;
        forward.push((id as u32, a));
        reverse.push((id as u32, b));
    }
    (forward, reverse)
}

fn build_index(tag: &str, n: usize) -> DualDiskAnnSearch {
    let root = scratch(tag);
    let (a, b) = dual_rows(n, 8);
    build_dual_with_search(&root, 0, &a, &b, params(8), search_params(n)).expect("build dual")
}

#[test]
fn directional_searches_use_distinct_graphs() {
    let index = build_index("directional", 200);
    let q = query(8);

    let forward = index
        .search_directional(&q, Direction::Forward, 5)
        .expect("forward search");
    let reverse = index
        .search_directional(&q, Direction::Reverse, 5)
        .expect("reverse search");

    assert_eq!(forward.len(), 5);
    assert_eq!(reverse.len(), 5);
    assert_ne!(
        forward.iter().map(|hit| hit.0).collect::<Vec<_>>(),
        reverse.iter().map(|hit| hit.0).collect::<Vec<_>>()
    );
    assert_eq!(forward[0].0, 0);
    assert_eq!(reverse[0].0, 199);
}

#[test]
fn merged_search_applies_directional_boost_and_dedupes() {
    let index = build_index("merged", 200);
    let q = query(8);
    let hits = index
        .search_merged(&q, 10, DirectionalBoost::new(0.7, 0.3).expect("boost"))
        .expect("merged search");

    assert_eq!(hits.len(), 10);
    assert_eq!(hits[0].0, 0);
    assert!(hits.windows(2).all(|pair| pair[0].1 >= pair[1].1));
    let ids: BTreeSet<_> = hits.iter().map(|hit| hit.0).collect();
    assert_eq!(ids.len(), hits.len());
}

#[test]
fn invalid_directional_boost_fails_before_search() {
    let index = build_index("bad-boost", 32);
    let err = index
        .search_merged(
            &query(8),
            5,
            DirectionalBoost {
                forward_weight: 0.8,
                reverse_weight: 0.3,
            },
        )
        .expect_err("invalid boost");

    assert_eq!(err.code, "CALYX_INDEX_INVALID_PARAMS");
}

#[test]
fn open_dual_missing_graph_fails_without_partial_fallback() {
    let root = scratch("missing-open");
    let (a, _) = dual_rows(16, 8);
    let a_path = dual_graph_path(&root, 0, Direction::Forward);
    build_diskann_graph(&a_path, &a, params(8)).expect("write forward only");

    let err = open_dual(&root, 0, search_params(16)).expect_err("missing reverse graph");

    assert_eq!(err.code, "CALYX_INDEX_IO");
}

#[test]
fn empty_dual_graph_search_is_empty_not_panic() {
    let root = scratch("empty");
    let index = build_dual(&root, 0, &[], &[], params(8)).expect("empty dual");
    let hits = index
        .search_merged(&query(8), 10, DirectionalBoost::default())
        .expect("empty search");

    assert!(hits.is_empty());
}

#[test]
fn k_above_node_count_returns_all_unique_ids() {
    let index = build_index("kgt", 8);
    let hits = index
        .search_merged(&query(8), 50, DirectionalBoost::default())
        .expect("k above count");

    assert_eq!(hits.len(), 8);
    let ids: BTreeSet<_> = hits.iter().map(|hit| hit.0).collect();
    assert_eq!(ids.len(), 8);
}

#[test]
#[cfg(unix)]
fn corrupt_reverse_graph_fails_reverse_while_forward_still_searches() {
    let root = scratch("corrupt-reverse");
    let (a, b) = dual_rows(64, 8);
    let index =
        build_dual_with_search(&root, 0, &a, &b, params(8), search_params(64)).expect("build");
    let b_path = dual_graph_path(&root, 0, Direction::Reverse);
    let mut bytes = std::fs::read(&b_path).expect("read reverse");
    bytes[0] ^= 0xff;
    std::fs::write(&b_path, bytes).expect("corrupt reverse");

    let forward = index
        .search_directional(&query(8), Direction::Forward, 5)
        .expect("forward survives reverse corruption");
    let err = index
        .search_directional(&query(8), Direction::Reverse, 5)
        .expect_err("reverse corrupt");

    assert_eq!(forward.len(), 5);
    assert_eq!(err.code, "CALYX_INDEX_CORRUPT");
}

#[test]
#[cfg(not(unix))]
fn corrupt_reverse_graph_fails_closed_on_open() {
    let root = scratch("corrupt-reverse-open");
    let (a, b) = dual_rows(64, 8);
    let index =
        build_dual_with_search(&root, 0, &a, &b, params(8), search_params(64)).expect("build");
    drop(index);
    let b_path = dual_graph_path(&root, 0, Direction::Reverse);
    let mut bytes = std::fs::read(&b_path).expect("read reverse");
    bytes[0] ^= 0xff;
    std::fs::write(&b_path, bytes).expect("corrupt reverse");

    let err = open_dual(&root, 0, search_params(64)).expect_err("corrupt reverse open");

    assert_eq!(err.code, "CALYX_INDEX_IO");
}

#[test]
fn sextant_index_adapter_returns_cxid_hits() {
    let mut index = build_index("trait", 32);
    let id = cx(7);
    index
        .insert(
            id,
            SlotVector::Dense {
                dim: 8,
                data: query(8),
            },
            9,
        )
        .expect("dual insert");

    let hits = index
        .search(
            &SlotVector::Dense {
                dim: 8,
                data: query(8),
            },
            4,
            None,
        )
        .expect("trait search");

    assert_eq!(hits[0].rank, 1);
    assert!(hits.iter().any(|hit| hit.cx_id == id));
    assert_eq!(index.stats().kind, "DualDiskANN");
    assert!(!index.is_degraded());
}

#[test]
#[ignore = "server-only FSV trigger writes dual DiskANN graph files"]
fn fsv_issue548_writes_dual_graphs_and_directional_hits() {
    let root = PathBuf::from(
        std::env::var("CALYX_DUAL_DISKANN_FSV_VAULT").expect("set CALYX_DUAL_DISKANN_FSV_VAULT"),
    );
    assert!(
        root.to_string_lossy().contains("issue548"),
        "FSV vault path must be dedicated to issue548"
    );
    let (a, b) = dual_rows(1000, 8);
    let index =
        build_dual_with_search(&root, 0, &a, &b, params(8), search_params(1000)).expect("build");
    let q = query(8);
    let forward = index
        .search_directional(&q, Direction::Forward, 10)
        .expect("forward fsv search");
    let reverse = index
        .search_directional(&q, Direction::Reverse, 10)
        .expect("reverse fsv search");
    let merged = index
        .search_merged(&q, 10, DirectionalBoost::new(0.7, 0.3).expect("boost"))
        .expect("merged fsv search");
    std::fs::write(
        root.join("directional_hits.csv"),
        hit_report(&forward, &reverse),
    )
    .expect("write directional hits");
    std::fs::write(root.join("merged_hits.csv"), merged_report(&merged)).expect("write merged");
    assert_eq!(
        &std::fs::read(dual_graph_path(&root, 0, Direction::Forward)).unwrap()[0..8],
        DISKANN_MAGIC
    );
    assert_eq!(
        &std::fs::read(dual_graph_path(&root, 0, Direction::Reverse)).unwrap()[0..8],
        DISKANN_MAGIC
    );
    assert_eq!(forward[0].0, 0);
    assert_eq!(reverse[0].0, 999);
}

#[test]
#[ignore = "server-only FSV trigger writes dual DiskANN edge artifacts"]
fn fsv_issue548_edges_write_before_after_artifacts() {
    let root = PathBuf::from(
        std::env::var("CALYX_DUAL_DISKANN_EDGE_DIR").expect("set CALYX_DUAL_DISKANN_EDGE_DIR"),
    );
    assert_eq!(
        root.file_name().and_then(|name| name.to_str()),
        Some("edges")
    );
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create edge root");

    edge_missing_direction(&root);
    edge_corrupt_reverse(&root);
    edge_invalid_boost(&root);
    edge_k_above_count(&root);
}

fn edge_missing_direction(root: &Path) {
    let vault = root.join("missing_direction");
    let (a, b) = dual_rows(32, 8);
    let index = build_dual_with_search(&vault, 0, &a, &b, params(8), search_params(32)).unwrap();
    let b_path = dual_graph_path(&vault, 0, Direction::Reverse);
    std::fs::write(root.join("missing-before.txt"), file_state(&b_path)).unwrap();
    std::fs::remove_file(&b_path).unwrap();
    let err = index
        .search_directional(&query(8), Direction::Reverse, 5)
        .expect_err("missing reverse");
    std::fs::write(root.join("missing-after.txt"), path_state(&b_path)).unwrap();
    std::fs::write(root.join("missing-result.txt"), err.code).unwrap();
}

fn edge_corrupt_reverse(root: &Path) {
    let vault = root.join("corrupt_reverse");
    let (a, b) = dual_rows(32, 8);
    let index = build_dual_with_search(&vault, 0, &a, &b, params(8), search_params(32)).unwrap();
    let b_path = dual_graph_path(&vault, 0, Direction::Reverse);
    std::fs::write(root.join("corrupt-before.txt"), first_bytes(&b_path)).unwrap();
    let mut bytes = std::fs::read(&b_path).unwrap();
    bytes[0] ^= 0xff;
    std::fs::write(&b_path, bytes).unwrap();
    let err = index
        .search_directional(&query(8), Direction::Reverse, 5)
        .expect_err("corrupt reverse");
    std::fs::write(root.join("corrupt-after.txt"), first_bytes(&b_path)).unwrap();
    std::fs::write(root.join("corrupt-result.txt"), err.code).unwrap();
}

fn edge_invalid_boost(root: &Path) {
    let vault = root.join("invalid_boost");
    let (a, b) = dual_rows(16, 8);
    let index = build_dual_with_search(&vault, 0, &a, &b, params(8), search_params(16)).unwrap();
    std::fs::write(
        root.join("boost-before.txt"),
        dir_listing(&vault.join("idx")),
    )
    .unwrap();
    let err = index
        .search_merged(
            &query(8),
            5,
            DirectionalBoost {
                forward_weight: 0.2,
                reverse_weight: 0.2,
            },
        )
        .expect_err("invalid boost");
    std::fs::write(
        root.join("boost-after.txt"),
        dir_listing(&vault.join("idx")),
    )
    .unwrap();
    std::fs::write(root.join("boost-result.txt"), err.code).unwrap();
}

fn edge_k_above_count(root: &Path) {
    let vault = root.join("k_above_count");
    let (a, b) = dual_rows(8, 8);
    let index = build_dual_with_search(&vault, 0, &a, &b, params(8), search_params(8)).unwrap();
    std::fs::write(root.join("kgt-before.txt"), dir_listing(&vault.join("idx"))).unwrap();
    let hits = index
        .search_merged(&query(8), 50, DirectionalBoost::default())
        .expect("k above count");
    std::fs::write(root.join("kgt-after.txt"), dir_listing(&vault.join("idx"))).unwrap();
    std::fs::write(
        root.join("kgt-result.txt"),
        format!("returned_hits={}\n", hits.len()),
    )
    .unwrap();
}

fn hit_report(forward: &[(u32, f32)], reverse: &[(u32, f32)]) -> String {
    let mut rows = vec!["direction,rank,local_id,score".to_string()];
    rows.extend(
        forward
            .iter()
            .enumerate()
            .map(|(idx, (id, score))| format!("forward,{},{id},{score:.6}", idx + 1)),
    );
    rows.extend(
        reverse
            .iter()
            .enumerate()
            .map(|(idx, (id, score))| format!("reverse,{},{id},{score:.6}", idx + 1)),
    );
    rows.join("\n")
}

fn merged_report(hits: &[(u32, f32)]) -> String {
    let mut rows = vec!["rank,local_id,weighted_score".to_string()];
    rows.extend(
        hits.iter()
            .enumerate()
            .map(|(idx, (id, score))| format!("{},{id},{score:.6}", idx + 1)),
    );
    rows.join("\n")
}

fn file_state(path: &Path) -> String {
    let bytes = std::fs::read(path).expect("read file");
    format!(
        "exists=true size={} blake3={}\n",
        bytes.len(),
        blake3::hash(&bytes)
    )
}

fn path_state(path: &Path) -> String {
    format!("exists={}\n", path.exists())
}

fn first_bytes(path: &Path) -> String {
    let bytes = std::fs::read(path).expect("read bytes");
    bytes[..16.min(bytes.len())]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn dir_listing(dir: &Path) -> String {
    let mut rows = Vec::new();
    for entry in std::fs::read_dir(dir).expect("read dir") {
        let entry = entry.expect("entry");
        let path = entry.path();
        if path.is_dir() {
            for child in std::fs::read_dir(path).expect("read child dir") {
                let child = child.expect("child");
                rows.push(format!(
                    "{}/{} {} bytes",
                    entry.file_name().to_string_lossy(),
                    child.file_name().to_string_lossy(),
                    child.metadata().expect("metadata").len()
                ));
            }
        }
    }
    rows.sort();
    rows.join("\n")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    #[test]
    fn merged_results_have_no_phantom_ids(weight in 0.0_f32..=1.0) {
        let index = build_index("prop", 32);
        let boost = DirectionalBoost::new(weight, 1.0 - weight).expect("valid boost");
        let hits = index.search_merged(&query(8), 20, boost).expect("search");

        prop_assert!(hits.iter().all(|(id, _)| *id < 32));
        let distinct: BTreeSet<_> = hits.iter().map(|(id, _)| *id).collect();
        prop_assert_eq!(distinct.len(), hits.len());
    }
}
