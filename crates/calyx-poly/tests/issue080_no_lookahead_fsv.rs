//! Issue #80 - no-look-ahead guarantees for as-of features and anchor timing.
//!
//! Source of truth: persisted no-look-ahead report, outcome-backfill report, and real AsterVault
//! anchor rows written to disk and read back.

use std::path::Path;

use calyx_assay::{TotalCorrelationConfig, TrustTag};
use calyx_aster::cf::{ColumnFamily, anchor_key};
use calyx_aster::vault::{AsterVault, VaultOptions, encode};
use calyx_core::{Anchor, AnchorKind, FixedClock, VaultId, VaultStore};
use calyx_poly::constellation::{resolution_anchor, resolution_label_anchor};
use calyx_poly::lenses::default_panel;
use calyx_poly::model::{MarketSnapshot, Resolution};
use calyx_poly::no_lookahead::{
    ERR_LOOKAHEAD_ANCHOR_BEFORE_RESOLUTION, ERR_LOOKAHEAD_BACKFILL_BEFORE_RESOLUTION,
    ERR_LOOKAHEAD_FEATURE_AFTER_SNAPSHOT, ERR_LOOKAHEAD_RESOLUTION_NOT_AFTER_SNAPSHOT,
    NoLookaheadTiming, read_no_lookahead_report, run_no_lookahead_report,
    validate_no_lookahead_timing, validate_resolution_anchor_timing,
};
use calyx_poly::outcome_backfill::{
    OutcomeBackfillJob, read_outcome_backfill_report, run_outcome_backfill_schedule,
};
use calyx_poly::panel_diagnostics::{
    PanelDiagnosticsConfig, PanelMatrix, compute_panel_diagnostics, write_panel_diagnostics,
};
use calyx_poly::pipeline::{ground_market, ingest_snapshot};
use calyx_poly::resolved_market_corpus::{ResolvedMarketInput, build_resolved_market_corpus};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
// calyx-shared-module: path=synthetic_panels.rs alias=__calyx_shared_synthetic_panels_rs local=synthetic visibility=private
use crate::__calyx_shared_synthetic_panels_rs as synthetic;

use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};
use synthetic::{SyntheticPanel, independent, proxy_up24h, resolved_anchor};

const PANEL_VERSION: u32 = 1;
const SLOT_COUNT: usize = 3;
const BACKFILL_SAMPLES: usize = 150;
const SNAPSHOT_TS: u64 = 1_785_500_000;
const RESOLVED_TS: u64 = 1_785_600_000;
const FEATURE_MAX_OBSERVED_AT: u64 = SNAPSHOT_TS * 1000 - 60_000;
const SNAPSHOT_OBSERVED_AT: u64 = SNAPSHOT_TS * 1000;
const RESOLUTION_OBSERVED_AT: u64 = RESOLVED_TS * 1000;
const BACKFILL_OBSERVED_AT: u64 = RESOLUTION_OBSERVED_AT + 60_000;
const VAULT_SALT: &[u8] = b"poly-issue080-no-lookahead";

fn cfg() -> PanelDiagnosticsConfig {
    PanelDiagnosticsConfig {
        tc: TotalCorrelationConfig {
            k: 3,
            bootstrap_resamples: 10,
            ..TotalCorrelationConfig::default()
        },
    }
}

#[test]
fn issue080_no_lookahead_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE080_FSV_ROOT", "poly-issue080-no-lookahead");
    reset_dir(&root);

    let happy = happy_path_proves_timing_backfill_and_vault_anchor(&root);
    let feature_after = edge_feature_after_snapshot_fails_closed();
    let resolution_before = edge_resolution_not_after_snapshot_fails_closed();
    let backfill_before = edge_backfill_before_resolution_fails_closed();
    let anchor_before = edge_anchor_before_resolution_fails_closed();
    let corpus_lookahead = edge_resolved_corpus_lookahead_fails_closed();

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 80,
        "proof_claim": "As-of feature timestamps are no newer than the snapshot, resolved anchors are written only after the snapshot and resolution, and outcome-backfill refuses timing that could leak labels into pre-resolution features.",
        "minimum_sufficient_corpus": {
            "timing_records": 1,
            "backfill_samples": BACKFILL_SAMPLES,
            "slot_count": SLOT_COUNT,
            "edge_records": 5,
            "why_this_is_sufficient": "One happy timing tuple exercises all scalar inequalities, one real vault row proves anchor write timing against the source of truth, and 150 generated rows is exactly the assay quorum needed for a Trusted 3-slot backfill recompute.",
            "why_smaller_is_insufficient": "Pure timing edges need only one row, but the integrated scheduler proof cannot go below 150 rows without becoming Provisional and failing a different guard.",
            "why_larger_is_wasteful": "More rows would repeat the same timestamp comparisons, scheduler write/readback, and vault anchor write without proving another issue #80 invariant."
        },
        "happy_path": happy,
        "edge_cases": {
            "feature_after_snapshot": feature_after,
            "resolution_not_after_snapshot": resolution_before,
            "backfill_before_resolution": backfill_before,
            "anchor_before_resolution": anchor_before,
            "resolved_corpus_lookahead": corpus_lookahead
        },
        "physical_files": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!("ISSUE080_NO_LOOKAHEAD_READBACK={}", readback_path.display());
}

fn happy_path_proves_timing_backfill_and_vault_anchor(root: &Path) -> Value {
    let timing = timing();
    let resolution = resolution();
    let anchors = vec![
        resolution_anchor(&resolution, 0),
        resolution_label_anchor(&resolution),
    ];
    let no_lookahead = run_no_lookahead_report(
        &root.join("happy/no-lookahead"),
        "issue080 persisted no-look-ahead report",
        timing.clone(),
        &anchors,
    )
    .expect("no-look-ahead report");
    let no_lookahead_readback =
        read_no_lookahead_report(&no_lookahead.report_path).expect("read no-look-ahead report");
    assert_eq!(no_lookahead_readback, no_lookahead.report);
    assert!(no_lookahead_readback.passed);
    assert_eq!(no_lookahead_readback.anchor_count, 2);

    let backfill = run_backfill_happy_path(root, &timing);
    let vault = run_vault_anchor_happy_path(root);

    json!({
        "no_lookahead_report_path": no_lookahead.report_path.display().to_string(),
        "no_lookahead_report": serde_json::to_value(&no_lookahead_readback).expect("no-lookahead JSON"),
        "outcome_backfill": backfill,
        "vault_anchor": vault
    })
}

fn run_backfill_happy_path(root: &Path, timing: &NoLookaheadTiming) -> Value {
    let clock = FixedClock::new(BACKFILL_OBSERVED_AT);
    let config = cfg();
    let panel = independent(80_001, BACKFILL_SAMPLES, SLOT_COUNT);
    let provisional_path = write_provisional(
        root,
        "happy/backfill/provisional",
        "issue80_happy",
        &panel,
        &clock,
        &config,
    );
    let job = backfill_job(
        provisional_path.clone(),
        resolved_matrix(&panel),
        timing.clone(),
    );
    let run =
        run_outcome_backfill_schedule(&root.join("happy/backfill/run"), &[job], &clock, &config)
            .expect("backfill schedule");
    let report = read_outcome_backfill_report(&run.report_path).expect("read backfill report");
    assert_eq!(report, run.report);
    assert_eq!(report.job_count, 1);
    let job_report = &report.jobs[0];
    assert_eq!(job_report.before_trust, TrustTag::Provisional);
    assert_eq!(job_report.after_trust, TrustTag::Trusted);
    assert_eq!(job_report.no_lookahead.timing, *timing);
    assert!(job_report.no_lookahead.passed);

    json!({
        "report_path": run.report_path.display().to_string(),
        "provisional_record_path": provisional_path.display().to_string(),
        "resolved_record_path": job_report.resolved_record_path,
        "n_samples": job_report.n_samples,
        "slot_keys": job_report.slot_keys,
        "no_lookahead": serde_json::to_value(&job_report.no_lookahead).expect("no-lookahead JSON")
    })
}

fn run_vault_anchor_happy_path(root: &Path) -> Value {
    let vault_dir = root.join("happy/vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .expect("open issue80 vault");
    let panel = default_panel(PANEL_VERSION, vec!["global".to_string()]);
    let snapshot = snapshot();
    let cx_id = ingest_snapshot(&vault, &panel, &snapshot, vault_id(), VAULT_SALT)
        .expect("ingest snapshot");
    let refs = ground_market(&vault, &[cx_id], &resolution(), 0).expect("ground market");
    vault.flush().expect("flush vault");
    let anchor_row = vault
        .read_cf_at(
            vault.snapshot(),
            ColumnFamily::Anchors,
            &anchor_key(cx_id, &AnchorKind::TestPass),
        )
        .expect("read anchor row")
        .expect("anchor row present");
    let anchor = encode::decode_anchor(&anchor_row).expect("decode anchor");
    assert!(
        anchor.observed_at > SNAPSHOT_OBSERVED_AT,
        "anchor must be after snapshot"
    );
    assert_eq!(anchor.observed_at, RESOLUTION_OBSERVED_AT);
    json!({
        "vault_dir": vault_dir.display().to_string(),
        "cx_id": cx_id.to_string(),
        "snapshot_observed_at": SNAPSHOT_OBSERVED_AT,
        "anchor_observed_at": anchor.observed_at,
        "ledger_seq": refs[0].seq,
        "anchor_source": anchor.source,
        "anchor_confidence": anchor.confidence
    })
}

fn edge_feature_after_snapshot_fails_closed() -> Value {
    let mut bad = timing();
    bad.feature_max_observed_at = bad.snapshot_observed_at + 1;
    let err = validate_no_lookahead_timing(&bad).expect_err("feature look-ahead rejected");
    assert_eq!(err.code(), ERR_LOOKAHEAD_FEATURE_AFTER_SNAPSHOT);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_resolution_not_after_snapshot_fails_closed() -> Value {
    let mut bad = timing();
    bad.resolution_observed_at = bad.snapshot_observed_at;
    bad.backfill_observed_at = bad.resolution_observed_at;
    let err = validate_no_lookahead_timing(&bad).expect_err("resolution look-ahead rejected");
    assert_eq!(err.code(), ERR_LOOKAHEAD_RESOLUTION_NOT_AFTER_SNAPSHOT);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_backfill_before_resolution_fails_closed() -> Value {
    let mut bad = timing();
    bad.backfill_observed_at = bad.resolution_observed_at - 1;
    let err = validate_no_lookahead_timing(&bad).expect_err("backfill before resolution rejected");
    assert_eq!(err.code(), ERR_LOOKAHEAD_BACKFILL_BEFORE_RESOLUTION);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_anchor_before_resolution_fails_closed() -> Value {
    let mut anchor = resolution_anchor(&resolution(), 0);
    anchor.observed_at = RESOLUTION_OBSERVED_AT - 1;
    let err = validate_resolution_anchor_timing(&timing(), &[anchor])
        .expect_err("pre-resolution anchor rejected");
    assert_eq!(err.code(), ERR_LOOKAHEAD_ANCHOR_BEFORE_RESOLUTION);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_resolved_corpus_lookahead_fails_closed() -> Value {
    let snapshot = snapshot();
    let mut resolution = resolution();
    resolution.resolved_ts = snapshot.snapshot_ts;
    let err = build_resolved_market_corpus(
        &[ResolvedMarketInput {
            snapshot: &snapshot,
            resolution: &resolution,
        }],
        PANEL_VERSION,
        VAULT_SALT,
        0.0,
    )
    .expect_err("resolved corpus look-ahead rejected");
    assert_eq!(err.code(), ERR_LOOKAHEAD_RESOLUTION_NOT_AFTER_SNAPSHOT);
    json!({"code": err.code(), "message": err.message()})
}

fn write_provisional(
    root: &Path,
    dir: &str,
    domain: &str,
    panel: &SyntheticPanel,
    clock: &FixedClock,
    config: &PanelDiagnosticsConfig,
) -> std::path::PathBuf {
    let matrix = proxy_matrix(panel);
    let diag = compute_panel_diagnostics(domain, PANEL_VERSION, &matrix, clock, config)
        .expect("compute provisional diagnostic");
    assert_eq!(diag.trust, TrustTag::Provisional);
    write_panel_diagnostics(&root.join(dir), &diag).expect("write provisional")
}

fn proxy_matrix(panel: &SyntheticPanel) -> PanelMatrix {
    let anchors: Vec<Anchor> = (0..panel.anchors.len())
        .map(|i| proxy_up24h(i % 2 == 0, i))
        .collect();
    PanelMatrix::new(panel.keys.clone(), panel.columns.clone(), anchors).expect("proxy matrix")
}

fn resolved_matrix(panel: &SyntheticPanel) -> PanelMatrix {
    let anchors: Vec<Anchor> = (0..panel.anchors.len())
        .map(|i| resolved_anchor(i % 2 == 0, i))
        .collect();
    PanelMatrix::new(panel.keys.clone(), panel.columns.clone(), anchors).expect("resolved matrix")
}

fn backfill_job(
    provisional_record_path: std::path::PathBuf,
    resolved_matrix: PanelMatrix,
    timing: NoLookaheadTiming,
) -> OutcomeBackfillJob {
    let mut proxy_anchor = proxy_up24h(true, 0);
    proxy_anchor.observed_at = SNAPSHOT_OBSERVED_AT;
    let mut resolved_anchor = resolved_anchor(true, 0);
    resolved_anchor.observed_at = RESOLUTION_OBSERVED_AT;
    OutcomeBackfillJob {
        job_id: "issue80-backfill".to_string(),
        domain: "issue80_happy".to_string(),
        panel_version: PANEL_VERSION,
        provisional_record_path,
        proxy_anchor,
        resolved_anchor,
        timing,
        resolved_matrix,
    }
}

fn timing() -> NoLookaheadTiming {
    NoLookaheadTiming {
        feature_max_observed_at: FEATURE_MAX_OBSERVED_AT,
        snapshot_observed_at: SNAPSHOT_OBSERVED_AT,
        resolution_observed_at: RESOLUTION_OBSERVED_AT,
        backfill_observed_at: BACKFILL_OBSERVED_AT,
    }
}

fn snapshot() -> MarketSnapshot {
    MarketSnapshot {
        token_id: "issue80-token".to_string(),
        condition_id: "issue80-condition".to_string(),
        outcome_index: 0,
        slug: "issue80-no-lookahead".to_string(),
        question: Some("Issue 080 no lookahead market?".to_string()),
        event_id: Some("issue80-event".to_string()),
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec!["issue80".to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: SNAPSHOT_TS,
        price: Some(0.62),
        mid: Some(0.62),
        best_bid: Some(0.61),
        best_ask: Some(0.63),
        spread: Some(0.02),
        tick_size: Some(0.01),
        volume_24h: Some(125_000.0),
        liquidity: Some(40_000.0),
        one_hour_change: Some(0.01),
        one_day_change: Some(-0.03),
        ofi: Some(0.2),
        yes_no_residual: Some(0.0),
        secs_to_resolution: Some(86_400.0),
        holders: vec![],
        makers: vec![],
        counterparty_volumes: vec![],
        onchain_fills: vec![],
        temporal_reference_ts: None,
        sequence_position: None,
        sequence_total: None,
        oracle_risk: Default::default(),
        book: Default::default(),
    }
}

fn resolution() -> Resolution {
    Resolution {
        condition_id: "issue80-condition".to_string(),
        winning_outcome_index: 0,
        winning_label: "YES".to_string(),
        resolved_ts: RESOLVED_TS,
        source: "uma".to_string(),
        disputed: false,
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
