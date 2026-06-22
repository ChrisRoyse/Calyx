use std::fs;
use std::time::Instant;

mod issue640_support;

use calyx_sextant::FusionStrategy;
use issue640_support::*;
use serde_json::{Value, json};

#[test]
#[ignore = "manual release FSV writes issue #640 latency-series source-of-truth artifacts"]
fn embedded_scale_perf_manual_fsv() {
    let scale_cx = env_usize("CALYX_ISSUE640_SCALE_CX", DEFAULT_SCALE_CX);
    let dim = env_usize("CALYX_ISSUE640_DIM", DEFAULT_DIM);
    let queries = env_usize("CALYX_ISSUE640_QUERY_COUNT", DEFAULT_QUERY_COUNT);
    let ingest_samples = env_usize("CALYX_ISSUE640_INGEST_SAMPLES", DEFAULT_INGEST_SAMPLES);
    if cfg!(debug_assertions) && scale_cx >= DEFAULT_SCALE_CX {
        panic!("issue #640 scale perf FSV must run with cargo test --release");
    }

    let root = fsv_root();
    let vault_dir = root.join("aster-vault");
    fs::create_dir_all(&vault_dir).unwrap();

    let build_started = Instant::now();
    let (indexes, stage1) = build_indexes(scale_cx, dim);
    let build_ms = build_started.elapsed().as_millis();
    let engine = engine_from_indexes(indexes, stage1);
    let slots = slot_ids(LENS_COUNT);
    let pipeline_slots = pipeline_slots();
    let query_ids = query_ids(scale_cx, queries);

    let known = known_search_readback(&engine, dim, scale_cx);
    let single = measure_search(
        &engine,
        dim,
        &query_ids,
        &slots[..1],
        FusionStrategy::SingleLens { slot: slots[0] },
        false,
    );
    let rrf = measure_search(&engine, dim, &query_ids, &slots, FusionStrategy::Rrf, false);
    let pipeline = measure_search(
        &engine,
        dim,
        &query_ids,
        &pipeline_slots,
        FusionStrategy::Pipeline,
        false,
    );
    let explain = measure_search(&engine, dim, &query_ids, &slots, FusionStrategy::Rrf, true);
    let explain_overhead = explain.p99_us.saturating_sub(rrf.p99_us);

    let edge_errors = search_edge_errors(dim);
    let ingest = measure_ingest(&vault_dir, dim, ingest_samples);
    let vault_bytes = dir_bytes(&vault_dir);
    let vault_sst_files = count_ext(&vault_dir, "sst");
    let wal_bytes = dir_bytes(&vault_dir.join("wal"));
    let prometheus = shell_capture(
        "curl",
        &[
            "-fsS",
            "-m",
            "3",
            "http://127.0.0.1:9090/api/v1/query?query=up",
        ],
    );
    let tei = json!({
        "tei_8088": shell_capture("curl", &["-fsS", "-m", "2", "http://127.0.0.1:8088/health"]),
        "tei_8089": shell_capture("curl", &["-fsS", "-m", "2", "http://127.0.0.1:8089/health"]),
        "tei_8090": shell_capture("curl", &["-fsS", "-m", "2", "http://127.0.0.1:8090/health"]),
    });

    let report = json!({
        "issue": ISSUE,
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "scale_cx": scale_cx,
        "dim": dim,
        "query_count": queries,
        "index_build_ms": build_ms,
        "budgets_us": {
            "single_lens_p99": 5_000,
            "rrf_6_lens_p99": 15_000,
            "pipeline_p99": 60_000,
            "explain_overhead": 3_000,
            "ingest_1_slot_p95": 5_000,
            "ingest_15_slot_p95": 20_000
        },
        "known_io": known,
        "search": {
            "single_lens": single.to_json(5_000),
            "rrf_6_lens": rrf.to_json(15_000),
            "pipeline": pipeline.to_json(60_000),
            "explain_true": explain.to_json(0),
            "explain_overhead_us": explain_overhead,
            "explain_overhead_pass": explain_overhead <= 3_000
        },
        "ingest": ingest,
        "edges": edge_errors,
        "resident_tei_probe": tei,
        "prometheus_up_query": prometheus,
        "vault_sot": {
            "path": vault_dir.display().to_string(),
            "bytes": vault_bytes,
            "sst_files": vault_sst_files,
            "wal_bytes": wal_bytes
        }
    });

    let artifact = root.join("issue640-embedded-scale-latency-series.json");
    fs::write(&artifact, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    let bytes = fs::read(&artifact).unwrap();
    let readback: Value = serde_json::from_slice(&bytes).unwrap();
    let digest = digest_hex(&bytes);

    println!("ISSUE640_FSV_ROOT={}", root.display());
    println!("ISSUE640_LATENCY_ARTIFACT={}", artifact.display());
    println!("ISSUE640_LATENCY_ARTIFACT_BLAKE3={digest}");
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(readback["scale_cx"], scale_cx);
    assert!(readback["known_io"]["match"].as_bool().unwrap());
    if enforce_budgets(scale_cx) {
        assert!(readback["search"]["single_lens"]["pass"].as_bool().unwrap());
        assert!(readback["search"]["rrf_6_lens"]["pass"].as_bool().unwrap());
        assert!(readback["search"]["pipeline"]["pass"].as_bool().unwrap());
        assert!(
            readback["search"]["explain_overhead_pass"]
                .as_bool()
                .unwrap()
        );
        assert!(readback["ingest"]["one_slot"]["pass"].as_bool().unwrap());
        assert!(
            readback["ingest"]["fifteen_slot"]["pass"]
                .as_bool()
                .unwrap()
        );
    }
}
