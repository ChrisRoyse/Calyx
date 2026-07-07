//! Issue #104 - reversible Anneal integration for index/fusion/tau tuning.
//!
//! Source of truth: physical replay corpus JSON, rollback parameter artifact, persisted
//! `.anneal/tripwire.toml`, report JSON, and append-only ledger JSONL.

use std::path::Path;

use calyx_anneal::{
    ConcatKey, IndexConfig, MatPlanConfig, ReplayAnchor, ReplayQuery, ShadowRevertReason,
    ShadowVerdict, TripwireMetric,
};
use calyx_core::{CxId, LensId};
use calyx_poly::{
    AnnealIntegrationArtifactRef, AnnealIntegrationMetricProfile, AnnealIntegrationMetricRow,
    AnnealIntegrationParamSet, AnnealIntegrationReport, AnnealIntegrationRequest,
    AnnealIntegrationStatus, AnnealIntegrationTripwireBounds,
    read_anneal_integration_ledger_entries, read_anneal_integration_report,
    run_anneal_integration_report,
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const GENERATED_AT_TS: u64 = 1_785_400_104;

#[test]
fn issue104_anneal_integration_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE104_FSV_ROOT", "poly-issue104-anneal");
    reset_dir(&root);

    let happy = happy_promotes_shadow_winner(&root);
    let regression = edge_metric_regression_auto_reverts(&root);
    let tripwire = edge_tripwire_crossing_auto_reverts(&root);
    let empty_replay = edge_empty_replay_auto_reverts(&root);
    let budget = edge_budget_exhaustion_auto_reverts(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 104,
        "proof_claim": "Poly wires calyx-anneal to tune index, fusion, and tau parameters only through held-out shadow replay, persists tripwire/report/ledger state, promotes only non-regressing candidates, and automatically reverts regressions, tripwire crossings, insufficient replay, and exhausted shadow budget.",
        "minimum_sufficient_corpus": {
            "replay_queries": 2,
            "parameter_families": ["index", "fusion", "tau"],
            "shadow_metrics": ["recall_at_k", "guard_far", "guard_frr", "search_p99", "ingest_p95"],
            "why_this_is_sufficient": "Two replay queries are the smallest corpus that proves the real ShadowExecutor aggregates more than one query and consumes one budget tick per query while exercising all three tuned parameter families and all five guarded metrics.",
            "why_smaller_is_insufficient": "One replay query cannot prove multi-query metric aggregation or second-query budget consumption; zero queries are a separate revert edge, not a promotion proof.",
            "why_larger_is_wasteful": "More replay rows would repeat the same Calyx Anneal shadow, tripwire, rollback, report, ledger, and readback paths without proving a new #104 behavior."
        },
        "happy_path": happy,
        "edge_cases": {
            "metric_regression": regression,
            "tripwire_crossing": tripwire,
            "empty_replay": empty_replay,
            "budget_exhaustion": budget
        },
        "physical_files": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!(
        "ISSUE104_ANNEAL_INTEGRATION_READBACK={}",
        readback_path.display()
    );
}

fn happy_promotes_shadow_winner(root: &Path) -> Value {
    let request = request_with_sources(
        root,
        "happy",
        replay_queries(),
        happy_candidate_metrics(),
        incumbent_metrics(),
        2,
    );
    let report = run_and_read(root, "happy", &request);
    assert_eq!(report.status, AnnealIntegrationStatus::Promoted);
    assert_eq!(report.active_after, request.candidate);
    assert_eq!(report.metrics.query_count, 2);
    assert!(report.changed_parameters.contains(&"index".to_string()));
    assert!(report.changed_parameters.contains(&"fusion".to_string()));
    assert!(report.changed_parameters.contains(&"tau".to_string()));
    assert!(Path::new(&report.tripwire_config_path).exists());
    assert_eq!(report.tripwire_thresholds.len(), 5);
    let ledger_path = Path::new(&request.ledger_dir).join("anneal_integration_ledger.jsonl");
    let ledger = read_anneal_integration_ledger_entries(&ledger_path).expect("read ledger");
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0], report.ledger_entry);
    json!({
        "status": report.status,
        "previous": report.previous,
        "candidate": report.candidate,
        "active_after": report.active_after,
        "changed_parameters": report.changed_parameters,
        "metrics": report.metrics,
        "tripwire_config_path": report.tripwire_config_path,
        "ledger_entry": report.ledger_entry,
        "report_hash": report.report_hash
    })
}

fn edge_metric_regression_auto_reverts(root: &Path) -> Value {
    let request = request_with_sources(
        root,
        "edge-regression",
        replay_queries(),
        regression_candidate_metrics(),
        incumbent_metrics(),
        2,
    );
    let report = run_and_read(root, "edge-regression", &request);
    assert_eq!(report.status, AnnealIntegrationStatus::Reverted);
    assert_eq!(report.active_after, request.current);
    assert!(matches!(
        &report.shadow_verdict,
        ShadowVerdict::Revert {
            reason: ShadowRevertReason::MetricRegression(TripwireMetric::RecallAtK),
            ..
        }
    ));
    edge_json(report)
}

fn edge_tripwire_crossing_auto_reverts(root: &Path) -> Value {
    let request = request_with_sources(
        root,
        "edge-tripwire",
        replay_queries(),
        tripwire_candidate_metrics(),
        tripwire_incumbent_metrics(),
        2,
    );
    let report = run_and_read(root, "edge-tripwire", &request);
    assert_eq!(report.status, AnnealIntegrationStatus::Reverted);
    assert_eq!(report.active_after, request.current);
    assert!(matches!(
        &report.shadow_verdict,
        ShadowVerdict::Revert {
            reason: ShadowRevertReason::TripwireCrossed(TripwireMetric::SearchP99),
            ..
        }
    ));
    edge_json(report)
}

fn edge_empty_replay_auto_reverts(root: &Path) -> Value {
    let request = request_with_sources(
        root,
        "edge-empty-replay",
        Vec::new(),
        Vec::new(),
        Vec::new(),
        1,
    );
    let report = run_and_read(root, "edge-empty-replay", &request);
    assert_eq!(report.status, AnnealIntegrationStatus::Reverted);
    assert_eq!(report.active_after, request.current);
    assert!(matches!(
        &report.shadow_verdict,
        ShadowVerdict::Revert {
            reason: ShadowRevertReason::InsufficientReplay,
            ..
        }
    ));
    assert_eq!(report.metrics.query_count, 0);
    edge_json(report)
}

fn edge_budget_exhaustion_auto_reverts(root: &Path) -> Value {
    let request = request_with_sources(
        root,
        "edge-budget",
        replay_queries(),
        happy_candidate_metrics(),
        incumbent_metrics(),
        1,
    );
    let report = run_and_read(root, "edge-budget", &request);
    assert_eq!(report.status, AnnealIntegrationStatus::Reverted);
    assert_eq!(report.active_after, request.current);
    assert!(matches!(
        &report.shadow_verdict,
        ShadowVerdict::Revert {
            reason: ShadowRevertReason::BudgetExhausted,
            ..
        }
    ));
    assert_eq!(report.metrics.query_count, 1);
    edge_json(report)
}

fn run_and_read(
    root: &Path,
    dir: &str,
    request: &AnnealIntegrationRequest,
) -> AnnealIntegrationReport {
    let run = run_anneal_integration_report(request, &root.join(dir)).expect("anneal integration");
    let report = read_anneal_integration_report(&run.report_path).expect("read report");
    assert_eq!(report, run.report);
    assert_eq!(
        report.ledger_entry,
        *run.ledger_entries.last().expect("ledger entry")
    );
    report
}

fn edge_json(report: AnnealIntegrationReport) -> Value {
    json!({
        "status": report.status,
        "reason": report.reason,
        "previous": report.previous,
        "candidate": report.candidate,
        "active_after": report.active_after,
        "metrics": report.metrics,
        "ledger_entry": report.ledger_entry,
        "report_hash": report.report_hash
    })
}

fn request_with_sources(
    root: &Path,
    dir: &str,
    replay_queries: Vec<ReplayQuery>,
    candidate_metrics: Vec<AnnealIntegrationMetricRow>,
    incumbent_metrics: Vec<AnnealIntegrationMetricRow>,
    budget_ticks: usize,
) -> AnnealIntegrationRequest {
    let case_dir = root.join(dir);
    let replay_path = case_dir.join("held_out_replay.json");
    write_json(
        &replay_path,
        &serde_json::to_value(&replay_queries).expect("replay json"),
    );
    let rollback_path = case_dir.join("rollback_parameters.json");
    let current = current_params();
    write_json(
        &rollback_path,
        &serde_json::to_value(&current).expect("rollback json"),
    );
    AnnealIntegrationRequest {
        domain: "crypto".to_string(),
        scope_id: "crypto:index-fusion-tau".to_string(),
        generated_at_ts: GENERATED_AT_TS,
        replay_seed: 104,
        replay_artifact: artifact_ref(&replay_path),
        rollback_artifact: artifact_ref(&rollback_path),
        tripwire_vault: case_dir.join("tripwire-vault").display().to_string(),
        ledger_dir: case_dir.join("ledger").display().to_string(),
        current,
        candidate: candidate_params(),
        tripwire_bounds: tripwire_bounds(),
        replay_queries,
        incumbent_metrics,
        candidate_metrics,
        budget_ticks,
    }
}

fn current_params() -> AnnealIntegrationParamSet {
    let a = LensId::from_bytes([1; 16]);
    let b = LensId::from_bytes([2; 16]);
    AnnealIntegrationParamSet {
        version: "crypto:index-fusion-tau:current".to_string(),
        index: IndexConfig {
            hnsw_ef: 64,
            hnsw_m: 16,
            diskann_beamwidth: 32,
            spann_cutoff: 1024,
            quant_bits: 16,
        },
        fusion: MatPlanConfig {
            eager_pairs: vec![(a, b)],
            indexed_concat_keys: vec![ConcatKey::new(a, b)],
        },
        tau: 0.42,
    }
}

fn candidate_params() -> AnnealIntegrationParamSet {
    let a = LensId::from_bytes([1; 16]);
    let b = LensId::from_bytes([2; 16]);
    let c = LensId::from_bytes([3; 16]);
    AnnealIntegrationParamSet {
        version: "crypto:index-fusion-tau:candidate".to_string(),
        index: IndexConfig {
            hnsw_ef: 128,
            hnsw_m: 32,
            diskann_beamwidth: 64,
            spann_cutoff: 2048,
            quant_bits: 8,
        },
        fusion: MatPlanConfig {
            eager_pairs: vec![(a, b), (a, c)],
            indexed_concat_keys: vec![ConcatKey::new(a, b), ConcatKey::new(a, c)],
        },
        tau: 0.36,
    }
}

fn replay_queries() -> Vec<ReplayQuery> {
    vec![
        ReplayQuery {
            query_id: 10,
            query_vector: vec![0.25, 0.5],
            expected_top_k: vec![ReplayAnchor {
                cx_id: CxId::from_bytes([10; 16]),
                similarity: 0.75,
            }],
        },
        ReplayQuery {
            query_id: 20,
            query_vector: vec![0.5, 0.25],
            expected_top_k: vec![ReplayAnchor {
                cx_id: CxId::from_bytes([20; 16]),
                similarity: 0.5,
            }],
        },
    ]
}

fn tripwire_bounds() -> AnnealIntegrationTripwireBounds {
    AnnealIntegrationTripwireBounds {
        recall_at_k_min: 0.90,
        guard_far_max: 0.03,
        guard_frr_max: 0.04,
        search_p99_max_ms: 150.0,
        ingest_p95_max_ms: 120.0,
        hysteresis: 0.0,
    }
}

fn incumbent_metrics() -> Vec<AnnealIntegrationMetricRow> {
    rows(&[
        profile(0.94, 0.010, 0.020, 110.0, 100.0),
        profile(0.95, 0.011, 0.022, 112.0, 102.0),
    ])
}

fn happy_candidate_metrics() -> Vec<AnnealIntegrationMetricRow> {
    rows(&[
        profile(0.98, 0.005, 0.010, 95.0, 80.0),
        profile(0.96, 0.008, 0.012, 90.0, 85.0),
    ])
}

fn regression_candidate_metrics() -> Vec<AnnealIntegrationMetricRow> {
    rows(&[
        profile(0.93, 0.005, 0.010, 95.0, 80.0),
        profile(0.93, 0.008, 0.012, 90.0, 85.0),
    ])
}

fn tripwire_incumbent_metrics() -> Vec<AnnealIntegrationMetricRow> {
    rows(&[
        profile(0.94, 0.010, 0.020, 170.0, 100.0),
        profile(0.95, 0.011, 0.022, 170.0, 102.0),
    ])
}

fn tripwire_candidate_metrics() -> Vec<AnnealIntegrationMetricRow> {
    rows(&[
        profile(0.98, 0.005, 0.010, 160.0, 80.0),
        profile(0.96, 0.008, 0.012, 160.0, 85.0),
    ])
}

fn rows(profiles: &[AnnealIntegrationMetricProfile]) -> Vec<AnnealIntegrationMetricRow> {
    let ids = [10_u64, 20_u64];
    profiles
        .iter()
        .enumerate()
        .map(|(idx, metrics)| AnnealIntegrationMetricRow {
            query_id: ids[idx],
            metrics: *metrics,
        })
        .collect()
}

fn profile(
    recall_at_k: f64,
    guard_far: f64,
    guard_frr: f64,
    search_p99_ms: f64,
    ingest_p95_ms: f64,
) -> AnnealIntegrationMetricProfile {
    AnnealIntegrationMetricProfile {
        recall_at_k,
        guard_far,
        guard_frr,
        search_p99_ms,
        ingest_p95_ms,
    }
}

fn artifact_ref(path: &Path) -> AnnealIntegrationArtifactRef {
    AnnealIntegrationArtifactRef {
        path: path.display().to_string(),
        blake3: hex(blake3::hash(&std::fs::read(path).expect("hash artifact")).as_bytes()),
    }
}
