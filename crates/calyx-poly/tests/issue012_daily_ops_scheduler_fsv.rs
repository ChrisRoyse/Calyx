//! Issue #12 - daily ops scheduler wiring to real graph/kernel and calibration jobs.
//!
//! Source of truth: scheduler state/report JSON plus delegated #73/#91/#111 reports read back from
//! disk and Graph CF readback inside the domain graph job.

#[path = "daily_ops_scheduler_fixture.rs"]
mod fixture;
#[allow(
    clippy::duplicate_mod,
    reason = "integration test fixture and FSV driver both include the same helper file"
)]
#[path = "fsv_support.rs"]
mod support;

use calyx_core::FixedClock;
use calyx_poly::daily_ops_scheduler::{DailyOpsSchedulerConfig, DailyOpsSchedulerDecision};
use calyx_ward::MIN_BAD_SCORES;
use fixture::{
    DailyOpsFixturePaths, TEST_TS, assert_c_drive, edge_invalid_config, open_vault,
    scheduler_config, scheduler_request, store_loom_constellations,
};
use serde_json::json;
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

#[test]
fn issue012_daily_ops_scheduler_known_truth_fsv() {
    let (root, _keep) = named_fsv_root(
        "POLY_ISSUE012_DAILY_OPS_FSV_ROOT",
        "poly-issue012-daily-ops-scheduler",
    );
    assert_c_drive(&root);
    reset_dir(&root);
    let paths = DailyOpsFixturePaths::new(root);
    let vault = open_vault(&paths.root.join("daily-vault"));
    let source_cx_ids = store_loom_constellations(&vault);
    let clock = FixedClock::new(TEST_TS);
    let config = scheduler_config();

    let first = run_tick(&paths, &vault, &source_cx_ids, config.clone(), &clock)
        .expect("first daily ops scheduler tick runs jobs");
    assert_eq!(
        first.report.decision,
        DailyOpsSchedulerDecision::RanDailyJobs
    );
    assert_eq!(first.state.job_invocation_count, 1);
    assert_eq!(
        first
            .domain_graph_run
            .as_ref()
            .expect("graph run")
            .report
            .kernel_edge_count,
        3
    );
    assert!(
        first
            .ward_run
            .as_ref()
            .expect("Ward run")
            .report
            .admission_ledger
            .admitted
    );
    assert_eq!(
        first
            .calibration_refit_run
            .as_ref()
            .expect("calibration run")
            .report
            .observation_count,
        30
    );

    let before_duplicate =
        calyx_poly::daily_ops_scheduler::read_daily_ops_scheduler_state(&first.state_path).unwrap();
    let duplicate = run_tick(&paths, &vault, &source_cx_ids, config.clone(), &clock)
        .expect("duplicate due slot skips");
    let after_duplicate =
        calyx_poly::daily_ops_scheduler::read_daily_ops_scheduler_state(&duplicate.state_path)
            .unwrap();
    assert_eq!(
        duplicate.report.decision,
        DailyOpsSchedulerDecision::SchedulerSkippedAlreadyRan
    );
    assert_eq!(after_duplicate.job_invocation_count, 1);
    assert_eq!(
        before_duplicate.job_invocation_count,
        after_duplicate.job_invocation_count
    );

    let invalid_cadence = edge_invalid_config(
        &paths,
        &vault,
        &source_cx_ids,
        DailyOpsSchedulerConfig {
            cadence_secs: 0,
            ..config.clone()
        },
        &clock,
    );
    let empty_job = edge_invalid_config(
        &paths,
        &vault,
        &source_cx_ids,
        DailyOpsSchedulerConfig {
            job_id: String::new(),
            ..config.clone()
        },
        &clock,
    );
    let job_id_mismatch = edge_invalid_config(
        &paths,
        &vault,
        &source_cx_ids,
        DailyOpsSchedulerConfig {
            job_id: "different-daily-job".to_string(),
            ..config
        },
        &clock,
    );

    let mut files = Vec::new();
    collect_files(&paths.root, &mut files);
    let report = json!({
        "issue": 12,
        "proof_claim": "A local daily scheduler tick drives the real domain graph/kernel build, Ward guard calibration/admission, and calibration-refit jobs, persists scheduler state/report artifacts, skips duplicate due slots before invoking daily jobs, and fails closed on malformed scheduler config.",
        "minimum_sufficient_proof_corpus": {
            "domain_graph_source_constellations": 2,
            "supplied_graph_edges": 4,
            "kernel_cycle_edges": 3,
            "ward_bad_anchor_rows": MIN_BAD_SCORES,
            "ward_good_anchor_rows": 1,
            "calibration_refit_rows": 30,
            "scheduler_ticks": 2,
            "malformed_config_edges": 3,
            "why_this_is_sufficient": "The selected corpus is the union of the existing minimum corpora for #73, #91, and #111: two constellations for real Loom/XTerm work, a three-edge kernel cycle plus one non-kernel component, Ward's exact 50 bad-score conformal floor plus one good row, and calibration's exact 30-row fitting floor. Two scheduler ticks prove first-run invocation and duplicate-slot skip.",
            "why_smaller_is_insufficient": "One constellation would not prove the domain graph build over multiple stored records; fewer than three kernel edges cannot form the computed-kernel cycle; 49 Ward bad rows or 29 calibration rows are rejected by the real engines; one scheduler tick cannot prove duplicate skip.",
            "why_larger_is_wasteful": "More markets, rows, or longer schedules would repeat the same scheduler, Graph CF, Ward, calibration, persistence, and readback paths without proving another #12 daily-ops invariant."
        },
        "source_of_truth": [
            "daily-ops-scheduler-state.json",
            "daily-ops-scheduler-report.json",
            "domain_graph_build_crypto.json",
            "Ward calibration report JSON",
            "calibration_refit_report.json",
            "Graph CF readback and CSR rebuild embedded in the domain graph report"
        ],
        "first_tick": first.report,
        "duplicate_edge": {
            "before": before_duplicate,
            "after": after_duplicate,
            "decision": duplicate.report.decision
        },
        "edge_cases": {
            "invalid_zero_cadence": invalid_cadence,
            "invalid_empty_job_id": empty_job,
            "state_job_id_mismatch": job_id_mismatch
        },
        "physical_files": files,
        "passed": true
    });
    let report_path = paths
        .root
        .join("issue012_daily_ops_scheduler_fsv_report.json");
    write_json(&report_path, &report);
    write_blake3sums(&paths.root);
    println!(
        "ISSUE012_DAILY_OPS_SCHEDULER_READBACK={}",
        report_path.display()
    );
}

fn run_tick<'a>(
    paths: &'a DailyOpsFixturePaths,
    vault: &'a calyx_aster::vault::AsterVault,
    source_cx_ids: &'a [calyx_core::CxId],
    config: DailyOpsSchedulerConfig,
    clock: &'a FixedClock,
) -> calyx_poly::Result<calyx_poly::daily_ops_scheduler::DailyOpsSchedulerRun> {
    calyx_poly::daily_ops_scheduler::run_daily_ops_scheduler_tick(scheduler_request(
        paths,
        vault,
        source_cx_ids,
        config,
        TEST_TS,
        clock,
    ))
}
