use std::fs;
use std::time::Instant;

use calyx_core::{CxId, SlotId, SlotVector, content_address};
use calyx_sextant::{
    CALYX_SEXTANT_DIM_MISMATCH, CALYX_SEXTANT_EF_TOO_SMALL, CALYX_SEXTANT_INDEX_EMPTY, HnswIndex,
    SextantIndex,
};
use serde_json::json;

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
#[ignore = "aiwonder FSV writes PH23 HNSW recall source-of-truth artifacts"]
fn hnsw_recall_aiwonder_fsv() {
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

fn dense(data: Vec<f32>) -> SlotVector {
    SlotVector::Dense {
        dim: data.len() as u32,
        data,
    }
}

fn cx(value: usize) -> CxId {
    CxId::from_bytes((value as u128).to_be_bytes())
}

fn p99(values: &mut [u128]) -> u128 {
    values.sort_unstable();
    values[((values.len() as f32 * 0.99).ceil() as usize).saturating_sub(1)]
}

fn digest_hex(bytes: &[u8]) -> String {
    content_address([bytes])
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
