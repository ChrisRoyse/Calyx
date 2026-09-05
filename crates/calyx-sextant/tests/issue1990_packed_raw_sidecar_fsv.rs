//! Issue #1990 FSV: the diskann raw-f32 vector sidecar (`graph.raw`) is a
//! single packed regular file (header + fixed `dim*4`-byte f32 records), not a
//! directory of one file per node. This test proves, against a real small index
//! built from real f32 vectors:
//!
//! 1. packed readback is byte-identical for id 0, id n-1, and an id whose vector
//!    contains negative and denormal components;
//! 2. the legacy v1 directory-of-files layout (the real pre-#1990 deployed
//!    format, written directly here) still reads correctly through the
//!    versioned reader;
//! 3. a truncated packed file fails closed with a typed `CALYX_INDEX_CORRUPT`
//!    error and never zero-fills.
//!
//! Building an index routes through the `calyx_aster::private_fs` owner-only
//! creation primitives, which have no non-Unix contract, so the whole file
//! compiles to an empty binary off Unix (mirroring the #1989 FSV test).
#![cfg(unix)]

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use calyx_core::{CxId, SlotId, SlotVector};
use calyx_sextant::index::{
    DiskAnnBuildParams, DiskAnnSearch, DiskAnnSearchParams, RAW_SIDECAR_HEADER_SIZE,
    RAW_SIDECAR_MAGIC, SextantIndex,
};

const DIM: usize = 8;
const NODES: usize = 48;
/// The id whose vector we deliberately load with negative and denormal
/// components to prove exact byte round-tripping of awkward bit patterns.
const NEGATIVE_ID: usize = 7;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("calyx-issue1990-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn cx(id: usize) -> CxId {
    let mut bytes = [0_u8; 16];
    bytes[8..].copy_from_slice(&(id as u64).to_be_bytes());
    CxId::from_bytes(bytes)
}

/// Deterministic real f32 vectors with a mix of signs and a distinct dominant
/// axis per row (so the graph is non-trivial), plus one row deliberately
/// carrying a strong negative and a denormal component.
fn make_rows() -> Vec<(CxId, Vec<f32>)> {
    let mut rows: Vec<(CxId, Vec<f32>)> = (0..NODES)
        .map(|id| {
            let mut vector: Vec<f32> = (0..DIM)
                .map(|axis| {
                    let base = ((id * 7 + axis * 13) % 17) as f32 - 8.0;
                    base * 0.125 + if (id + axis) % 3 == 0 { -0.5 } else { 0.5 }
                })
                .collect();
            vector[id % DIM] += 4.0;
            (cx(id), vector)
        })
        .collect();
    // Awkward-but-finite bit patterns that must survive the packed round trip.
    rows[NEGATIVE_ID].1[0] = -3.5;
    rows[NEGATIVE_ID].1[1] = f32::from_bits(1); // smallest positive denormal
    assert!(
        rows[NEGATIVE_ID].1.iter().any(|value| *value < 0.0),
        "the negative-id fixture must contain a negative component"
    );
    rows
}

fn build_params() -> DiskAnnBuildParams {
    DiskAnnBuildParams {
        dim: DIM,
        m_max: 16,
        ef_construction: 64,
        alpha: 1.2,
    }
}

fn search_params() -> DiskAnnSearchParams {
    DiskAnnSearchParams {
        beamwidth: 16,
        ef_search: 64,
        rescore_k: 16,
        rescore_from_raw: true,
    }
}

fn graph_path(dir: &Path) -> PathBuf {
    let path = dir.join("idx").join("slot_00.ann").join("graph.cda");
    fs::create_dir_all(path.parent().unwrap()).expect("create .ann dir");
    path
}

fn assert_dense_bit_identical(vector: &SlotVector, expected: &[f32]) {
    let SlotVector::Dense { dim, data } = vector else {
        panic!("expected a dense vector");
    };
    assert_eq!(*dim as usize, expected.len(), "unexpected vector dim");
    for (axis, (got, want)) in data.iter().zip(expected).enumerate() {
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "axis {axis} differs: got {got:?} want {want:?}"
        );
    }
}

/// The legacy v1 layout: a directory holding one file per node named by decimal
/// id, each `dim*4` little-endian f32 bytes. Written directly here because this
/// is the real deployed pre-#1990 on-disk format, not a mock.
fn write_legacy_dir(dir: &Path, rows: &[(CxId, Vec<f32>)]) {
    fs::create_dir_all(dir).expect("create legacy sidecar dir");
    for (id, (_, vector)) in rows.iter().enumerate() {
        let bytes: Vec<u8> = vector
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        fs::write(dir.join(id.to_string()), bytes).expect("write legacy sidecar file");
    }
}

#[test]
fn packed_sidecar_reads_back_byte_identical() {
    let dir = scratch("packed");
    let graph = graph_path(&dir);
    let rows = make_rows();

    let index = DiskAnnSearch::build(
        SlotId::new(0),
        &graph,
        &rows,
        build_params(),
        None,
        search_params(),
    )
    .expect("build packed index");

    // The sidecar is one packed regular file of exactly header + n*dim*4 bytes.
    let raw = graph.with_extension("raw");
    let meta = fs::symlink_metadata(&raw).expect("stat packed sidecar");
    assert!(
        meta.file_type().is_file(),
        "graph.raw must be a regular file"
    );
    assert!(
        !meta.file_type().is_symlink(),
        "graph.raw must never be a symlink"
    );
    assert_eq!(
        meta.len(),
        (RAW_SIDECAR_HEADER_SIZE + NODES * DIM * 4) as u64,
        "packed sidecar size must be header + n*dim*4"
    );
    let bytes = fs::read(&raw).expect("read packed sidecar");
    assert_eq!(&bytes[0..8], &RAW_SIDECAR_MAGIC, "packed sidecar magic");

    // Byte-identical readback through the packed path for the three edges.
    for &id in &[0, NEGATIVE_ID, NODES - 1] {
        let vector = index
            .vector(cx(id))
            .unwrap_or_else(|| panic!("packed vector for id {id} missing"));
        assert_dense_bit_identical(&vector, &rows[id].1);
    }

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn legacy_directory_sidecar_still_reads() {
    let dir = scratch("legacy");
    let graph = graph_path(&dir);
    let rows = make_rows();

    // Build the graph (this also emits a packed sidecar next to graph.cda).
    DiskAnnSearch::build(
        SlotId::new(0),
        &graph,
        &rows,
        build_params(),
        None,
        search_params(),
    )
    .expect("build graph for legacy test");

    // Hand-build a v1 directory fixture at a separate path and open the graph
    // against it, exercising the directory branch of the versioned reader.
    let legacy = dir.join("cf").join("slot_00.raw");
    write_legacy_dir(&legacy, &rows);
    assert!(legacy.is_dir(), "legacy fixture must be a directory");

    let index = DiskAnnSearch::open(
        SlotId::new(0),
        &graph,
        rows.iter().map(|(cx_id, _)| *cx_id).collect(),
        Some(legacy.clone()),
        search_params(),
    )
    .expect("open against legacy v1 dir");

    for &id in &[0, NEGATIVE_ID, NODES - 1] {
        let vector = index
            .vector(cx(id))
            .unwrap_or_else(|| panic!("legacy vector for id {id} missing"));
        assert_dense_bit_identical(&vector, &rows[id].1);
    }

    // A search that rescores from the legacy sidecar must return real hits.
    let hits = index
        .search_ids(&rows[NEGATIVE_ID].1, 10, &search_params())
        .expect("legacy-dir rescore search");
    assert!(!hits.is_empty(), "legacy-dir rescore returned no hits");

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn truncated_packed_sidecar_fails_closed() {
    let dir = scratch("trunc");
    let graph = graph_path(&dir);
    let rows = make_rows();

    let index = DiskAnnSearch::build(
        SlotId::new(0),
        &graph,
        &rows,
        build_params(),
        None,
        search_params(),
    )
    .expect("build packed index");

    // Chop the tail so the file length no longer matches header + n*dim*4.
    let raw = graph.with_extension("raw");
    let len = fs::metadata(&raw).expect("stat packed sidecar").len();
    OpenOptions::new()
        .write(true)
        .open(&raw)
        .expect("open packed sidecar for truncation")
        .set_len(len - 4)
        .expect("truncate packed sidecar");

    let error = index
        .search_ids(&rows[NEGATIVE_ID].1, 10, &search_params())
        .expect_err("truncated packed sidecar must fail closed");
    assert_eq!(
        error.code, "CALYX_INDEX_CORRUPT",
        "expected typed corrupt error, got {}: {}",
        error.code, error.message
    );

    fs::remove_dir_all(&dir).unwrap();
}
