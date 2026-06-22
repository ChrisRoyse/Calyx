//! PH68 T03 - SPANN centroids in RAM and posting lists on disk.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use calyx_aster::cf::base_key;
use calyx_core::{SlotId, SlotVector, SparseEntry};
use calyx_sextant::index::spann::centroids::SpannCentroidIndex;
use calyx_sextant::index::spann::posting::encode_posting_block;
use calyx_sextant::index::{
    PostingListReader, PostingListWriter, PostingMember, SPANN_CENTROID_MAGIC, SextantIndex,
    SpannSearch, build_centroids,
};
use proptest::prelude::*;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

#[path = "spann/support.rs"]
mod support;

use support::{
    cx_usize_be as cx, dir_listing, file_state, first_bytes, fsv_cx_map, fsv_roots, hex, sparse,
    write_fsv_vault,
};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("calyx-spann-t03")
        .join(format!("{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
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

/// Dense row -> sparse (idx,val) pairs for posting storage (#701: members carry
/// their vectors so search ranks by true query distance).
fn sparse_of(row: &[f32]) -> Vec<(u32, f32)> {
    row.iter()
        .enumerate()
        .map(|(i, v)| (i as u32, *v))
        .collect()
}

fn postings_for_assignments(
    dir: &PathBuf,
    centroids: &SpannCentroidIndex,
    rows: &[(u32, Vec<f32>)],
) -> PostingListWriter {
    let writer = PostingListWriter::new(dir);
    let mut grouped = BTreeMap::<u32, Vec<PostingMember>>::new();
    for &(cx_id, centroid_id) in centroids.assignments() {
        let vector = sparse_of(&rows[cx_id as usize].1);
        grouped
            .entry(centroid_id)
            .or_default()
            .push(PostingMember::new(cx_id, vector));
    }
    for (centroid_id, mut entries) in grouped {
        entries.sort_by_key(|m| m.cx_id);
        writer
            .write_list(centroid_id, &entries)
            .expect("write list");
    }
    writer
}

/// Exact L2 top-k over the whole row set — the ground truth SPANN recall is scored
/// against. Returns cx ids ordered nearest-first (ties broken by id).
fn brute_force_topk(rows: &[(u32, Vec<f32>)], query: &[f32], k: usize) -> Vec<u32> {
    let mut scored: Vec<(u32, f32)> = rows
        .iter()
        .map(|(id, v)| {
            let d: f32 = v.iter().zip(query).map(|(a, b)| (a - b) * (a - b)).sum();
            (*id, d)
        })
        .collect();
    scored.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    scored.into_iter().take(k).map(|(id, _)| id).collect()
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
    let entries: Vec<PostingMember> = (0..200)
        .map(|_| {
            next += rng.gen_range(1..5);
            let nnz = rng.gen_range(1..6);
            let vector: Vec<(u32, f32)> = (0..nnz)
                .map(|j| (j as u32, rng.gen_range(-1.0_f32..1.0)))
                .collect();
            PostingMember::new(next, vector)
        })
        .collect();

    writer.write_list(7, &entries).expect("write postings");
    let read = PostingListReader::new(&dir)
        .read_list(7)
        .expect("read postings");

    assert_eq!(read.len(), 200);
    assert!(read.windows(2).all(|pair| pair[0].cx_id < pair[1].cx_id));
    for (expected, actual) in entries.iter().zip(read) {
        assert_eq!(actual.cx_id, expected.cx_id);
        assert_eq!(actual.vector.len(), expected.vector.len());
        for ((ei, ev), (ai, av)) in expected.vector.iter().zip(&actual.vector) {
            assert_eq!(ai, ei);
            assert!((av - ev).abs() <= 1.0e-6);
        }
    }
}

#[test]
fn zstd_block_is_smaller_than_raw_for_repetitive_postings() {
    let dir = scratch("posting-zstd");
    let writer = PostingListWriter::new(&dir);
    let entries: Vec<PostingMember> = (0..1000)
        .map(|id| PostingMember::new(id, vec![(0, 1.0_f32)]))
        .collect();
    let raw = encode_posting_block(&entries).expect("raw block");

    writer.write_list(0, &entries).expect("write compressed");
    let compressed = std::fs::metadata(dir.join("pl_0000.spb"))
        .expect("stat compressed")
        .len() as usize;

    assert!(compressed < raw.len(), "{compressed} >= {}", raw.len());
}

#[test]
fn spann_full_probe_matches_exact_nearest_neighbors() {
    // With every centroid probed, SPANN sees all members and — ranking by TRUE
    // query distance (#701) — must return the exact L2 top-k. This is impossible
    // to pass with the old static-scalar ranking, which ignored the query.
    let rows = vectors(2000, 32, 99);
    let n_centroids = 44;
    let centroids = build_centroids(&rows, n_centroids, 99);
    let dir = scratch("search-e2e");
    postings_for_assignments(&dir, &centroids, &rows);
    let search = SpannSearch::new(SlotId::new(0), centroids, &dir);

    for &qi in &[0_usize, 31, 500, 1234, 1999] {
        let query = &rows[qi].1;
        let hits = search.search(query, 10, n_centroids).expect("search");
        let got: Vec<u32> = hits.iter().map(|(id, _)| *id).collect();
        let truth = brute_force_topk(&rows, query, 10);

        assert_eq!(hits.len(), 10);
        assert_eq!(got, truth, "query {qi}: SPANN full-probe != exact NN");
        // A row queried against itself must rank first (its L2 distance is 0).
        assert_eq!(hits[0].0, qi as u32, "self should be nearest");
        // Scores are similarities (higher = closer) -> strictly non-increasing.
        assert!(hits.windows(2).all(|pair| pair[0].1 >= pair[1].1));
    }
}

#[test]
fn spann_limited_probe_has_high_true_recall() {
    // The real operating point probes only a few centroids. True recall@10 vs
    // brute force must stay high — the metric the FSV gate actually requires.
    // 3000 rows, 30 centroids (~100/region); probe 10 (1/3 of regions) — the same
    // ~1/3 probe budget the partitioned 1e6 FSV uses. Single-assignment SPANN at
    // this budget clears 0.85 true recall@10 (boundary duplication, #714, lifts it
    // further at lower probe budgets).
    let rows = vectors(3000, 32, 7);
    let centroids = build_centroids(&rows, 30, 7);
    let dir = scratch("search-recall");
    postings_for_assignments(&dir, &centroids, &rows);
    let search = SpannSearch::new(SlotId::new(0), centroids, &dir);

    let (mut found, mut total) = (0_usize, 0_usize);
    for qi in (0..3000).step_by(50) {
        let query = &rows[qi].1;
        let hits = search.search(query, 10, 10).expect("search");
        let got: BTreeSet<u32> = hits.iter().map(|(id, _)| *id).collect();
        let truth = brute_force_topk(&rows, query, 10);
        found += truth.iter().filter(|id| got.contains(id)).count();
        total += truth.len();
    }
    let recall = found as f32 / total as f32;
    assert!(recall >= 0.85, "true recall@10 {recall} < 0.85");
}

#[test]
fn spann_boundary_duplication_writes_member_to_adjacent_postings() {
    let dir = scratch("boundary-duplication");
    let centroids =
        SpannCentroidIndex::from_parts(1, vec![vec![0.0], vec![10.0]], Vec::new(), Vec::new())
            .expect("centroids");
    let mut search =
        SpannSearch::new(SlotId::new(0), centroids, &dir).with_boundary_duplication(3.0, 2);
    let vector = sparse_vector(1, &[(0, 5.0)]);

    search
        .insert(cx(42), vector.clone(), 1)
        .expect("insert boundary vector");

    let first = PostingListReader::new(&dir).read_list(0).expect("first");
    let second = PostingListReader::new(&dir).read_list(1).expect("second");
    assert_eq!(first.iter().map(|m| m.cx_id).collect::<Vec<_>>(), vec![0]);
    assert_eq!(second.iter().map(|m| m.cx_id).collect::<Vec<_>>(), vec![0]);
    assert_eq!(first[0].vector, sparse_of(&[5.0]));
    assert_eq!(second[0].vector, sparse_of(&[5.0]));

    let hits = SextantIndex::search(&search, &vector, 1, Some(1)).expect("adapter search");
    assert_eq!(hits[0].cx_id, cx(42));
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
    let all: Vec<PostingMember> = rows
        .iter()
        .map(|(id, v)| PostingMember::new(*id, sparse_of(v)))
        .collect();
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
    // n_probe clamped past the centroid count still returns the exact NN: every
    // list is identical so all members are seen, and rank 0 is the self row.
    assert_eq!(hits[0].0, 0, "self row is nearest");
}

fn sparse_vector(dim: u32, entries: &[(u32, f32)]) -> SlotVector {
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
    postings_for_assignments(&root, &centroids, &rows);
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
    postings_for_assignments(&clamp, &centroids, &rows);
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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    #[test]
    fn n_probe_search_returns_distinct_top_k(n_probe in 1_usize..=8) {
        let dir = scratch(&format!("prop-{n_probe}"));
        let rows = vectors(64, 8, 31);
        let centroids = build_centroids(&rows, 8, 31);
        let all: Vec<PostingMember> = rows
            .iter()
            .map(|(id, v)| PostingMember::new(*id, sparse_of(v)))
            .collect();
        let writer = PostingListWriter::new(&dir);
        for centroid_id in 0..centroids.centroid_count() as u32 {
            writer.write_list(centroid_id, &all).expect("write list");
        }
        let search = SpannSearch::new(SlotId::new(0), centroids, &dir);

        let hits = search.search(&rows[5].1, 10, n_probe).expect("search");
        let distinct: BTreeSet<_> = hits.iter().map(|(id, _)| *id).collect();

        prop_assert_eq!(hits.len(), 10);
        prop_assert_eq!(distinct.len(), hits.len());
        // Every list is identical and holds all rows, so any probe count sees the
        // full set and the self row (query = rows[5]) must rank first.
        prop_assert_eq!(hits[0].0, 5);
    }
}
