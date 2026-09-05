//! Issue #77 - outcome-backfill scheduler FSV.
//!
//! Source of truth: physical provisional diagnostics, per-job resolved diagnostics,
//! trust-transition JSON, and the outcome-backfill report written to disk and read back.

use std::path::Path;

use calyx_assay::{TotalCorrelationConfig, TrustTag};
use calyx_core::{Anchor, FixedClock};
use calyx_poly::diagnostics_store;
use calyx_poly::grounding::{ERR_BACKFILL_CONTRADICTION, TrustTransition};
use calyx_poly::no_lookahead::NoLookaheadTiming;
use calyx_poly::outcome_backfill::{
    ERR_BACKFILL_CORPUS_MISMATCH, ERR_BACKFILL_EMPTY, ERR_BACKFILL_NOT_PROVISIONAL,
    ERR_BACKFILL_NOT_TRUSTED, OutcomeBackfillJob, read_outcome_backfill_report,
    run_outcome_backfill_schedule,
};
use calyx_poly::panel_diagnostics::{
    PanelDiagnosticsConfig, PanelMatrix, compute_panel_diagnostics, read_panel_diagnostics,
    write_panel_diagnostics,
};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
// calyx-shared-module: path=synthetic_panels.rs alias=__calyx_shared_synthetic_panels_rs local=synthetic visibility=private
use crate::__calyx_shared_synthetic_panels_rs as synthetic;

use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};
use synthetic::{SyntheticPanel, independent, proxy_up24h, resolved_anchor};

const PANEL_VERSION: u32 = 1;
const SLOT_COUNT: usize = 3;
const MIN_TRUSTED_CORPUS: usize = 150;
const BELOW_FLOOR_CORPUS: usize = 149;
const FEATURE_MAX_OBSERVED_AT: u64 = 1_785_400_000;
const SNAPSHOT_OBSERVED_AT: u64 = 1_785_500_000;
const RESOLUTION_OBSERVED_AT: u64 = 1_785_600_000;
const BACKFILL_OBSERVED_AT: u64 = 1_785_700_000;

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
fn issue077_outcome_backfill_scheduler_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE077_FSV_ROOT", "poly-issue077-outcome-backfill");
    reset_dir(&root);
    let clock = FixedClock::new(1_785_700_000);
    let config = cfg();

    let happy = happy_backfill_remeasures_and_upgrades(&root, &clock, &config);
    let empty = edge_empty_schedule_fails_closed(&root, &clock, &config);
    let trusted_source = edge_trusted_source_fails_closed(&root, &clock, &config);
    let mismatch = edge_corpus_mismatch_fails_closed(&root, &clock, &config);
    let below_floor = edge_below_floor_resolved_recompute_fails_closed(&root, &clock, &config);
    let contradiction = edge_contradicting_resolution_fails_closed(&root, &clock, &config);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 77,
        "proof_claim": "The outcome-backfill scheduler reads a Provisional diagnostic, remeasures the same historical panel against resolved UMA anchors, persists a trust transition and resolved diagnostic, and upgrades only to readback-proven Trusted evidence.",
        "minimum_sufficient_corpus": {
            "valid_backfill_samples": MIN_TRUSTED_CORPUS,
            "slot_count": SLOT_COUNT,
            "below_floor_edge_samples": BELOW_FLOOR_CORPUS,
            "why_this_is_sufficient": "150 rows is exactly the assay total-correlation and interaction-information quorum for a 3-slot panel, so it exercises the real non-provisional recompute path.",
            "why_smaller_is_insufficient": "149 rows stays below the 50-per-slot quorum and can only prove the fail-closed below-floor edge, not a Trusted upgrade.",
            "why_larger_is_wasteful": "more rows would repeat the same scheduler, promotion, recompute, write, and readback paths without proving an additional issue #77 invariant."
        },
        "happy_path": happy,
        "edge_cases": {
            "empty_schedule": empty,
            "trusted_source": trusted_source,
            "corpus_mismatch": mismatch,
            "below_floor_resolved_recompute": below_floor,
            "contradicting_resolution": contradiction
        },
        "physical_files": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!(
        "ISSUE077_OUTCOME_BACKFILL_READBACK={}",
        readback_path.display()
    );
}

fn happy_backfill_remeasures_and_upgrades(
    root: &Path,
    clock: &FixedClock,
    config: &PanelDiagnosticsConfig,
) -> Value {
    let panel = independent(77_001, MIN_TRUSTED_CORPUS, SLOT_COUNT);
    let provisional_path = write_provisional(
        root,
        "happy/provisional",
        "issue77_happy",
        &panel,
        clock,
        config,
    );
    let job = job(
        "happy-upgrade",
        "issue77_happy",
        provisional_path.clone(),
        resolved_matrix(&panel),
        true,
        true,
    );
    let run = run_outcome_backfill_schedule(&root.join("happy/run"), &[job], clock, config)
        .expect("happy backfill run");
    let report = read_outcome_backfill_report(&run.report_path).expect("read report");
    assert_eq!(report, run.report);
    assert_eq!(report.job_count, 1);
    assert_eq!(report.completed_count, 1);
    let job_report = &report.jobs[0];
    assert_eq!(job_report.before_trust, TrustTag::Provisional);
    assert_eq!(job_report.after_trust, TrustTag::Trusted);
    assert_eq!(job_report.n_samples, MIN_TRUSTED_CORPUS);
    assert_eq!(job_report.slot_keys.len(), SLOT_COUNT);
    assert_eq!(job_report.transition.from, TrustTag::Provisional);
    assert_eq!(job_report.transition.to, TrustTag::Trusted);

    let resolved_diag =
        read_panel_diagnostics(Path::new(&job_report.resolved_record_path)).expect("resolved diag");
    assert_eq!(resolved_diag.trust, TrustTag::Trusted);
    assert_eq!(resolved_diag.n_samples, MIN_TRUSTED_CORPUS);
    let transition: TrustTransition =
        diagnostics_store::read_json(Path::new(&job_report.transition_path))
            .expect("transition readback");
    assert_eq!(transition, job_report.transition);

    json!({
        "report_path": run.report_path.display().to_string(),
        "provisional_record_path": provisional_path.display().to_string(),
        "resolved_record_path": job_report.resolved_record_path,
        "transition_path": job_report.transition_path,
        "no_lookahead": serde_json::to_value(&job_report.no_lookahead).expect("no-lookahead JSON"),
        "before_trust": format!("{:?}", job_report.before_trust),
        "after_trust": format!("{:?}", job_report.after_trust),
        "n_samples": job_report.n_samples,
        "slot_keys": job_report.slot_keys,
        "resolved_provenance_hash": job_report.resolved_provenance_hash,
        "transition": serde_json::to_value(&job_report.transition).expect("transition JSON")
    })
}

fn edge_empty_schedule_fails_closed(
    root: &Path,
    clock: &FixedClock,
    config: &PanelDiagnosticsConfig,
) -> Value {
    let err = run_outcome_backfill_schedule(&root.join("edge-empty"), &[], clock, config)
        .expect_err("empty schedule rejected");
    assert_eq!(err.code(), ERR_BACKFILL_EMPTY);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_trusted_source_fails_closed(
    root: &Path,
    clock: &FixedClock,
    config: &PanelDiagnosticsConfig,
) -> Value {
    let panel = independent(77_002, MIN_TRUSTED_CORPUS, SLOT_COUNT);
    let matrix = resolved_matrix(&panel);
    let trusted = compute_panel_diagnostics(
        "issue77_trusted_source",
        PANEL_VERSION,
        &matrix,
        clock,
        config,
    )
    .expect("trusted diagnostic");
    assert_eq!(trusted.trust, TrustTag::Trusted);
    let trusted_path = write_panel_diagnostics(&root.join("edge-trusted-source/source"), &trusted)
        .expect("write trusted source");
    let job = job(
        "trusted-source",
        "issue77_trusted_source",
        trusted_path,
        resolved_matrix(&panel),
        true,
        true,
    );
    let err =
        run_outcome_backfill_schedule(&root.join("edge-trusted-source/run"), &[job], clock, config)
            .expect_err("already trusted source rejected");
    assert_eq!(err.code(), ERR_BACKFILL_NOT_PROVISIONAL);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_corpus_mismatch_fails_closed(
    root: &Path,
    clock: &FixedClock,
    config: &PanelDiagnosticsConfig,
) -> Value {
    let panel = independent(77_003, MIN_TRUSTED_CORPUS, SLOT_COUNT);
    let provisional_path = write_provisional(
        root,
        "edge-corpus-mismatch/source",
        "issue77_mismatch",
        &panel,
        clock,
        config,
    );
    let mismatched_panel = independent(77_004, MIN_TRUSTED_CORPUS + 1, SLOT_COUNT);
    let job = job(
        "corpus-mismatch",
        "issue77_mismatch",
        provisional_path,
        resolved_matrix(&mismatched_panel),
        true,
        true,
    );
    let err = run_outcome_backfill_schedule(
        &root.join("edge-corpus-mismatch/run"),
        &[job],
        clock,
        config,
    )
    .expect_err("corpus mismatch rejected");
    assert_eq!(err.code(), ERR_BACKFILL_CORPUS_MISMATCH);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_below_floor_resolved_recompute_fails_closed(
    root: &Path,
    clock: &FixedClock,
    config: &PanelDiagnosticsConfig,
) -> Value {
    let panel = independent(77_005, BELOW_FLOOR_CORPUS, SLOT_COUNT);
    let provisional_path = write_provisional(
        root,
        "edge-below-floor/source",
        "issue77_below_floor",
        &panel,
        clock,
        config,
    );
    let job = job(
        "below-floor",
        "issue77_below_floor",
        provisional_path,
        resolved_matrix(&panel),
        true,
        true,
    );
    let err =
        run_outcome_backfill_schedule(&root.join("edge-below-floor/run"), &[job], clock, config)
            .expect_err("below-floor recompute rejected");
    assert_eq!(err.code(), ERR_BACKFILL_NOT_TRUSTED);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_contradicting_resolution_fails_closed(
    root: &Path,
    clock: &FixedClock,
    config: &PanelDiagnosticsConfig,
) -> Value {
    let panel = independent(77_006, MIN_TRUSTED_CORPUS, SLOT_COUNT);
    let provisional_path = write_provisional(
        root,
        "edge-contradiction/source",
        "issue77_contradiction",
        &panel,
        clock,
        config,
    );
    let job = job(
        "contradiction",
        "issue77_contradiction",
        provisional_path,
        resolved_matrix(&panel),
        true,
        false,
    );
    let err =
        run_outcome_backfill_schedule(&root.join("edge-contradiction/run"), &[job], clock, config)
            .expect_err("contradicting resolution rejected");
    assert_eq!(err.code(), ERR_BACKFILL_CONTRADICTION);
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

fn job(
    job_id: &str,
    domain: &str,
    provisional_record_path: std::path::PathBuf,
    resolved_matrix: PanelMatrix,
    proxy_outcome: bool,
    resolved_outcome: bool,
) -> OutcomeBackfillJob {
    let mut proxy_anchor = proxy_up24h(proxy_outcome, 0);
    proxy_anchor.observed_at = SNAPSHOT_OBSERVED_AT;
    let mut resolved_anchor = resolved_anchor(resolved_outcome, 0);
    resolved_anchor.observed_at = RESOLUTION_OBSERVED_AT;
    OutcomeBackfillJob {
        job_id: job_id.to_string(),
        domain: domain.to_string(),
        panel_version: PANEL_VERSION,
        provisional_record_path,
        proxy_anchor,
        resolved_anchor,
        timing: timing(),
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
