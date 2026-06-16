//! PH68 T03 - SPANN centroids in RAM and posting lists on disk.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use calyx_aster::cf::base_key;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, SparseEntry,
    VaultId,
};
use calyx_sextant::index::spann::centroids::SpannCentroidIndex;
use calyx_sextant::index::spann::posting::encode_posting_block;
use calyx_sextant::index::{
    PostingListReader, PostingListWriter, SPANN_CENTROID_MAGIC, SextantIndex, SpannSearch,
    build_centroids,
};
use proptest::prelude::*;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("calyx-spann-t03")
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

fn vectors(n: usize, dim: usize, seed: u64) -> Vec<(u32, Vec<f32>)> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    (0..n)
        .map(|idx| {
            let mut v: Vec<f32> = (0..dim).map(|_| rng.gen_range(-1.0..1.0)).collect();
            v[idx % dim] += 2.0;
            (idx as u32, v)
        })
        .collect()
}

fn postings_for_assignments(
    dir: &PathBuf,
    centroids: &SpannCentroidIndex,
    total: usize,
) -> PostingListWriter {
    let writer = PostingListWriter::new(dir);
    let mut grouped = BTreeMap::<u32, Vec<(u32, f32)>>::new();
    for &(cx_id, centroid_id) in centroids.assignments() {
        let score = 10_000.0 - cx_id as f32 / total as f32;
        grouped.entry(centroid_id).or_default().push((cx_id, score));
    }
    for (centroid_id, mut entries) in grouped {
        entries.sort_by_key(|(id, _)| *id);
        writer
            .write_list(centroid_id, &entries)
            .expect("write list");
    }
    writer
}

#[test]
fn centroid_probe_returns_distinct_ids_under_cluster_count() {
    let rows = vectors(1000, 32, 7);
    let index = build_centroids(&rows, 31, 7);
    let hits = index.nearest_centroids(&rows[17].1, 5);
    let distinct: BTreeSet<_> = hits.iter().copied().collect();

    assert_eq!(hits.len(), 5);
    assert_eq!(distinct.len(), 5);
    assert!(hits.iter().all(|id| *id < 31));
}

#[test]
fn centroid_file_round_trips_first_vector_byte_exact() {
    let dir = scratch("centroid-roundtrip");
    let rows = vectors(128, 16, 11);
    let index = build_centroids(&rows, 12, 11);

    index.save(&dir).expect("save centroids");
    let bytes = std::fs::read(dir.join("centroids.spn")).expect("read raw centroids");
    assert_eq!(&bytes[0..8], SPANN_CENTROID_MAGIC.as_slice());

    let reopened = SpannCentroidIndex::open(&dir).expect("open centroids");
    assert_eq!(reopened.centroid_count(), index.centroid_count());
    let original_bits: Vec<_> = index.centroids()[0].iter().map(|v| v.to_bits()).collect();
    let reopened_bits: Vec<_> = reopened.centroids()[0]
        .iter()
        .map(|v| v.to_bits())
        .collect();
    assert_eq!(reopened_bits, original_bits);
}

#[test]
fn posting_block_round_trips_sorted_ids_and_scores() {
    let dir = scratch("posting-roundtrip");
    let writer = PostingListWriter::new(&dir);
    let mut rng = ChaCha8Rng::seed_from_u64(3);
    let mut next = 0_u32;
    let entries: Vec<_> = (0..200)
        .map(|_| {
            next += rng.gen_range(1..5);
            (next, rng.gen_range(0.0_f32..1.0))
        })
        .collect();

    writer.write_list(7, &entries).expect("write postings");
    let read = PostingListReader::new(&dir)
        .read_list(7)
        .expect("read postings");

    assert_eq!(read.len(), 200);
    assert!(read.windows(2).all(|pair| pair[0].0 < pair[1].0));
    for ((expected_id, expected_score), (actual_id, actual_score)) in entries.iter().zip(read) {
        assert_eq!(actual_id, *expected_id);
        assert!((actual_score - expected_score).abs() <= 1.0e-5);
    }
}

#[test]
fn zstd_block_is_smaller_than_raw_for_repetitive_postings() {
    let dir = scratch("posting-zstd");
    let writer = PostingListWriter::new(&dir);
    let entries: Vec<_> = (0..1000).map(|id| (id, 1.0_f32)).collect();
    let raw = encode_posting_block(&entries).expect("raw block");

    writer.write_list(0, &entries).expect("write compressed");
    let compressed = std::fs::metadata(dir.join("pl_0000.spb"))
        .expect("stat compressed")
        .len() as usize;

    assert!(compressed < raw.len(), "{compressed} >= {}", raw.len());
}

#[test]
fn spann_search_end_to_end_returns_top_k_descending_scores() {
    let rows = vectors(2000, 32, 99);
    let centroids = build_centroids(&rows, 44, 99);
    let dir = scratch("search-e2e");
    postings_for_assignments(&dir, &centroids, rows.len());
    let search = SpannSearch::new(SlotId::new(0), centroids, &dir);

    let hits = search.search(&rows[31].1, 10, 4).expect("search");

    assert_eq!(hits.len(), 10);
    assert!(hits.iter().all(|(id, _)| *id < 2000));
    assert!(hits.windows(2).all(|pair| pair[0].1 >= pair[1].1));
}

#[test]
fn empty_list_and_probe_clamp_are_non_errors() {
    let dir = scratch("edges");
    let reader = PostingListReader::new(&dir);
    assert!(
        reader
            .read_list(99)
            .expect("missing list is empty")
            .is_empty()
    );

    let rows = vectors(64, 8, 31);
    let centroids = build_centroids(&rows, 8, 31);
    let all: Vec<_> = (0..64).map(|id| (id, 100.0 - id as f32)).collect();
    let writer = PostingListWriter::new(&dir);
    for centroid_id in 0..centroids.centroid_count() as u32 {
        writer
            .write_list(centroid_id, &all)
            .expect("write cloned list");
    }
    let search = SpannSearch::new(SlotId::new(0), centroids, &dir);
    let hits = search.search(&rows[0].1, 10, 99).expect("clamped search");

    assert_eq!(hits.len(), 10);
    assert_eq!(
        hits.iter()
            .map(|(id, _)| *id)
            .collect::<BTreeSet<_>>()
            .len(),
        10
    );
}

#[test]
fn corrupted_zstd_and_flipped_centroid_magic_fail_closed() {
    let dir = scratch("corrupt");
    std::fs::write(dir.join("pl_0000.spb"), b"not zstd").expect("write corrupt block");
    let err = PostingListReader::new(&dir)
        .read_list(0)
        .expect_err("corrupt block must fail");
    assert_eq!(err.code, "CALYX_INDEX_CORRUPT");

    let rows = vectors(16, 4, 4);
    let index = build_centroids(&rows, 4, 4);
    index.save(&dir).expect("save centroids");
    let path = dir.join("centroids.spn");
    let mut bytes = std::fs::read(&path).expect("read centroids");
    bytes[0] ^= 0xff;
    std::fs::write(&path, bytes).expect("flip magic");
    let err = SpannCentroidIndex::open(&dir).expect_err("bad magic must fail");
    assert_eq!(err.code, "CALYX_INDEX_CORRUPT");
}

#[test]
fn sextant_index_adapter_routes_sparse_inserts_to_postings() {
    let rows = vectors(8, 8, 13);
    let centroids = build_centroids(&rows, 4, 13);
    let dir = scratch("trait");
    let mut search = SpannSearch::new(SlotId::new(2), centroids, &dir).with_default_n_probe(4);
    let id = cx(7);

    search
        .insert(id, sparse(&[(0, 2.0), (3, 1.0)], 8), 5)
        .expect("insert sparse");
    let hits =
        SextantIndex::search(&search, &sparse(&[(0, 2.0)], 8), 1, Some(4)).expect("trait search");

    assert_eq!(hits[0].cx_id, id);
    assert_eq!(hits[0].rank, 1);
    assert_eq!(search.vector(id), Some(sparse(&[(0, 2.0), (3, 1.0)], 8)));
    assert_eq!(search.stats().kind, "SPANN");
}

#[test]
#[ignore = "server-only FSV trigger writes SPANN files for manual byte readback"]
fn fsv_issue547_writes_centroids_postings_and_search_hits() {
    let (root, vault_root) = fsv_roots();
    std::fs::create_dir_all(&root).expect("create FSV slot dir");
    let rows = vectors(100_000, 32, 547);
    let cx_map: Vec<_> = (0..rows.len()).map(cx).collect();

    if let Some(vault_dir) = vault_root.as_ref() {
        write_fsv_vault(vault_dir, &rows, &cx_map);
    }

    let centroids = build_centroids(&rows, 316, 547);
    centroids.save(&root).expect("save FSV centroids");
    postings_for_assignments(&root, &centroids, rows.len());
    std::fs::write(root.join("cx_map.csv"), fsv_cx_map(&centroids, &cx_map)).expect("write cx map");
    let search = SpannSearch::new(SlotId::new(0), centroids, &root);
    let hits = search.search(&rows[547].1, 10, 16).expect("FSV search");
    let report = hits
        .iter()
        .map(|(id, score)| {
            let cx_id = cx_map[*id as usize];
            format!("{id},{score:.6},{cx_id},{}", hex(&base_key(cx_id)))
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(root.join("search_hits.csv"), report).expect("write search hits");
    assert_eq!(hits.len(), 10);
}

#[test]
#[ignore = "server-only FSV trigger writes SPANN edge artifacts"]
fn fsv_issue547_edges_write_before_after_artifacts() {
    let root = std::env::var("CALYX_SPANN_EDGE_DIR")
        .map(PathBuf::from)
        .expect("set CALYX_SPANN_EDGE_DIR");
    assert_eq!(
        root.file_name().and_then(|name| name.to_str()),
        Some("edges"),
        "edge FSV root must be a dedicated directory named edges"
    );
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create edge root");

    let missing = root.join("missing_posting_list");
    std::fs::create_dir_all(&missing).expect("create missing edge");
    std::fs::write(root.join("missing-before.txt"), dir_listing(&missing)).expect("missing before");
    let missing_read = PostingListReader::new(&missing)
        .read_list(7)
        .expect("missing posting list is empty");
    std::fs::write(root.join("missing-after.txt"), dir_listing(&missing)).expect("missing after");
    std::fs::write(
        root.join("missing-result.txt"),
        format!("centroid_id=7 entries={}\n", missing_read.len()),
    )
    .expect("missing result");

    let corrupt = root.join("corrupt_zstd");
    std::fs::create_dir_all(&corrupt).expect("create corrupt edge");
    std::fs::write(corrupt.join("pl_0000.spb"), b"not zstd").expect("write corrupt block");
    std::fs::write(
        root.join("corrupt-before.txt"),
        file_state(&corrupt.join("pl_0000.spb")),
    )
    .expect("corrupt before");
    let corrupt_err = PostingListReader::new(&corrupt)
        .read_list(0)
        .expect_err("corrupt block must fail closed");
    std::fs::write(
        root.join("corrupt-after.txt"),
        file_state(&corrupt.join("pl_0000.spb")),
    )
    .expect("corrupt after");
    std::fs::write(root.join("corrupt-result.txt"), corrupt_err.code).expect("corrupt result");

    let magic = root.join("bad_centroid_magic");
    let rows = vectors(16, 4, 4);
    let index = build_centroids(&rows, 4, 4);
    index.save(&magic).expect("save edge centroids");
    let magic_path = magic.join("centroids.spn");
    std::fs::write(root.join("magic-before.txt"), first_bytes(&magic_path)).expect("magic before");
    let mut bytes = std::fs::read(&magic_path).expect("read magic edge");
    bytes[0] ^= 0xff;
    std::fs::write(&magic_path, bytes).expect("flip magic edge");
    let magic_err = SpannCentroidIndex::open(&magic).expect_err("bad magic must fail closed");
    std::fs::write(root.join("magic-after.txt"), first_bytes(&magic_path)).expect("magic after");
    std::fs::write(root.join("magic-result.txt"), magic_err.code).expect("magic result");

    let clamp = root.join("probe_clamp");
    std::fs::create_dir_all(&clamp).expect("create clamp edge");
    let rows = vectors(64, 8, 31);
    let centroids = build_centroids(&rows, 8, 31);
    postings_for_assignments(&clamp, &centroids, rows.len());
    std::fs::write(root.join("clamp-before.txt"), dir_listing(&clamp)).expect("clamp before");
    let search = SpannSearch::new(SlotId::new(0), centroids, &clamp);
    let hits = search.search(&rows[0].1, 10, 99).expect("probe clamp");
    std::fs::write(root.join("clamp-after.txt"), dir_listing(&clamp)).expect("clamp after");
    std::fs::write(
        root.join("clamp-result.txt"),
        format!("requested_n_probe=99 returned_hits={}\n", hits.len()),
    )
    .expect("clamp result");
}

fn fsv_roots() -> (PathBuf, Option<PathBuf>) {
    if let Ok(vault) = std::env::var("CALYX_SPANN_FSV_VAULT") {
        let vault = PathBuf::from(vault);
        return (vault.join("idx").join("slot_00.sparse"), Some(vault));
    }
    let root = std::env::var("CALYX_SPANN_FSV_DIR")
        .map(PathBuf::from)
        .expect("set CALYX_SPANN_FSV_DIR or CALYX_SPANN_FSV_VAULT");
    (root, None)
}

fn write_fsv_vault(vault_dir: &PathBuf, rows: &[(u32, Vec<f32>)], cx_map: &[CxId]) {
    std::fs::create_dir_all(vault_dir).expect("create FSV vault dir");
    let vault = AsterVault::open(
        vault_dir,
        fsv_vault_id(),
        b"issue547-spann-fsv".to_vec(),
        VaultOptions::default(),
    )
    .expect("open FSV vault");
    for chunk in rows.chunks(1000) {
        let batch = chunk
            .iter()
            .map(|(id, vector)| fsv_constellation(*id, vector, cx_map[*id as usize]))
            .collect::<Vec<_>>();
        vault.put_batch(batch).expect("write FSV batch");
    }
    vault.flush().expect("flush FSV vault");
}

fn fsv_constellation(local_id: u32, vector: &[f32], cx_id: CxId) -> Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(SlotId::new(0), sparse_from_dense(vector));
    let input = format!("synthetic://issue547-spann/{local_id}");
    let mut metadata = BTreeMap::new();
    metadata.insert("fsv_issue".to_string(), "547".to_string());
    metadata.insert("local_id".to_string(), local_id.to_string());
    Constellation {
        cx_id,
        vault_id: fsv_vault_id(),
        panel_version: 547,
        created_at: 1_786_000_000 + u64::from(local_id),
        input_ref: InputRef {
            hash: *blake3::hash(input.as_bytes()).as_bytes(),
            pointer: Some(input),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata,
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

fn sparse_from_dense(vector: &[f32]) -> SlotVector {
    SlotVector::Sparse {
        dim: vector.len() as u32,
        entries: vector
            .iter()
            .enumerate()
            .map(|(idx, val)| SparseEntry {
                idx: idx as u32,
                val: *val,
            })
            .collect(),
    }
}

fn fsv_cx_map(centroids: &SpannCentroidIndex, cx_map: &[CxId]) -> String {
    let mut rows = vec!["local_id,cx_id,base_key_hex,centroid_id".to_string()];
    rows.extend(
        centroids
            .assignments()
            .iter()
            .map(|(local_id, centroid_id)| {
                let cx_id = cx_map[*local_id as usize];
                format!(
                    "{local_id},{cx_id},{},{}",
                    hex(&base_key(cx_id)),
                    centroid_id
                )
            }),
    );
    rows.join("\n")
}

fn fsv_vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse()
        .expect("valid vault id")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn dir_listing(dir: &Path) -> String {
    let mut rows = std::fs::read_dir(dir)
        .expect("read edge dir")
        .map(|entry| {
            let entry = entry.expect("read edge entry");
            let size = entry.metadata().expect("edge metadata").len();
            format!("{} {size} bytes", entry.file_name().to_string_lossy())
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows.join("\n")
}

fn file_state(path: &Path) -> String {
    let bytes = std::fs::read(path).expect("read edge file");
    format!("size={} blake3={}\n", bytes.len(), blake3::hash(&bytes))
}

fn first_bytes(path: &Path) -> String {
    let bytes = std::fs::read(path).expect("read edge bytes");
    hex(&bytes[..16.min(bytes.len())])
}

fn sparse(entries: &[(u32, f32)], dim: u32) -> SlotVector {
    SlotVector::Sparse {
        dim,
        entries: entries
            .iter()
            .map(|(idx, val)| SparseEntry {
                idx: *idx,
                val: *val,
            })
            .collect(),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    #[test]
    fn n_probe_search_returns_distinct_top_k(n_probe in 1_usize..=8) {
        let dir = scratch(&format!("prop-{n_probe}"));
        let rows = vectors(64, 8, 31);
        let centroids = build_centroids(&rows, 8, 31);
        let all: Vec<_> = (0..64).map(|id| (id, 100.0 - id as f32)).collect();
        let writer = PostingListWriter::new(&dir);
        for centroid_id in 0..centroids.centroid_count() as u32 {
            writer.write_list(centroid_id, &all).expect("write list");
        }
        let search = SpannSearch::new(SlotId::new(0), centroids, &dir);

        let hits = search.search(&rows[5].1, 10, n_probe).expect("search");
        let distinct: BTreeSet<_> = hits.iter().map(|(id, _)| *id).collect();

        prop_assert_eq!(hits.len(), 10);
        prop_assert_eq!(distinct.len(), hits.len());
    }
}
