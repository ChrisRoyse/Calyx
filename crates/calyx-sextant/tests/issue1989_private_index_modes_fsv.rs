//! Issue #1989 FSV: every on-disk DiskANN slot-index artifact must be created
//! owner-only (files `0600`, directories `0700`) at creation time, so opening a
//! freshly built vault never triggers a `harden_tree` fchmod+fsync repair
//! storm.
//!
//! This test owns its own process (registered as a standalone `[[test]]` in
//! `Cargo.toml`) so it can install a deliberately permissive `umask` without
//! racing the process-global state of the aggregated integration harness. The
//! whole file compiles to an empty binary on non-Unix targets, mirroring the
//! `#[cfg(unix)]` split of the `private_fs` primitives it exercises.
#![cfg(unix)]

use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use calyx_core::{CxId, SlotId};
use calyx_sextant::index::{DiskAnnBuildParams, DiskAnnSearch, DiskAnnSearchParams};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

const PRIVATE_FILE_MODE: u32 = 0o600;
const PRIVATE_DIRECTORY_MODE: u32 = 0o700;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("calyx-issue1989-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn real_vectors(n: usize, dim: usize, seed: u64) -> Vec<(CxId, Vec<f32>)> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    (0..n)
        .map(|idx| {
            let mut vector: Vec<f32> = (0..dim).map(|_| rng.random_range(-1.0..1.0)).collect();
            // Give each row a distinct dominant axis so the graph is non-trivial.
            vector[idx % dim] += 4.0;
            let mut bytes = [0_u8; 16];
            bytes[8..].copy_from_slice(&(idx as u64).to_be_bytes());
            (CxId::from_bytes(bytes), vector)
        })
        .collect()
}

fn mode_of(path: &Path) -> u32 {
    std::fs::symlink_metadata(path).unwrap().mode() & 0o777
}

/// Recursively assert every descendant of `root` is owner-only, returning the
/// number of files and directories inspected so the caller can prove the raw
/// sidecar loop actually ran.
fn assert_owner_only(root: &Path) -> (usize, usize) {
    let (mut files, mut dirs) = (0_usize, 0_usize);
    let meta = std::fs::symlink_metadata(root).unwrap();
    assert!(
        !meta.file_type().is_symlink(),
        "{} is a symlink; index writers must never emit links",
        root.display()
    );
    if meta.is_dir() {
        dirs += 1;
        assert_eq!(
            mode_of(root),
            PRIVATE_DIRECTORY_MODE,
            "directory {} expected 0700, observed {:04o}",
            root.display(),
            mode_of(root)
        );
        for entry in std::fs::read_dir(root).unwrap() {
            let (f, d) = assert_owner_only(&entry.unwrap().path());
            files += f;
            dirs += d;
        }
    } else {
        assert!(meta.is_file(), "{} is not a regular file", root.display());
        files += 1;
        assert_eq!(
            mode_of(root),
            PRIVATE_FILE_MODE,
            "file {} expected 0600, observed {:04o}",
            root.display(),
            mode_of(root)
        );
    }
    (files, dirs)
}

#[test]
fn fresh_slot_index_is_owner_only_under_permissive_umask() {
    use nix::sys::stat::{Mode, umask};

    let dir = scratch("modes");
    let ann_dir = dir.join("idx").join("slot_00.ann");
    let graph_path = ann_dir.join("graph.cda");
    // Passing `None` makes the builder emit the default raw sidecar
    // (`graph.cda` -> `graph.raw`). Post-#1990 this is a single packed regular
    // file, not the old per-node 0664 directory: this test proves that one file
    // is created owner-only under a permissive umask.
    let raw_sidecar = graph_path.with_extension("raw");
    let dim = 16_usize;
    let rows = real_vectors(200, dim, 1989);

    // umask 002 is the production-style permissive mask that let 0664/0775
    // descendants leak in before this fix; explicit creation modes must ignore
    // it. Restore the prior mask immediately after the build.
    let previous = umask(Mode::from_bits_truncate(0o002));
    let build = DiskAnnSearch::build(
        SlotId::new(0),
        &graph_path,
        &rows,
        DiskAnnBuildParams {
            dim,
            m_max: 16,
            ef_construction: 64,
            alpha: 1.2,
        },
        None,
        DiskAnnSearchParams {
            beamwidth: 32,
            ef_search: 64,
            rescore_k: 10,
            rescore_from_raw: true,
        },
    );
    umask(previous);
    build.expect("build diskann slot index");

    // The build must have emitted the artifacts named in #1989.
    assert!(graph_path.is_file(), "graph.cda missing");
    assert!(
        ann_dir.join("graph.metric").is_file(),
        "graph.metric marker missing"
    );
    // Post-#1990: the sidecar is one packed regular file, not a directory.
    assert!(
        raw_sidecar.is_file(),
        "graph.raw must be a single packed regular file, not a directory"
    );
    assert!(
        !std::fs::symlink_metadata(&raw_sidecar)
            .unwrap()
            .file_type()
            .is_symlink(),
        "graph.raw must never be a symlink"
    );

    // Its size is exactly header + node_count * dim * 4 bytes.
    let expected_len =
        (calyx_sextant::index::RAW_SIDECAR_HEADER_SIZE + rows.len() * dim * 4) as u64;
    assert_eq!(
        std::fs::metadata(&raw_sidecar).unwrap().len(),
        expected_len,
        "packed sidecar size must be header + n*dim*4"
    );

    // The packed file itself must be owner-only 0600 at creation time.
    assert_eq!(
        mode_of(&raw_sidecar),
        PRIVATE_FILE_MODE,
        "packed graph.raw expected 0600, observed {:04o}",
        mode_of(&raw_sidecar)
    );

    // Walk the whole .ann tree: every file 0600, every dir 0700, no drift.
    // The tree now holds three files (graph.cda, graph.metric, graph.raw) under
    // a single directory (the .ann dir) — no per-node sidecar dir.
    let (files, dirs) = assert_owner_only(&ann_dir);
    assert!(
        files >= 3,
        "expected at least graph.cda + graph.metric + packed graph.raw, saw {files}"
    );
    assert_eq!(dirs, 1, "expected only the .ann dir, saw {dirs}");

    std::fs::remove_dir_all(&dir).unwrap();
}
