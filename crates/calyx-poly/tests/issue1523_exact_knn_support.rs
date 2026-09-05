//! Shared support for the dedicated issue #1523 FSV binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, VaultId};
use calyx_poly::exact_knn::{
    ExactKnnConfig, exact_cosine_synthetic_probe, exact_cosine_top_k_with_config,
};
use calyx_poly::knn_base_rate::{ResolvedExemplar, knn_base_rate_with_execution};
use calyx_poly::knn_graph_edges::{
    compute_knn_edges_with_execution, persist_knn_edges_on_ingest_with_execution,
};
use calyx_poly::{
    PARAMETER_ADAPTATION_MIN_ROWS, ParameterAdaptationArtifactRef, ParameterAdaptationRequest,
    ParameterAdaptationSchedule, ParameterAdaptationStatus, ParameterObservation,
    ParameterSetSnapshot, compute_parameter_adaptation_report_with_execution,
};
use serde_json::{Value, json};

pub fn fsv_root() -> PathBuf {
    calyx_fsv::fsv_root_or_target(
        "POLY_ISSUE1523_FSV_ROOT",
        "issue1523-cuda-exact-knn",
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/fsv/issue1523"),
    )
}

pub fn exact_batch_parity() -> Value {
    let rows = [
        vec![1.0, 0.0],
        vec![1.0, 0.0],
        vec![1.0, 0.0],
        vec![-1.0, 0.0],
    ];
    let refs = rows.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let queries = [&rows[0][..], &rows[1][..], &rows[2][..]];
    let result = exact_cosine_top_k_with_config(
        &refs,
        &queries,
        Some(&[0, 1, 2]),
        ExactKnnConfig {
            k: 2,
            query_batch_rows: 2,
            corpus_chunk_rows: 2,
        },
    )
    .expect("batched exact tie ranking");
    assert_eq!(result.rankings, vec![vec![1, 2], vec![0, 2], vec![0, 1]]);
    assert_execution(&result.execution, 0);
    if cfg!(feature = "cuda") {
        assert_eq!(result.execution.cuda_reports.len(), 2);
        assert!(result.execution.cuda_reports.iter().all(|row| {
            row.chunks == 2 && row.intermediate_readback_pairs == 0 && !row.host_merge
        }));
    } else {
        assert_eq!(result.execution.exhaustive_cpu_similarity_evaluations, 12);
    }
    json!({"rankings": result.rankings, "execution": result.execution})
}

pub fn neighbor_path_parity(root: &Path) -> Value {
    let ingested = exemplar(10, &[1.0, 0.0], true);
    let corpus = vec![
        exemplar(1, &[1.0, 0.0], true),
        exemplar(2, &[0.8, 0.2], true),
        exemplar(3, &[-1.0, 0.0], false),
    ];
    let vault = AsterVault::open(
        root.join("vault"),
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse::<VaultId>().unwrap(),
        b"poly-issue1523-knn".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let (run, execution) = persist_knn_edges_on_ingest_with_execution(
        &vault,
        "poly_issue1523",
        "crypto",
        &ingested,
        &corpus,
        2,
    )
    .expect("GPU-backed graph persistence");
    let hashes = run
        .readback_edges
        .iter()
        .map(|row| row.value_blake3.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        hashes,
        [
            "01985196ab50cffc6c5ace418ef0352a0a4ed949917a5a1c20cf944e89ab4afe",
            "bee8422afa8dd2b449eddb1616ff21130cb06f6fa413aa27fd0d1c46e068e535",
        ]
    );
    assert_execution(&execution, execution.shortlist_cpu_similarity_evaluations);

    let tied = vec![
        exemplar(3, &[1.0, 0.0], false),
        exemplar(1, &[1.0, 0.0], true),
        exemplar(2, &[1.0, 0.0], false),
    ];
    let (tie_edges, tie_execution) =
        compute_knn_edges_with_execution("crypto", &ingested, &tied, 2).unwrap();
    assert_eq!(
        tie_edges.iter().map(|row| row.dst).collect::<Vec<_>>(),
        vec![cx(1), cx(2)]
    );
    let (base, base_execution) = knn_base_rate_with_execution(&tied, &[1.0, 0.0], 2).unwrap();
    assert_eq!(
        base.neighbors
            .iter()
            .map(|row| row.cx_id.as_str())
            .collect::<Vec<_>>(),
        vec![cx(1).to_string(), cx(2).to_string()]
    );
    assert_eq!(base.p_yes, 0.5);
    assert_eq!(base.mean_similarity, 1.0);
    assert_eq!(base.reliability, 0.1);
    json!({
        "graph_edge_hashes": hashes,
        "graph_execution": execution,
        "tie_dst": tie_edges.iter().map(|row| row.dst).collect::<Vec<_>>(),
        "tie_execution": tie_execution,
        "base_rate": base,
        "base_execution": base_execution,
    })
}

pub fn adaptation_parity(root: &Path) -> Value {
    let observations = golden_observations();
    let request = adaptation_request(root, "parity", observations, vec![1, 3, 5], 5);
    let (report, execution) =
        compute_parameter_adaptation_report_with_execution(&request, 0).unwrap();
    assert_eq!(report.status, ParameterAdaptationStatus::Promoted);
    assert_eq!(report.proposed.knn_k, 1);
    assert_eq!(report.proposed.te_lag, 2);
    assert_eq!(report.metrics.current_knn_brier, 0.16000000000000003);
    assert_eq!(report.metrics.selected_knn_brier, 0.0);
    assert_eq!(execution.query_count, 8);
    assert_eq!(execution.query_batches, 1);
    assert_eq!(execution.output_k, 7);
    assert_eq!(execution.shortlist_cpu_similarity_evaluations, 56);
    assert_execution(&execution, 56);
    if cfg!(feature = "cuda") {
        assert_eq!(
            execution.cuda_reports.len(),
            1,
            "one ranking must serve all k values"
        );
    } else {
        assert_eq!(execution.exhaustive_cpu_similarity_evaluations, 64);
    }
    json!({"proposed": report.proposed, "metrics": report.metrics, "status": report.status, "execution": execution})
}

pub fn larger_than_vram_probe() -> Value {
    if !cfg!(feature = "cuda") {
        return json!({"executed": false, "reason": "non_cuda_reference_build"});
    }
    let Some(rows) = env_u64("POLY_ISSUE1523_SYNTHETIC_ROWS") else {
        return json!({"executed": false, "reason": "explicit_large_probe_not_requested"});
    };
    let dim = env_usize("POLY_ISSUE1523_SYNTHETIC_DIM").unwrap_or(128);
    let chunk_rows = env_usize("POLY_ISSUE1523_SYNTHETIC_CHUNK_ROWS").unwrap_or(1_000_000);
    let total_vram = env_u64("POLY_ISSUE1523_GPU_TOTAL_BYTES")
        .expect("large probe requires preflight physical GPU byte count");
    let logical_bytes = rows.saturating_mul(dim as u64).saturating_mul(4);
    assert!(
        logical_bytes > total_vram,
        "synthetic corpus must exceed physical VRAM"
    );
    let query = vec![1.0f32 / (dim as f32).sqrt(); dim];
    let started = Instant::now();
    let report =
        exact_cosine_synthetic_probe(0x1523, rows, dim, &query, 1, 10, chunk_rows).unwrap();
    assert!(report.chunks > 1);
    assert_eq!(report.corpus_uploads, 0);
    assert_eq!(report.intermediate_readback_pairs, 0);
    assert_eq!(report.final_readback_pairs, 10);
    assert!(!report.host_merge);
    assert_eq!(report.device_generated_values, rows * dim as u64);
    json!({"executed": true, "logical_corpus_bytes": logical_bytes, "gpu_total_bytes": total_vram, "wall_ms": started.elapsed().as_millis(), "execution": report})
}

fn adaptation_request(
    root: &Path,
    label: &str,
    observations: Vec<ParameterObservation>,
    candidate_knn_k: Vec<usize>,
    current_k: usize,
) -> ParameterAdaptationRequest {
    let dir = root.join(label);
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

fn golden_observations() -> Vec<ParameterObservation> {
    let lag_signal = [0.05, 0.10, 0.95, 0.90, 0.15, 0.20, 0.85, 0.80];
    let outcomes = [false, false, false, false, true, true, false, false];
    (0..8)
        .map(|idx| ParameterObservation {
            ts: 1_000 + idx as u64,
            scalar_value: 0.20 + idx as f64 * 0.05,
            heavy_tail_value: 2f64.powi(idx as i32),
            lag_signal: lag_signal[idx],
            outcome_yes: outcomes[idx],
            knn_vector: if outcomes[idx] {
                vec![1.0, 0.05 * idx as f32]
            } else {
                vec![-1.0, 0.05 * idx as f32]
            },
        })
        .collect()
}

fn exemplar(id: u8, vector: &[f32], outcome_yes: bool) -> ResolvedExemplar {
    ResolvedExemplar {
        cx_id: cx(id),
        vector: vector.to_vec(),
        outcome_yes,
    }
}

fn cx(id: u8) -> CxId {
    CxId::from_bytes([id; 16])
}

fn assert_execution(execution: &calyx_poly::exact_knn::ExactKnnExecution, shortlist: u64) {
    assert_eq!(execution.shortlist_cpu_similarity_evaluations, shortlist);
    if cfg!(feature = "cuda") {
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
    } else {
        assert_eq!(execution.backend, "cpu-reference-non-cuda-build");
        assert!(!execution.cuda_compiled);
        assert!(execution.cuda_reports.is_empty());
    }
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name).ok()?.parse().ok()
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.parse().ok()
}
