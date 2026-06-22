//! PH68 gap #604 - concat cross-term DiskANN tests.

use std::path::PathBuf;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use calyx_core::SlotId;
use calyx_sextant::index::{
    ConcatCrossTermDiskAnn, ConcatCrossTermKey, DiskAnnBuildParams, DiskAnnSearchParams,
};
use sextant_support::cx_usize_be as cx;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("calyx-diskann-concat")
        .join(format!("{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch");
    dir
}

fn vector(row: usize, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|axis| {
            let anchor = if axis == row % dim { 4.0 } else { 0.0 };
            anchor + ((row * 17 + axis * 5) % 13) as f32 * 0.01
        })
        .collect()
}

fn rows(count: usize, dim: usize) -> Vec<(ConcatCrossTermKey, Vec<f32>)> {
    (0..count)
        .map(|idx| {
            (
                ConcatCrossTermKey {
                    cx_id: cx(idx),
                    a: SlotId::new(1),
                    b: SlotId::new(2),
                },
                vector(idx, dim),
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
        rescore_k: 32,
        rescore_from_raw: false,
    }
}

#[test]
fn concat_diskann_reopens_and_returns_xterm_key() {
    let root = scratch("happy").join("idx/xterm.concat.ann");
    let rows = rows(32, 6);
    ConcatCrossTermDiskAnn::build(&root, &rows, build_params(6), search_params())
        .expect("build concat index");

    let magic = std::fs::read(root.join("keys.cdx")).expect("read keys sidecar");
    assert_eq!(&magic[..8], b"CLXXTRM1");
    let reopened = ConcatCrossTermDiskAnn::open(&root, search_params()).expect("reopen");
    let hits = reopened
        .search_terms(&rows[11].1, 5, Some(64))
        .expect("search concat");

    assert_eq!(hits[0].key, rows[11].0);
    assert_eq!(hits[0].key.a, SlotId::new(1));
    assert_eq!(hits[0].key.b, SlotId::new(2));
    assert!(root.join("graph.cda").is_file());
}

#[test]
fn empty_concat_rows_fail_without_graph_file() {
    let root = scratch("empty").join("idx/xterm.concat.ann");
    let err = ConcatCrossTermDiskAnn::build(&root, &[], build_params(4), search_params())
        .expect_err("empty rows fail");

    assert_eq!(err.code, "CALYX_INDEX_INVALID_PARAMS");
    assert!(!root.join("graph.cda").exists());
}

#[test]
fn concat_query_dim_mismatch_fails_closed() {
    let root = scratch("dim").join("idx/xterm.concat.ann");
    let rows = rows(12, 4);
    let index = ConcatCrossTermDiskAnn::build(&root, &rows, build_params(4), search_params())
        .expect("build concat index");
    let err = index
        .search_terms(&[1.0, 2.0, 3.0], 3, Some(32))
        .expect_err("dim mismatch");

    assert_eq!(err.code, "CALYX_INDEX_DIM_MISMATCH");
}
