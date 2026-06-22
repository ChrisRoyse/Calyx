//! PH68 T01 — DiskANN graph format + builder tests (issue #545).
//!
//! Graph-constructing tests are `#[ignore = "server-only"]` per the PH68 scale
//! boundary and run explicitly in a manual verification run:
//! `cargo test -p calyx-sextant --test diskann_graph -- --include-ignored`

use std::path::PathBuf;

use calyx_sextant::index::diskann::graph::{
    DISKANN_BLOCK_ALIGN, DISKANN_FORMAT_VERSION, DISKANN_MAGIC, DISKANN_NODE_ALIGN,
    DiskAnnVectorRef,
};
use calyx_sextant::index::{
    DiskAnnBuildParams, build_diskann_graph, node_block_size, open_diskann_graph,
};
use proptest::prelude::*;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::var("CALYX_DISKANN_FSV_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-diskann-t01"))
        .join(format!("{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn synthetic_vectors(n: usize, dim: usize, seed: u64) -> Vec<(u32, Vec<f32>)> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    (0..n)
        .map(|i| {
            (
                u32::try_from(i).expect("test n fits u32"),
                (0..dim).map(|_| rng.gen_range(-1.0_f32..1.0)).collect(),
            )
        })
        .collect()
}

fn params(dim: usize, m_max: usize) -> DiskAnnBuildParams {
    DiskAnnBuildParams {
        dim,
        m_max,
        ef_construction: 64,
        alpha: 1.2,
    }
}

fn expected_i8_direction(vector: &[f32]) -> Vec<i8> {
    let max_abs = vector
        .iter()
        .fold(0.0_f32, |acc, value| acc.max(value.abs()));
    if max_abs == 0.0 {
        return vec![0; vector.len()];
    }
    let scale = 127.0 / max_abs;
    vector
        .iter()
        .map(|value| (value * scale).round().clamp(-127.0, 127.0) as i8)
        .collect()
}

fn assert_i8_direction(node: DiskAnnVectorRef<'_>, expected: &[f32], label: &str) {
    let DiskAnnVectorRef::I8(actual) = node else {
        panic!("{label} should be a v3 i8 directional graph node");
    };
    assert_eq!(actual, expected_i8_direction(expected), "{label}");
}

#[test]
fn node_block_size_is_cacheline_multiple_for_known_pairs() {
    for (dim, m_max) in [(4, 8), (128, 32), (768, 64), (1536, 48)] {
        let size = node_block_size(dim, m_max);
        assert_eq!(
            size % DISKANN_NODE_ALIGN,
            0,
            "({dim},{m_max}) -> {size} not a 64-byte multiple"
        );
        assert!(size >= dim.div_ceil(4) * 4 + 4 + m_max * 4);
    }
    // Hand-computed (2+2=4 discipline): dim=4, m_max=8 -> 4+4+32 = 40 -> 64.
    assert_eq!(node_block_size(4, 8), 64);
    // dim=768, m_max=64 -> 768+4+256 = 1028 -> 1088.
    assert_eq!(node_block_size(768, 64), 1088);
    // dim=1536, m_max=48 -> 1536+4+192 = 1732 -> 1792.
    assert_eq!(node_block_size(1536, 48), 1792);
}

#[test]
fn empty_input_is_invalid_params_and_writes_nothing() {
    let dir = scratch("empty");
    let path = dir.join("graph.cda");
    let err = build_diskann_graph(&path, &[], params(4, 8)).expect_err("empty must fail");
    assert_eq!(err.code, "CALYX_INDEX_INVALID_PARAMS");
    assert!(
        !path.exists(),
        "no graph file may be written on invalid input"
    );
    assert!(!path.with_extension("cda.tmp").exists(), "no tmp residue");
}

#[test]
fn m_max_zero_is_invalid_params() {
    let dir = scratch("mmax0");
    let path = dir.join("graph.cda");
    let vectors = synthetic_vectors(8, 4, 42);
    let err = build_diskann_graph(&path, &vectors, params(4, 0)).expect_err("m_max=0 must fail");
    assert_eq!(err.code, "CALYX_INDEX_INVALID_PARAMS");
    assert!(!path.exists());
}

#[test]
fn non_dense_ids_are_invalid_params() {
    let dir = scratch("sparse-ids");
    let path = dir.join("graph.cda");
    let mut vectors = synthetic_vectors(4, 4, 42);
    vectors[2].0 = 7; // gap
    let err = build_diskann_graph(&path, &vectors, params(4, 8)).expect_err("gap ids must fail");
    assert_eq!(err.code, "CALYX_INDEX_INVALID_PARAMS");
    assert!(!path.exists());
}

#[test]
fn non_finite_vector_is_invalid_params() {
    let dir = scratch("nan");
    let path = dir.join("graph.cda");
    let mut vectors = synthetic_vectors(4, 4, 42);
    vectors[1].1[2] = f32::NAN;
    let err = build_diskann_graph(&path, &vectors, params(4, 8)).expect_err("NaN must fail");
    assert_eq!(err.code, "CALYX_INDEX_INVALID_PARAMS");
    assert!(!path.exists());
}

#[test]
#[ignore = "server-only"]
fn hundred_node_graph_round_trips_quantized_direction() {
    let dir = scratch("rt100");
    let path = dir.join("graph.cda");
    let vectors = synthetic_vectors(100, 4, 42);
    build_diskann_graph(&path, &vectors, params(4, 8)).expect("build");
    let reader = open_diskann_graph(&path).expect("open");
    assert_eq!(reader.node_count(), 100);
    for (id, vector) in &vectors {
        let node = reader.read_node(*id).expect("read node");
        assert_i8_direction(node.vector, vector, &format!("node {id} direction"));
        assert!(
            node.neighbors.len() <= 8,
            "node {id} degree {} > m_max",
            node.neighbors.len()
        );
        assert!(node.neighbors.iter().all(|&n| n < 100 && n != *id));
    }
}

#[test]
#[ignore = "server-only"]
fn header_round_trips_all_fields() {
    let dir = scratch("header");
    let path = dir.join("graph.cda");
    let vectors = synthetic_vectors(50, 16, 7);
    build_diskann_graph(&path, &vectors, params(16, 12)).expect("build");
    let reader = open_diskann_graph(&path).expect("open");
    let header = reader.header();
    assert_eq!(header.format_version, DISKANN_FORMAT_VERSION);
    assert_eq!(header.dim, 16);
    assert_eq!(header.m_max, 12);
    assert_eq!(header.node_count, 50);
    assert!(header.max_degree <= 12);
    assert!(u64::from(header.entry_point_id) < 50);
    // Magic on disk, independently of the reader.
    let bytes = std::fs::read(&path).expect("read raw");
    assert_eq!(&bytes[0..8], DISKANN_MAGIC.as_slice());
    assert_eq!(
        bytes.len(),
        DISKANN_BLOCK_ALIGN + 50 * node_block_size(16, 12),
        "file len == header block + node_count x node_block_size"
    );
}

#[test]
#[ignore = "server-only"]
fn single_node_graph_is_parseable_with_empty_neighbors() {
    let dir = scratch("single");
    let path = dir.join("graph.cda");
    let vectors = synthetic_vectors(1, 4, 42);
    build_diskann_graph(&path, &vectors, params(4, 8)).expect("build single");
    let reader = open_diskann_graph(&path).expect("open single");
    assert_eq!(reader.header().entry_point_id, 0);
    assert_eq!(reader.node_count(), 1);
    let node = reader.read_node(0).expect("read node 0");
    assert_i8_direction(node.vector, &vectors[0].1, "single node direction");
    assert!(node.neighbors.is_empty(), "single node has no neighbors");
}

#[test]
#[ignore = "server-only"]
fn flipped_magic_byte_is_corrupt_not_panic() {
    let dir = scratch("magic-flip");
    let path = dir.join("graph.cda");
    let vectors = synthetic_vectors(10, 4, 42);
    build_diskann_graph(&path, &vectors, params(4, 8)).expect("build");
    let mut bytes = std::fs::read(&path).expect("read");
    bytes[0] ^= 0xFF;
    std::fs::write(&path, &bytes).expect("rewrite");
    let err = open_diskann_graph(&path).expect_err("flipped magic must fail closed");
    assert_eq!(err.code, "CALYX_INDEX_CORRUPT");
}

#[test]
#[ignore = "server-only"]
fn truncated_file_is_corrupt() {
    let dir = scratch("truncate");
    let path = dir.join("graph.cda");
    let vectors = synthetic_vectors(10, 4, 42);
    build_diskann_graph(&path, &vectors, params(4, 8)).expect("build");
    let bytes = std::fs::read(&path).expect("read");
    std::fs::write(&path, &bytes[..bytes.len() - 4096]).expect("truncate");
    let err = open_diskann_graph(&path).expect_err("truncated file must fail closed");
    assert_eq!(err.code, "CALYX_INDEX_CORRUPT");
}

#[test]
#[ignore = "server-only"]
fn read_node_out_of_range_is_invalid_params() {
    let dir = scratch("oob");
    let path = dir.join("graph.cda");
    let vectors = synthetic_vectors(10, 4, 42);
    build_diskann_graph(&path, &vectors, params(4, 8)).expect("build");
    let reader = open_diskann_graph(&path).expect("open");
    let err = reader.read_node(10).expect_err("id 10 of 0..10 must fail");
    assert_eq!(err.code, "CALYX_INDEX_INVALID_PARAMS");
}

/// PH68 T01 FSV (issue #545): 1000-node graph in a manual verification run hotpool NVMe.
/// SoT: `$CALYX_DISKANN_FSV_SOT` (e.g. `/zfs/hot/calyx/fsv-issue545/idx/slot_00.ann/graph.cda`).
/// Prints hand-computed expected size vs actual bytes for independent `xxd`/`ls` readback.
#[test]
#[ignore = "server-only"]
fn fsv_issue545_thousand_node_graph_i8_v3() {
    let path = PathBuf::from(
        std::env::var("CALYX_DISKANN_FSV_SOT")
            .expect("set CALYX_DISKANN_FSV_SOT to the graph.cda SoT path"),
    );
    let (dim, m_max, n) = (128_usize, 32_usize, 1000_usize);
    let vectors = synthetic_vectors(n, dim, 42);
    build_diskann_graph(&path, &vectors, params(dim, m_max)).expect("build 1000-node graph");
    // Hand-computed: block = 128+4+32*4 = 260 -> 320; file = 4096 + 1000*320.
    let expected = DISKANN_BLOCK_ALIGN + n * node_block_size(dim, m_max);
    assert_eq!(node_block_size(dim, m_max), 320);
    assert_eq!(expected, 324_096);
    let actual = std::fs::metadata(&path).expect("stat SoT").len();
    println!("FSV SoT: {}", path.display());
    println!("FSV expected file size: {expected} B; actual: {actual} B");
    assert_eq!(actual, expected as u64);
    let reader = open_diskann_graph(&path).expect("open SoT");
    println!("FSV header: {:?}", reader.header());
    for id in [0_u32, 999] {
        let node = reader.read_node(id).expect("read node");
        let expected = expected_i8_direction(&vectors[id as usize].1);
        let DiskAnnVectorRef::I8(vector) = node.vector else {
            panic!("node {id} should be v3 i8");
        };
        assert_eq!(vector, expected.as_slice(), "node {id} direction");
        assert!(node.neighbors.len() <= m_max);
        println!(
            "FSV node {id}: first i8 = {} (expected {}), degree = {}",
            vector[0],
            expected[0],
            node.neighbors.len()
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn node_block_size_never_truncates(dim in 1_usize..=2048, m_max in 1_usize..=96) {
        let size = node_block_size(dim, m_max);
        prop_assert!(size >= dim.div_ceil(4) * 4 + 4 + m_max * 4);
        prop_assert_eq!(size % DISKANN_NODE_ALIGN, 0);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    #[test]
    #[ignore = "server-only"]
    fn build_preserves_direction_as_i8(
        n in 2_usize..40,
        dim in 1_usize..32,
        m_max in 1_usize..16,
    ) {
        let dir = scratch(&format!("prop-{n}-{dim}-{m_max}"));
        let path = dir.join("graph.cda");
        let vectors = synthetic_vectors(n, dim, 42);
        build_diskann_graph(&path, &vectors, DiskAnnBuildParams {
            dim, m_max, ef_construction: 32, alpha: 1.2,
        }).expect("build");
        let reader = open_diskann_graph(&path).expect("open");
        for (id, vector) in &vectors {
            let node = reader.read_node(*id).expect("read");
            let DiskAnnVectorRef::I8(actual) = node.vector else {
                return Err(TestCaseError::fail("node should be v3 i8"));
            };
            let expected = expected_i8_direction(vector);
            prop_assert_eq!(actual, expected.as_slice());
            prop_assert!(node.neighbors.len() <= m_max);
        }
    }
}
