use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use calyx_core::{CxId, FixedClock};
use calyx_lodestar::{
    CALYX_KERNEL_RECALL_BELOW_GATE, GroundednessReport, InMemoryAnnIndex, InMemoryCorpus, Kernel,
    LodestarError, RecallQuery, RecallReport, RecallTestParams, build_kernel_index,
    kernel_recall_gate, kernel_recall_test, kernel_recall_test_with_clock,
};
use serde_json::json;

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn kernel(members: Vec<CxId>) -> Kernel {
    Kernel {
        kernel_id: cx(66),
        panel_version: 1,
        anchor_kind: Some("synthetic_anchor".to_string()),
        corpus_shard_hash: [6; 32],
        members: members.clone(),
        kernel_graph: members,
        groundedness: GroundednessReport {
            reached_anchor: 1.0,
            unanchored_members: Vec::new(),
        },
        recall: RecallReport::default(),
        built_at_millis: 1,
        estimator_provenance: "test".to_string(),
        warnings: Vec::new(),
    }
}

fn one_hot_rows(count: u8) -> Vec<RecallQuery> {
    (1..=count)
        .map(|seed| {
            let mut vector = vec![0.0; count as usize];
            vector[seed as usize - 1] = 1.0;
            RecallQuery {
                cx_id: cx(seed),
                vector,
            }
        })
        .collect()
}

fn small_rows(count: u8) -> Vec<RecallQuery> {
    (1..=count)
        .map(|seed| RecallQuery {
            cx_id: cx(seed),
            vector: vec![
                seed as f32,
                (seed % 7) as f32 + 1.0,
                (seed % 5) as f32 + 2.0,
            ],
        })
        .collect()
}

fn embeddings(rows: &[RecallQuery]) -> BTreeMap<CxId, Vec<f32>> {
    rows.iter()
        .map(|query| (query.cx_id, query.vector.clone()))
        .collect()
}

fn fsv_root(case: &str) -> PathBuf {
    let base = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph33-t04"));
    base.join(case)
}

fn write_readback(case: &str, name: &str, value: serde_json::Value) {
    let root = fsv_root(case);
    fs::create_dir_all(&root).expect("create readback root");
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("readback write");
    println!("PH33_T04_READBACK={}", path.display());
}

#[test]
fn recall_test_perfect_kernel_ratio_is_one() {
    let rows = one_hot_rows(10);
    let corpus = InMemoryCorpus::new("perfect-synthetic", rows.clone());
    let full = InMemoryAnnIndex::new(rows.clone()).unwrap();
    let index = build_kernel_index(
        &kernel(rows.iter().map(|query| query.cx_id).collect()),
        &embeddings(&rows),
    )
    .unwrap();
    let params = RecallTestParams {
        held_out_fraction: 1.0,
        top_k: 10,
        ..RecallTestParams::default()
    };

    let report = kernel_recall_test(&index, &full, &corpus, &params).unwrap();

    println!(
        "RECALL_TEST_PERFECT ratio={} kernel_only={} held_out={:?}",
        report.ratio, report.kernel_only, report.held_out
    );
    write_readback(
        "perfect",
        "recall-test-perfect.json",
        json!({ "report": report }),
    );

    assert_eq!(report.kernel_only, 1.0);
    assert_eq!(report.ratio, 1.0);
    assert_eq!(report.warning, None);
    assert_eq!(report.n_queries_tested, 10);
}

#[test]
fn recall_test_degraded_kernel_emits_below_gate_warning() {
    let rows = one_hot_rows(10);
    let corpus = InMemoryCorpus::new("degraded-synthetic", rows.clone());
    let full = InMemoryAnnIndex::new(rows.clone()).unwrap();
    let index = build_kernel_index(&kernel(vec![cx(1)]), &embeddings(&rows)).unwrap();
    let params = RecallTestParams {
        held_out_fraction: 1.0,
        top_k: 10,
        min_recall_ratio: 0.95,
        ..RecallTestParams::default()
    };

    let report = kernel_recall_test(&index, &full, &corpus, &params).unwrap();

    println!(
        "RECALL_TEST_DEGRADED ratio={} warning={:?}",
        report.ratio, report.warning
    );
    write_readback(
        "degraded",
        "recall-test-degraded.json",
        json!({ "report": report }),
    );

    assert!((report.kernel_only - 0.1).abs() <= 1e-6);
    assert!(report.ratio < 0.95);
    assert!(
        report
            .warning
            .as_deref()
            .unwrap()
            .starts_with(CALYX_KERNEL_RECALL_BELOW_GATE)
    );
}

#[test]
fn recall_gate_passes_and_fails_closed_on_below_gate() {
    let rows = one_hot_rows(10);
    let corpus = InMemoryCorpus::new("gate-synthetic", rows.clone());
    let full = InMemoryAnnIndex::new(rows.clone()).unwrap();
    let embeddings = embeddings(&rows);
    let perfect = build_kernel_index(
        &kernel(rows.iter().map(|query| query.cx_id).collect()),
        &embeddings,
    )
    .unwrap();
    let degraded = build_kernel_index(&kernel(vec![cx(1)]), &embeddings).unwrap();
    let params = RecallTestParams {
        held_out_fraction: 1.0,
        top_k: 10,
        min_recall_ratio: 0.95,
        ..RecallTestParams::default()
    };

    let pass = kernel_recall_gate(&perfect, &full, &corpus, &params).unwrap();
    let report_only = kernel_recall_test(&degraded, &full, &corpus, &params).unwrap();
    let fail = kernel_recall_gate(&degraded, &full, &corpus, &params).unwrap_err();

    println!(
        "RECALL_GATE pass_ratio={} report_warning={:?} fail_code={}",
        pass.ratio,
        report_only.warning,
        fail.code()
    );
    write_readback(
        "gate",
        "recall-gate-fail-closed.json",
        json!({
            "pass": pass,
            "report_only_below_gate": report_only,
            "gate_error": fail.code(),
            "gate_error_message": fail.to_string(),
        }),
    );

    assert_eq!(pass.warning, None);
    assert!(
        report_only
            .warning
            .as_deref()
            .unwrap()
            .starts_with(CALYX_KERNEL_RECALL_BELOW_GATE)
    );
    assert_eq!(fail.code(), CALYX_KERNEL_RECALL_BELOW_GATE);
}

#[test]
fn recall_test_sampling_is_deterministic_and_ceil_counted() {
    let rows = small_rows(100);
    let corpus = InMemoryCorpus::new("sample-synthetic", rows.clone());
    let full = InMemoryAnnIndex::new(rows.clone()).unwrap();
    let index = build_kernel_index(
        &kernel(rows.iter().map(|query| query.cx_id).collect()),
        &embeddings(&rows),
    )
    .unwrap();
    let params = RecallTestParams {
        held_out_fraction: 0.1,
        top_k: 5,
        rng_seed: 42,
        ..RecallTestParams::default()
    };

    let first = kernel_recall_test(&index, &full, &corpus, &params).unwrap();
    let second = kernel_recall_test(&index, &full, &corpus, &params).unwrap();

    println!(
        "RECALL_TEST_DETERMINISTIC held_out={:?} n_queries={}",
        first.held_out, first.n_queries_tested
    );
    write_readback(
        "deterministic",
        "recall-test-deterministic.json",
        json!({
            "first": first,
            "second": second,
            "held_out_equal": first.held_out == second.held_out,
        }),
    );

    assert_eq!(first.held_out, second.held_out);
    assert_eq!(first.n_queries_tested, 10);
    assert_eq!(second.n_queries_tested, 10);
}

#[test]
fn recall_test_edges_and_clock_seed_fail_closed() {
    let rows = small_rows(10);
    let corpus = InMemoryCorpus::new("edge-synthetic", rows.clone());
    let full = InMemoryAnnIndex::new(rows.clone()).unwrap();
    let index = build_kernel_index(
        &kernel(rows.iter().map(|query| query.cx_id).collect()),
        &embeddings(&rows),
    )
    .unwrap();
    let all_params = RecallTestParams {
        held_out_fraction: 1.0,
        top_k: 3,
        ..RecallTestParams::default()
    };
    let all = kernel_recall_test(&index, &full, &corpus, &all_params).unwrap();
    let zero = kernel_recall_test(
        &index,
        &full,
        &corpus,
        &RecallTestParams {
            held_out_fraction: 0.0,
            ..RecallTestParams::default()
        },
    )
    .unwrap_err();
    let invalid_ratio = kernel_recall_test(
        &index,
        &full,
        &corpus,
        &RecallTestParams {
            min_recall_ratio: 1.01,
            ..RecallTestParams::default()
        },
    )
    .unwrap_err();
    let invalid_top_k = kernel_recall_test(
        &index,
        &full,
        &corpus,
        &RecallTestParams {
            top_k: 0,
            ..RecallTestParams::default()
        },
    )
    .unwrap_err();
    let empty_corpus = InMemoryCorpus::new("empty-synthetic", Vec::new());
    let empty = kernel_recall_gate(&index, &full, &empty_corpus, &all_params).unwrap_err();
    let clock_params = RecallTestParams {
        held_out_fraction: 0.3,
        top_k: 3,
        rng_seed: 0,
        ..RecallTestParams::default()
    };
    let clock = FixedClock::new(1_785_400_000);
    let clock_first =
        kernel_recall_test_with_clock(&index, &full, &corpus, &clock_params, &clock).unwrap();
    let clock_second =
        kernel_recall_test_with_clock(&index, &full, &corpus, &clock_params, &clock).unwrap();

    println!(
        "RECALL_TEST_EDGES all={} zero={} invalid_ratio={} invalid_top_k={} clock_held_out={:?}",
        all.n_queries_tested,
        zero.code(),
        invalid_ratio.code(),
        invalid_top_k.code(),
        clock_first.held_out
    );
    write_readback(
        "edges",
        "recall-test-edges.json",
        json!({
            "all": all,
            "zero": zero.code(),
            "empty_corpus": empty.code(),
            "invalid_ratio": invalid_ratio.code(),
            "invalid_top_k": invalid_top_k.code(),
            "clock_first": clock_first,
            "clock_second": clock_second,
            "clock_held_out_equal": clock_first.held_out == clock_second.held_out,
        }),
    );

    assert_eq!(all.n_queries_tested, 10);
    assert!(matches!(zero, LodestarError::RecallEmptyCorpus));
    assert!(matches!(empty, LodestarError::RecallEmptyCorpus));
    assert_eq!(invalid_ratio.code(), "CALYX_RECALL_INVALID_PARAMS");
    assert_eq!(invalid_top_k.code(), "CALYX_RECALL_INVALID_PARAMS");
    assert_eq!(clock_first.held_out, clock_second.held_out);
}
