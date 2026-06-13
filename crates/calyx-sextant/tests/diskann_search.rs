//! PH68 T02 - DiskANN beam search + raw-f32 rescore tests (issue #546).

use std::collections::BTreeSet;
use std::path::PathBuf;

use calyx_core::{CxId, SlotId, SlotVector};
use calyx_sextant::index::{DiskAnnBuildParams, DiskAnnSearch, DiskAnnSearchParams, SextantIndex};
use proptest::prelude::*;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("calyx-diskann-t02")
        .join(format!("{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn cx(idx: usize) -> CxId {
    let mut bytes = [0_u8; 16];
    bytes[8..16].copy_from_slice(&(idx as u64).to_be_bytes());
    CxId::from_bytes(bytes)
}

fn vectors(n: usize, dim: usize, seed: u64) -> Vec<(CxId, Vec<f32>)> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    (0..n)
        .map(|idx| {
            let mut vector: Vec<f32> = (0..dim).map(|_| rng.gen_range(-1.0..1.0)).collect();
            vector[idx % dim] += 4.0;
            (cx(idx), vector)
        })
        .collect()
}

fn build_params(dim: usize) -> DiskAnnBuildParams {
    DiskAnnBuildParams {
        dim,
        m_max: 16,
        ef_construction: 64,
        alpha: 1.2,
    }
}

fn search_params(k: usize, ef: usize, raw: bool) -> DiskAnnSearchParams {
    DiskAnnSearchParams {
        beamwidth: 32,
        ef_search: ef,
        rescore_k: k.max(1),
        rescore_from_raw: raw,
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let (dot, aa, bb) = a
        .iter()
        .zip(b)
        .fold((0.0_f32, 0.0_f32, 0.0_f32), |(dot, aa, bb), (x, y)| {
            (dot + x * y, aa + x * x, bb + y * y)
        });
    if aa == 0.0 || bb == 0.0 {
        0.0
    } else {
        dot / (aa.sqrt() * bb.sqrt())
    }
}

fn build_index(tag: &str, rows: &[(CxId, Vec<f32>)]) -> DiskAnnSearch {
    let dir = scratch(tag);
    DiskAnnSearch::build(
        SlotId::new(0),
        dir.join("idx/slot_00.ann/graph.cda"),
        rows,
        build_params(rows[0].1.len()),
        None,
        search_params(32, 64, false),
    )
    .expect("build diskann search")
}

#[test]
fn thousand_node_search_returns_ordered_nonnegative_distances() {
    let rows = vectors(1000, 128, 42);
    let index = build_index("ordered1000", &rows);

    let hits = index
        .search_ids(&rows[17].1, 10, &search_params(10, 64, false))
        .expect("search");

    assert_eq!(hits.len(), 10);
    assert!(hits.iter().all(|(id, _)| *id < 1000));
    assert!(hits.iter().all(|(_, distance)| *distance >= 0.0));
    assert!(hits.windows(2).all(|pair| pair[0].1 <= pair[1].1));
}

#[test]
fn planted_exact_query_returns_node_seven_rank_zero() {
    let rows = vectors(128, 32, 7);
    let index = build_index("exact7", &rows);

    let hits = index
        .search_ids(&rows[7].1, 10, &search_params(10, 96, false))
        .expect("search exact");

    assert_eq!(hits[0].0, 7);
    assert!(hits[0].1 <= 1.0e-5, "distance was {}", hits[0].1);
}

#[test]
fn raw_rescore_reads_sidecar_and_matches_exact_distances() {
    let dir = scratch("rescore");
    let raw_dir = dir.join("cf/slot_00.raw");
    std::fs::create_dir_all(&raw_dir).expect("raw dir");
    let raw = vectors(100, 16, 99);
    let approx: Vec<_> = raw
        .iter()
        .map(|(cx_id, vector)| {
            let mut v = vector.clone();
            v[0] = 0.0;
            (*cx_id, v)
        })
        .collect();
    for (idx, (_, vector)) in raw.iter().enumerate() {
        let bytes: Vec<_> = vector.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(raw_dir.join(idx.to_string()), bytes).expect("write raw");
    }
    let index = DiskAnnSearch::build(
        SlotId::new(0),
        dir.join("idx/slot_00.ann/graph.cda"),
        &approx,
        build_params(16),
        Some(raw_dir),
        search_params(20, 80, true),
    )
    .expect("build");

    let hits = index
        .search_ids(&raw[7].1, 5, &search_params(20, 80, true))
        .expect("raw rescore");

    assert_eq!(hits.len(), 5);
    assert!(hits.windows(2).all(|pair| pair[0].1 <= pair[1].1));
    for (id, distance) in hits {
        let exact = 1.0 - cosine(&raw[7].1, &raw[id as usize].1);
        assert!(
            (distance - exact.max(0.0)).abs() <= 1.0e-5,
            "node {id}: {distance} != {exact}"
        );
    }
}

#[test]
fn k_above_node_count_returns_all_nodes() {
    let rows = vectors(8, 8, 3);
    let index = build_index("kgt", &rows);

    let hits = index
        .search_ids(&rows[0].1, 50, &search_params(50, 64, false))
        .expect("search");

    assert_eq!(hits.len(), 8);
}

#[test]
fn empty_graph_returns_empty_hits() {
    let index = DiskAnnSearch::empty(SlotId::new(0), 4, scratch("empty").join("graph.cda"));

    let hits = index
        .search_ids(&[1.0, 0.0, 0.0, 0.0], 10, &search_params(10, 16, false))
        .expect("empty search");

    assert!(hits.is_empty());
}

#[test]
fn query_dim_mismatch_fails_closed() {
    let rows = vectors(16, 8, 11);
    let index = build_index("dim-mismatch", &rows);

    let err = index
        .search_ids(&[1.0, 2.0], 5, &search_params(5, 16, false))
        .expect_err("dimension mismatch");

    assert_eq!(err.code, "CALYX_INDEX_DIM_MISMATCH");
}

#[test]
fn truncated_graph_open_fails_as_index_io_for_search() {
    let rows = vectors(16, 8, 13);
    let dir = scratch("truncate");
    let path = dir.join("idx/slot_00.ann/graph.cda");
    let _ = DiskAnnSearch::build(
        SlotId::new(0),
        &path,
        &rows,
        build_params(8),
        None,
        search_params(10, 32, false),
    )
    .expect("build");
    let bytes = std::fs::read(&path).expect("read graph");
    std::fs::write(&path, &bytes[..bytes.len() - 4096]).expect("truncate");

    let err = DiskAnnSearch::open(
        SlotId::new(0),
        &path,
        rows.iter().map(|(cx_id, _)| *cx_id).collect(),
        None,
        search_params(10, 32, false),
    )
    .expect_err("truncated graph must fail");

    assert_eq!(err.code, "CALYX_INDEX_IO");
}

#[test]
fn sextant_index_adapter_returns_cxid_hits_and_vectors() {
    let rows = vectors(32, 8, 21);
    let mut index = build_index("trait", &rows);
    index.set_base_seq(12);

    let hits = index
        .search(
            &SlotVector::Dense {
                dim: 8,
                data: rows[3].1.clone(),
            },
            4,
            Some(32),
        )
        .expect("trait search");

    assert_eq!(hits.len(), 4);
    assert_eq!(hits[0].rank, 1);
    assert!(
        hits.iter()
            .all(|hit| rows.iter().any(|(id, _)| *id == hit.cx_id))
    );
    assert_eq!(index.stats().kind, "DiskANN");
    assert!(index.vector(rows[3].0).is_some());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    #[test]
    fn search_count_distinct_and_sorted(k in 1_usize..=50, beamwidth in 4_usize..=128) {
        let rows = vectors(48, 12, 17);
        let index = build_index("prop", &rows);
        let ef = beamwidth.max(k).min(128);
        let params = DiskAnnSearchParams {
            beamwidth,
            ef_search: ef,
            rescore_k: k,
            rescore_from_raw: false,
        };

        let hits = index.search_ids(&rows[5].1, k, &params).expect("search");

        assert_eq!(hits.len(), k.min(rows.len()));
        let distinct: BTreeSet<_> = hits.iter().map(|(id, _)| *id).collect();
        assert_eq!(distinct.len(), hits.len());
        prop_assert!(hits.windows(2).all(|pair| pair[0].1 <= pair[1].1));
    }
}
