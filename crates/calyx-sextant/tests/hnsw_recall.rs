use std::fs;
use std::time::Instant;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use calyx_aster::gc::{AnnGcReclaimer, AnnIndexGraph, SharedAnnIndex};
use calyx_core::{CxId, SlotId};
use calyx_sextant::{
    CALYX_SEXTANT_DIM_MISMATCH, CALYX_SEXTANT_EF_TOO_SMALL, CALYX_SEXTANT_INDEX_EMPTY, HnswIndex,
    SextantIndex,
};
use serde_json::json;
use sextant_support::{cx_usize_be as cx, dense, digest_hex};

#[test]
fn hnsw_ef_search_recalls_bruteforce_neighbors() {
    let index = build_index(512, 8);
    let queries = query_vectors(512, 8, 32);
    let recall = mean_recall(&index, &queries, 10, 128);

    assert!(recall >= 0.8, "recall@10={recall}");
    assert!(index.neighbor_counts().into_iter().all(|count| count <= 32));
}

#[test]
fn hnsw_search_edges_fail_closed() {
    let empty = HnswIndex::new(SlotId::new(23), 4, 7);
    let empty_error = empty
        .search(&dense(vec![1.0, 0.0, 0.0, 0.0]), 1, Some(1))
        .unwrap_err();
    assert_eq!(empty_error.code, CALYX_SEXTANT_INDEX_EMPTY);

    let mut index = HnswIndex::new(SlotId::new(23), 4, 7);
    index
        .insert(cx(1), dense(vec![1.0, 0.0, 0.0, 0.0]), 1)
        .unwrap();
    index
        .insert(cx(2), dense(vec![0.0, 1.0, 0.0, 0.0]), 2)
        .unwrap();

    let k_zero = index
        .search(&dense(vec![1.0, 0.0, 0.0, 0.0]), 0, Some(1))
        .unwrap_err();
    assert_eq!(k_zero.code, CALYX_SEXTANT_EF_TOO_SMALL);

    let ef_small = index
        .search(&dense(vec![1.0, 0.0, 0.0, 0.0]), 2, Some(1))
        .unwrap_err();
    assert_eq!(ef_small.code, CALYX_SEXTANT_EF_TOO_SMALL);

    let dim = index
        .search(&dense(vec![1.0, 0.0, 0.0]), 1, Some(1))
        .unwrap_err();
    assert_eq!(dim.code, CALYX_SEXTANT_DIM_MISMATCH);

    let all_rows = index
        .search(&dense(vec![1.0, 0.0, 0.0, 0.0]), 5, None)
        .unwrap();
    assert_eq!(all_rows.len(), 2);
}

#[test]
fn hnsw_duplicate_insert_reconnects_updated_vector() {
    let mut index = build_index(128, 8);
    let moved = cx(0);
    let original = unit_vector(0, 8);
    let target = unit_vector(127, 8);
    index.insert(moved, dense(target.clone()), 999).unwrap();

    let got = index.search(&dense(target), 1, Some(32)).unwrap();
    let old = index.search(&dense(original), 1, Some(32)).unwrap();

    assert_eq!(got[0].cx_id, moved);
    assert_ne!(old[0].cx_id, moved);
    assert_eq!(index.stats().base_seq, 999);
    assert_eq!(index.stats().built_at_seq, 999);
}

#[test]
fn hnsw_byte_identical_query_returns_self_with_minimal_ef() {
    let index = build_index(256, 8);
    let target = unit_vector(199, 8);
    let got = index.search(&dense(target), 1, Some(1)).unwrap();

    assert_eq!(got[0].cx_id, cx(199));
}

#[test]
fn hnsw_tombstones_are_purged_by_shared_ann_gc() {
    let mut index = build_index(100, 8);
    for i in 0..30 {
        assert!(index.mark_deleted(cx(i), 1_000 + i as u64).unwrap());
    }
    assert_eq!(index.total_nodes(), 100);
    assert_eq!(index.live_len(), 70);
    assert_eq!(index.tombstone_count(), 30);
    assert!((index.tombstone_ratio() - 0.30).abs() < f64::EPSILON);
    assert_eq!(index.vector(cx(0)), None);
    let deleted_query = dense(unit_vector(0, 8));
    let got = index.search(&deleted_query, 10, Some(64)).unwrap();
    assert_eq!(got.len(), 10);
    assert!(!got.iter().any(|hit| hit.cx_id == cx(0)));

    let shared = SharedAnnIndex::new(index);
    let old_reader = shared.current().unwrap();
    let reclaimer = AnnGcReclaimer::with_limits(std::time::Duration::ZERO, 0.25, 0.80);
    let result = reclaimer.run_once_at(&shared, "slot_23", 0.10, 1);

    assert!(result.triggered);
    assert_eq!(result.total_nodes_before, 100);
    assert_eq!(result.tombstoned_nodes_before, 30);
    assert_eq!(old_reader.ann_tombstone_stats().tombstoned_nodes, 30);
    let after = shared.current().unwrap();
    assert_eq!(after.total_nodes(), 70);
    assert_eq!(after.live_len(), 70);
    assert_eq!(after.tombstone_count(), 0);
    assert_eq!(after.ann_tombstone_stats().tombstone_ratio(), 0.0);
}

#[test]
#[ignore = "manual FSV writes PH23 HNSW recall source-of-truth artifacts"]
fn hnsw_recall_manual_fsv() {
    let root = std::env::var("CALYX_FSV_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-hnsw-recall-fsv"));
    fs::create_dir_all(&root).unwrap();

    let n = 10_000;
    let dim = 8;
    let k = 10;
    let ef = 128;
    let index = build_index(n, dim);
    assert_eq!(index.stats().len, n);
    let queries = query_vectors(n, dim, 100);
    let mut latencies = Vec::with_capacity(queries.len());
    let mut recalls = Vec::with_capacity(queries.len());
    for query in &queries {
        let exact = index.brute_force(query, k);
        let start = Instant::now();
        let got = index.search(&dense(query.clone()), k, Some(ef)).unwrap();
        latencies.push(start.elapsed().as_micros());
        recalls.push(recall_at_k(&got, &exact, k));
    }
    let recall = recalls.iter().sum::<f32>() / recalls.len() as f32;
    let p99_us = p99(&mut latencies);

    let empty_error = HnswIndex::new(SlotId::new(23), dim as u32, 7)
        .search(&dense(queries[0].clone()), k, Some(ef))
        .unwrap_err()
        .code
        .to_string();
    let ef_error = index
        .search(&dense(queries[0].clone()), k, Some(k - 1))
        .unwrap_err()
        .code
        .to_string();
    let dim_error = index
        .search(&dense(vec![1.0, 0.0]), 1, Some(1))
        .unwrap_err()
        .code
        .to_string();
    let mut update_index = build_index(128, dim);
    let moved = cx(0);
    let update_target = unit_vector(127, dim);
    update_index
        .insert(moved, dense(update_target.clone()), 999)
        .unwrap();
    let update_hit = update_index
        .search(&dense(update_target), 1, Some(32))
        .unwrap();

    let report = json!({
        "n": n,
        "stored_rows": index.stats().len,
        "dim": dim,
        "queries": queries.len(),
        "k": k,
        "ef": ef,
        "recall_at_10": recall,
        "p99_us": p99_us,
        "max_neighbor_count": index.neighbor_counts().into_iter().max().unwrap_or(0),
        "layer_histogram": index.layer_histogram(),
        "edge_empty": empty_error,
        "edge_ef_too_small": ef_error,
        "edge_dim_mismatch": dim_error,
        "duplicate_update_top_id": update_hit[0].cx_id.to_string(),
        "duplicate_update_base_seq": update_index.stats().base_seq,
        "duplicate_update_built_at_seq": update_index.stats().built_at_seq,
    });
    let path = root.join("hnsw-recall-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let readback: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let digest = digest_hex(&bytes);

    println!("PH23_HNSW_FSV_ROOT={}", root.display());
    println!("PH23_HNSW_RECALL_REPORT={}", path.display());
    println!("PH23_HNSW_RECALL_REPORT_BLAKE3={digest}");
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert!(readback["recall_at_10"].as_f64().unwrap() >= 0.95);
    assert!(readback["p99_us"].as_u64().unwrap() < 5_000);
    assert_eq!(readback["stored_rows"], n);
    assert_eq!(readback["edge_empty"], CALYX_SEXTANT_INDEX_EMPTY);
    assert_eq!(readback["edge_ef_too_small"], CALYX_SEXTANT_EF_TOO_SMALL);
    assert_eq!(readback["edge_dim_mismatch"], CALYX_SEXTANT_DIM_MISMATCH);
    assert_eq!(readback["duplicate_update_top_id"], moved.to_string());
    assert_eq!(readback["duplicate_update_base_seq"], 999);
    assert_eq!(readback["duplicate_update_built_at_seq"], 999);
}

fn build_index(n: usize, dim: usize) -> HnswIndex {
    let mut index = HnswIndex::new(SlotId::new(23), dim as u32, 7);
    for i in 0..n {
        index
            .insert(cx(i), dense(unit_vector(i, dim)), i as u64 + 1)
            .unwrap();
    }
    index
}

fn query_vectors(n: usize, dim: usize, count: usize) -> Vec<Vec<f32>> {
    (0..count)
        .map(|idx| unit_vector((idx * 97 + 13) % n, dim))
        .collect()
}

fn unit_vector(i: usize, dim: usize) -> Vec<f32> {
    let t = i as f32 * 0.013;
    let mut data = vec![0.0_f32; dim];
    data[0] = t.cos();
    data[1] = t.sin();
    for (axis, value) in data.iter_mut().enumerate().skip(2) {
        *value = (((i + axis * 17) % 31) as f32 - 15.0) * 0.002;
    }
    normalize(data)
}

fn normalize(mut data: Vec<f32>) -> Vec<f32> {
    let norm = data.iter().map(|value| value * value).sum::<f32>().sqrt();
    for value in &mut data {
        *value /= norm;
    }
    data
}

fn mean_recall(index: &HnswIndex, queries: &[Vec<f32>], k: usize, ef: usize) -> f32 {
    queries
        .iter()
        .map(|query| {
            let exact = index.brute_force(query, k);
            let got = index.search(&dense(query.clone()), k, Some(ef)).unwrap();
            recall_at_k(&got, &exact, k)
        })
        .sum::<f32>()
        / queries.len() as f32
}

fn recall_at_k(got: &[calyx_sextant::IndexSearchHit], exact: &[(CxId, f32)], k: usize) -> f32 {
    let exact_ids: Vec<_> = exact.iter().take(k).map(|hit| hit.0).collect();
    got.iter()
        .take(k)
        .filter(|hit| exact_ids.contains(&hit.cx_id))
        .count() as f32
        / k as f32
}

fn p99(values: &mut [u128]) -> u128 {
    values.sort_unstable();
    values[((values.len() as f32 * 0.99).ceil() as usize).saturating_sub(1)]
}
