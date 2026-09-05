//! Benchmark-only support for the dedicated issue #1523 FSV binary.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::Instant;

use calyx_core::CxId;
use calyx_poly::knn_base_rate::ResolvedExemplar;
use calyx_poly::knn_graph_edges::{
    EDGE_KNN_RESOLVED, KnnGraphEdge, compute_knn_edges_batch_with_execution,
};
use calyx_poly::{
    PARAMETER_ADAPTATION_MIN_ROWS, ParameterAdaptationArtifactRef, ParameterAdaptationRequest,
    ParameterAdaptationSchedule, ParameterObservation, ParameterSetSnapshot,
    compute_parameter_adaptation_report_with_execution,
};
use serde_json::{Value, json};

pub fn speed_proof(root: &Path) -> Value {
    if !cfg!(feature = "cuda")
        || std::env::var("POLY_ISSUE1523_RUN_BENCH").ok().as_deref() != Some("1")
    {
        return json!({"executed": false, "reason": "explicit_cuda_benchmark_not_requested"});
    }
    let ingest = ingest_speed();
    let adaptation = adaptation_speed(root);
    let readback = json!({"executed": true, "ingest": ingest, "adaptation": adaptation});
    fs::write(
        root.join("speed_readback.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
    assert!(
        readback["ingest"]["speedup"].as_f64().unwrap() > 1.0,
        "ingest speed proof did not clear 1x: {}",
        readback["ingest"]
    );
    assert!(
        readback["adaptation"]["speedup"].as_f64().unwrap() > 1.0,
        "adaptation speed proof did not clear 1x: {}",
        readback["adaptation"]
    );
    readback
}

fn ingest_speed() -> Value {
    let rows = env_usize("POLY_ISSUE1523_INGEST_ROWS").unwrap_or(100_000);
    let query_count = env_usize("POLY_ISSUE1523_INGEST_QUERIES").unwrap_or(128);
    let dim = env_usize("POLY_ISSUE1523_INGEST_DIM").unwrap_or(64);
    let k = 10;
    let corpus = (0..rows)
        .map(|idx| generated_exemplar(idx as u64, dim, false))
        .collect::<Vec<_>>();
    let queries = (0..query_count)
        .map(|idx| generated_exemplar((rows + idx) as u64, dim, true))
        .collect::<Vec<_>>();
    let cpu_started = Instant::now();
    let expected = legacy_graph_batch("bench", &queries, &corpus, k);
    let cpu = cpu_started.elapsed();
    let gpu_started = Instant::now();
    let (actual, execution) =
        compute_knn_edges_batch_with_execution("bench", &queries, &corpus, k).unwrap();
    let gpu = gpu_started.elapsed();
    assert_eq!(actual, expected);
    assert_execution(&execution, (query_count * (k + 32)) as u64);
    json!({"corpus_rows": rows, "queries": query_count, "dim": dim, "cpu_ms": cpu.as_secs_f64()*1000.0, "gpu_ms": gpu.as_secs_f64()*1000.0, "speedup": cpu.as_secs_f64()/gpu.as_secs_f64(), "execution": execution})
}

fn adaptation_speed(root: &Path) -> Value {
    let rows = env_usize("POLY_ISSUE1523_ADAPT_ROWS").unwrap_or(1_500);
    let dim = env_usize("POLY_ISSUE1523_ADAPT_DIM").unwrap_or(32);
    let candidates = vec![5, 15, 31];
    let observations = generated_observations(rows, dim);
    let request = adaptation_request(root, observations.clone(), candidates.clone(), 31);
    let cpu_started = Instant::now();
    let candidate_briers = candidates
        .iter()
        .map(|k| (*k, legacy_brier(&observations, *k)))
        .collect::<Vec<_>>();
    let selected = candidate_briers
        .iter()
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
        .unwrap()
        .0;
    let current_brier = legacy_brier(&observations, 31);
    let selected_brier = legacy_brier(&observations, selected);
    let cpu = cpu_started.elapsed();
    let gpu_started = Instant::now();
    let (report, execution) =
        compute_parameter_adaptation_report_with_execution(&request, 0).unwrap();
    let gpu = gpu_started.elapsed();
    assert_eq!(report.proposed.knn_k, selected);
    assert!((report.metrics.current_knn_brier - current_brier).abs() < 1e-12);
    assert!((report.metrics.selected_knn_brier - selected_brier).abs() < 1e-12);
    assert_execution(&execution, (rows * (31 + 32)) as u64);
    json!({"rows": rows, "dim": dim, "candidate_k": candidates, "cpu_ms": cpu.as_secs_f64()*1000.0, "gpu_ms": gpu.as_secs_f64()*1000.0, "speedup": cpu.as_secs_f64()/gpu.as_secs_f64(), "execution": execution})
}

fn legacy_graph_batch(
    domain: &str,
    queries: &[ResolvedExemplar],
    corpus: &[ResolvedExemplar],
    k: usize,
) -> Vec<Vec<KnnGraphEdge>> {
    assert!(!domain.trim().is_empty());
    assert!(!corpus.is_empty());
    assert!(k > 0 && k <= corpus.len());
    queries
        .iter()
        .map(|query| legacy_graph_query(domain, query, corpus, k))
        .collect()
}

fn legacy_graph_query(
    domain: &str,
    query: &ResolvedExemplar,
    corpus: &[ResolvedExemplar],
    k: usize,
) -> Vec<KnnGraphEdge> {
    let dim = query.vector.len();
    assert!(dim > 0);
    assert!(query.vector.iter().all(|value| value.is_finite()));
    let mut seen = BTreeSet::from([query.cx_id]);
    for row in corpus {
        assert!(seen.insert(row.cx_id));
        assert_eq!(row.vector.len(), dim);
        assert!(row.vector.iter().all(|value| value.is_finite()));
    }
    let corpus_by_cx: BTreeMap<CxId, &ResolvedExemplar> =
        corpus.iter().map(|row| (row.cx_id, row)).collect();
    let mut scored = corpus
        .iter()
        .map(|row| (row.cx_id, cosine_f32(&query.vector, &row.vector)))
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.to_string().cmp(&right.0.to_string()))
    });
    scored
        .into_iter()
        .take(k)
        .enumerate()
        .map(|(idx, (dst, raw_similarity))| {
            let neighbor = corpus_by_cx.get(&dst).expect("legacy hit came from corpus");
            let similarity = canonical_score(f64::from(raw_similarity));
            KnnGraphEdge {
                src: query.cx_id,
                dst,
                edge_type: EDGE_KNN_RESOLVED.to_string(),
                rank: idx + 1,
                similarity,
                weight: similarity.clamp(0.0, 1.0),
                domain: domain.to_string(),
                k,
                corpus_len: corpus.len(),
                query_outcome_yes: query.outcome_yes,
                neighbor_outcome_yes: neighbor.outcome_yes,
            }
        })
        .collect()
}

fn legacy_brier(rows: &[ParameterObservation], k: usize) -> f64 {
    let total = (0..rows.len())
        .map(|query| {
            let mut scored = (0..rows.len())
                .filter(|other| *other != query)
                .map(|other| {
                    (
                        cosine_f64(&rows[query].knn_vector, &rows[other].knn_vector),
                        other,
                    )
                })
                .collect::<Vec<_>>();
            scored.sort_by(|left, right| right.0.partial_cmp(&left.0).unwrap_or(Ordering::Equal));
            let yes = scored
                .iter()
                .take(k)
                .filter(|row| rows[row.1].outcome_yes)
                .count();
            let p = yes as f64 / k as f64;
            let y = f64::from(rows[query].outcome_yes);
            (p - y) * (p - y)
        })
        .sum::<f64>();
    total / rows.len() as f64
}

fn adaptation_request(
    root: &Path,
    observations: Vec<ParameterObservation>,
    candidate_knn_k: Vec<usize>,
    current_k: usize,
) -> ParameterAdaptationRequest {
    let dir = root.join("speed");
    fs::create_dir_all(&dir).unwrap();
    let observations_path = dir.join("observations.json");
    fs::write(
        &observations_path,
        serde_json::to_vec_pretty(&observations).unwrap(),
    )
    .unwrap();
    let rollback_path = dir.join("rollback.json");
    fs::write(&rollback_path, br#"{"restore":"previous"}"#).unwrap();
    let scheduled_at_ts = observations.last().unwrap().ts + 20;
    ParameterAdaptationRequest {
        domain: "crypto".to_string(),
        horizon_bucket: "1h_24h".to_string(),
        observations_artifact: artifact(&observations_path),
        rollback_artifact: artifact(&rollback_path),
        ledger_dir: dir.join("ledger").display().to_string(),
        current: ParameterSetSnapshot {
            version: "crypto:1h_24h:previous".to_string(),
            encoder_sigma: 0.01,
            quantile_edges: vec![0.0, 1.0, 2.0],
            te_lag: 1,
            knn_k: current_k,
        },
        schedule: ParameterAdaptationSchedule {
            previous_run_ts: observations[3.min(observations.len() - 1)].ts,
            scheduled_at_ts,
            min_rows: PARAMETER_ADAPTATION_MIN_ROWS,
            min_new_rows: 1,
            max_te_lag: 3,
            candidate_knn_k,
            min_brier_improvement: 1.0e-9,
        },
        observations,
    }
}

fn artifact(path: &Path) -> ParameterAdaptationArtifactRef {
    ParameterAdaptationArtifactRef {
        path: path.display().to_string(),
        blake3: blake3::hash(&fs::read(path).unwrap()).to_hex().to_string(),
    }
}

fn generated_observations(rows: usize, dim: usize) -> Vec<ParameterObservation> {
    (0..rows)
        .map(|idx| {
            let vector = generated_vector(idx as u64, dim);
            ParameterObservation {
                ts: 10_000 + idx as u64,
                scalar_value: idx as f64 * 0.001 + (idx % 7) as f64,
                heavy_tail_value: (idx + 1) as f64,
                lag_signal: ((idx * 17 % 101) as f64) / 101.0,
                outcome_yes: vector[0] > 0.0,
                knn_vector: vector,
            }
        })
        .collect()
}

fn generated_exemplar(id: u64, dim: usize, query: bool) -> ResolvedExemplar {
    let stable = u128::from(id) + 1 + if query { 1u128 << 120 } else { 0 };
    let vector = generated_vector(id, dim);
    ResolvedExemplar {
        cx_id: CxId::from_bytes(stable.to_be_bytes()),
        outcome_yes: vector[0] > 0.0,
        vector,
    }
}

fn generated_vector(seed: u64, dim: usize) -> Vec<f32> {
    let mut state = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut values = Vec::with_capacity(dim);
    for col in 0..dim {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let random = ((state >> 40) as i32 - (1 << 23)) as f32 / (1 << 23) as f32;
        values.push(random + ((seed + col as u64) % 19) as f32 * 0.001);
    }
    values[0] += if seed.is_multiple_of(3) { 2.0 } else { -2.0 };
    values
}

fn cosine_f32(left: &[f32], right: &[f32]) -> f32 {
    let (mut dot, mut a, mut b) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in left.iter().zip(right) {
        dot += x * y;
        a += x * x;
        b += y * y;
    }
    if a == 0.0 || b == 0.0 {
        0.0
    } else {
        dot / (a.sqrt() * b.sqrt())
    }
}

fn cosine_f64(left: &[f32], right: &[f32]) -> f64 {
    let (mut dot, mut a, mut b) = (0.0f64, 0.0f64, 0.0f64);
    for (x, y) in left.iter().zip(right) {
        let (x, y) = (f64::from(*x), f64::from(*y));
        dot += x * y;
        a += x * x;
        b += y * y;
    }
    if a <= f64::EPSILON || b <= f64::EPSILON {
        0.0
    } else {
        dot / (a.sqrt() * b.sqrt())
    }
}

fn canonical_score(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn assert_execution(execution: &calyx_poly::exact_knn::ExactKnnExecution, shortlist: u64) {
    assert_eq!(execution.shortlist_cpu_similarity_evaluations, shortlist);
    assert_eq!(execution.backend, "cuvs-bruteforce-chunked");
    assert!(execution.cuda_compiled);
    assert_eq!(execution.exhaustive_cpu_similarity_evaluations, 0);
    assert!(!execution.cuda_reports.is_empty());
    assert!(execution.cuda_reports.iter().all(|row| {
        row.backend.starts_with("cuvs-")
            && row.intermediate_readback_pairs == 0
            && !row.host_merge
            && row.pinned_staging
    }));
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name).ok()?.parse().ok()
}
