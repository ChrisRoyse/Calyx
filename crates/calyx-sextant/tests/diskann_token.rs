//! PH68 gap #604 - token DiskANN + segmented MaxSim tests.

use std::path::PathBuf;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use calyx_core::{CxId, SlotId, SlotVector};
use calyx_sextant::index::{
    DiskAnnBuildParams, DiskAnnSearchParams, SextantIndex, TokenDiskAnnMaxSim,
};
use sextant_support::cx_usize_be as cx;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("calyx-diskann-token")
        .join(format!("{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch");
    dir
}

fn token(doc: usize, tok: usize, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|axis| {
            let anchor = if axis == (doc + tok) % dim { 3.0 } else { 0.0 };
            anchor + ((doc * 13 + tok * 7 + axis) % 11) as f32 * 0.01
        })
        .collect()
}

fn rows(docs: usize, tokens: usize, dim: usize) -> Vec<(CxId, Vec<Vec<f32>>)> {
    (0..docs)
        .map(|doc| {
            (
                cx(doc),
                (0..tokens).map(|tok| token(doc, tok, dim)).collect(),
            )
        })
        .collect()
}

fn build_params(dim: usize) -> DiskAnnBuildParams {
    DiskAnnBuildParams {
        dim,
        m_max: 8,
        ef_construction: 32,
        alpha: 1.2,
    }
}

fn search_params() -> DiskAnnSearchParams {
    DiskAnnSearchParams {
        beamwidth: 16,
        ef_search: 64,
        rescore_k: 64,
        rescore_from_raw: false,
    }
}

#[test]
fn token_diskann_maxsim_reopens_and_ranks_exact_doc_first() {
    let root = scratch("happy").join("idx/slot_00.token.ann");
    let rows = rows(16, 3, 4);
    let index = TokenDiskAnnMaxSim::build(
        SlotId::new(0),
        &root,
        &rows,
        build_params(4),
        search_params(),
    )
    .expect("build token index");

    for name in ["graph.cda", "docs.cdt", "token_docs.u32", "tokens.f32"] {
        assert!(root.join(name).is_file(), "{name} sidecar is persisted");
    }
    let reopened =
        TokenDiskAnnMaxSim::open(SlotId::new(0), &root, search_params(), 64).expect("reopen");
    let query = SlotVector::Multi {
        token_dim: 4,
        tokens: rows[7].1.clone(),
    };
    let hits = reopened.search(&query, 5, Some(64)).expect("search");

    assert_eq!(hits[0].cx_id, rows[7].0);
    assert_eq!(hits[0].rank, 1);
    assert_eq!(reopened.stats().kind, "token_diskann_maxsim");
    assert_eq!(index.vector(rows[7].0), Some(query));
}

#[test]
fn empty_token_rows_fail_without_graph_file() {
    let root = scratch("empty").join("idx/slot_00.token.ann");
    let err =
        TokenDiskAnnMaxSim::build(SlotId::new(0), &root, &[], build_params(4), search_params())
            .expect_err("empty rows fail");

    assert_eq!(err.code, "CALYX_INDEX_INVALID_PARAMS");
    assert!(!root.join("graph.cda").exists());
}

#[test]
fn token_query_dim_mismatch_fails_closed() {
    let root = scratch("dim").join("idx/slot_00.token.ann");
    let rows = rows(8, 2, 4);
    let index = TokenDiskAnnMaxSim::build(
        SlotId::new(0),
        &root,
        &rows,
        build_params(4),
        search_params(),
    )
    .expect("build token index");
    let err = index
        .search(
            &SlotVector::Multi {
                token_dim: 3,
                tokens: vec![vec![1.0, 2.0, 3.0]],
            },
            3,
            Some(32),
        )
        .expect_err("dim mismatch");

    assert_eq!(err.code, "CALYX_SEXTANT_VECTOR_SHAPE");
}

#[test]
fn missing_token_sidecar_fails_closed_on_open() {
    let root = scratch("missing").join("idx/slot_00.token.ann");
    let rows = rows(8, 2, 4);
    TokenDiskAnnMaxSim::build(
        SlotId::new(0),
        &root,
        &rows,
        build_params(4),
        search_params(),
    )
    .expect("build token index");
    std::fs::remove_file(root.join("tokens.f32")).expect("remove token bytes");

    let err = TokenDiskAnnMaxSim::open(SlotId::new(0), &root, search_params(), 16)
        .expect_err("missing sidecar must fail");

    assert_eq!(err.code, "CALYX_INDEX_IO");
}
