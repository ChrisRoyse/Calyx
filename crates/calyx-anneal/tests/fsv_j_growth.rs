#[path = "fsv_j_growth/support.rs"]
mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use calyx_anneal::{
    AnchorId, AnnealLedgerAction, ArtifactPtr, AsterGrowthCf, CandidateAction, GoodhartChecker,
    GoodhartLedgerContext, GoodhartState, GoodhartViolation, GradientCandidate, GrowthCurve,
    HeldOutSet, IntelligenceGradient, JObjectiveContext, LensContributionDelta,
    add_goodhart_penalty_to_vault, compute_j, format_report, intelligence_report,
    record_goodhart_report, write_intelligence_report_snapshot,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{FixedClock, LensId};
use serde_json::json;
use support::{
    FSV_TS, Metrics, StaticWard, cf_rows, cf_sha256, open_ledger, open_vault,
    report_for_growth_step, rollback_gamed_candidate, seed_base_cf, write_json,
};

#[test]
#[ignore = "requires CALYX_ISSUE428_FSV_ROOT in a manual verification run"]
fn ph48_j_growth_goodhart_manual_fsv() {
    let root = PathBuf::from(
        std::env::var("CALYX_ISSUE428_FSV_ROOT").expect("set CALYX_ISSUE428_FSV_ROOT"),
    );
    assert!(
        !root.exists(),
        "choose a fresh CALYX_ISSUE428_FSV_ROOT; already exists: {}",
        root.display()
    );
    fs::create_dir_all(&root).expect("create FSV root");
    let vault_dir = root.join("stage10-vault");
    let vault = open_vault(&vault_dir);

    seed_base_cf(&vault);
    let base_before_hash = cf_sha256(&vault, ColumnFamily::Base);
    let base_before_rows = cf_rows(&vault, ColumnFamily::Base);

    let report = write_measured_j_report(&root, &vault);
    let growth = write_rising_growth_curve(&vault);
    let goodhart = reject_gamed_change(&vault_dir, &vault);

    let base_after_hash = cf_sha256(&vault, ColumnFamily::Base);
    let base_after_rows = cf_rows(&vault, ColumnFamily::Base);
    assert_eq!(base_before_hash, base_after_hash);

    let base_evidence = json!({
        "source_of_truth": format!("{}/cf/base", vault_dir.display()),
        "before_sha256": base_before_hash,
        "after_sha256": base_after_hash,
        "before_rows": base_before_rows,
        "after_rows": base_after_rows,
        "unchanged": true
    });
    write_json(&root.join("base-sha256.json"), &base_evidence);

    let evidence = json!({
        "source_of_truth": {
            "anneal_report": format!("{}/cf/anneal_report", vault_dir.display()),
            "anneal_growth": format!("{}/cf/anneal_growth", vault_dir.display()),
            "ledger": format!("{}/cf/ledger", vault_dir.display()),
            "base": format!("{}/cf/base", vault_dir.display())
        },
        "trigger": "PH48 deterministic Stage 10 integration FSV: measured J, 1000 synthetic growth samples, gamed correlated lens rejected and reverted",
        "seed": "0xDEADBEEF",
        "no_live_tei_calls": true,
        "j_is_measured": report,
        "growth_rises_on_corpus": growth,
        "gamed_change_rejected": goodhart,
        "no_data_deleted": base_evidence
    });
    write_json(&root.join("ph48_j_growth_goodhart.json"), &evidence);
    println!("ISSUE428_FSV_ROOT={}", root.display());
}

fn write_measured_j_report(root: &Path, vault: &AsterVault) -> serde_json::Value {
    let metrics = Metrics::report();
    let context = JObjectiveContext::new("issue428", 8);
    let j_value = compute_j(&context, &metrics).expect("compute measured J");
    assert!((j_value.j - 11.1).abs() < 1e-12);
    assert!((j_value.dpi_headroom - 0.5).abs() < 1e-12);

    let clock = Arc::new(FixedClock::new(FSV_TS));
    let mut gradient = IntelligenceGradient::new(j_value, clock.clone());
    gradient.refresh(vec![GradientCandidate {
        action: CandidateAction::LabelAnchor {
            anchor: AnchorId::new("issue428-heldout").unwrap(),
            estimated_dj: 2.0,
        },
        cost_budget_units: 1,
    }]);
    let report = intelligence_report(
        &context,
        &metrics,
        &gradient,
        &GoodhartState::default(),
        None,
        clock.as_ref(),
    );
    let text = format_report(&report);
    assert!(report.j > 0.0);
    assert!(report.dpi_headroom.is_finite());
    assert_eq!(report.provisional_excluded, 0);
    assert!(text.contains("next_best_action: LabelAnchor"));
    for label in [
        "w1_info",
        "w2_n_eff",
        "w3_sufficiency",
        "w4_kernel_recall",
        "w5_oracle_accuracy",
        "w6_mistake_rate",
        "w7_compression",
        "w8_coverage",
    ] {
        assert!(text.contains(label), "missing term label {label}");
    }
    write_intelligence_report_snapshot(vault, &report).expect("write report snapshot");

    let fixture = json!({
        "domain": "issue428",
        "panel_len": 8,
        "gradient_ts": FSV_TS,
        "gradient_candidates": [{
            "action": {
                "action": "label_anchor",
                "anchor": "issue428-heldout",
                "estimated_dj": 2.0
            },
            "cost_budget_units": 1
        }],
        "metrics": metrics
    });
    write_json(&root.join("intelligence-fixture.json"), &fixture);

    json!({
        "expected_j": 11.1,
        "actual_j": report.j,
        "expected_dpi_headroom": 0.5,
        "actual_dpi_headroom": report.dpi_headroom,
        "provisional_excluded": report.provisional_excluded,
        "human_report": text,
        "anneal_report_rows": cf_rows(vault, ColumnFamily::AnnealReport)
    })
}

fn write_rising_growth_curve(vault: &AsterVault) -> serde_json::Value {
    let clock = Arc::new(FixedClock::new(FSV_TS + 1));
    let mut curve =
        GrowthCurve::load_from_cf(AsterGrowthCf::new(vault), clock.clone(), 1_100).unwrap();
    let before_rows = cf_rows(vault, ColumnFamily::AnnealGrowth);
    for step in 0..1_000_u64 {
        let report = report_for_growth_step(step, clock.as_ref());
        curve
            .record_sample(
                &report,
                10,
                vec![
                    format!("ingest_10_synthetic_docs_step_{step}"),
                    "run_sleep_pass".to_string(),
                    "autotune_bandit_tick".to_string(),
                ],
            )
            .expect("record growth sample");
    }
    let after_rows = cf_rows(vault, ColumnFamily::AnnealGrowth);
    let summary = curve.curve_summary_with_window(100);
    assert!(curve.is_rising(100));
    assert!(summary.j_last > summary.j_first);
    assert!(curve.plot_ascii(60, 10).contains('*'));
    assert_eq!(before_rows.len(), 0);
    assert_eq!(after_rows.len(), 1_000);

    json!({
        "before_rows": before_rows.len(),
        "after_rows": after_rows.len(),
        "summary": summary,
        "plot_ascii": curve.plot_ascii(60, 10),
        "first_sample": curve.samples().next(),
        "last_sample": curve.samples().last(),
        "row_prefixes": after_rows.iter().take(3).chain(after_rows.iter().rev().take(3)).cloned().collect::<Vec<_>>()
    })
}

fn reject_gamed_change(vault_dir: &Path, vault: &AsterVault) -> serde_json::Value {
    let before = compute_j(&JObjectiveContext::new("issue428", 8), &Metrics::report()).unwrap();
    let after_train = compute_j(
        &JObjectiveContext::new("issue428", 8),
        &Metrics::gamed_train(),
    )
    .unwrap();
    let held_before =
        compute_j(&JObjectiveContext::new("issue428", 8), &Metrics::report()).unwrap();
    let held_after = compute_j(
        &JObjectiveContext::new("issue428", 8),
        &Metrics::heldout_flat(),
    )
    .unwrap();
    let train_delta = after_train.j - before.j;
    let checker = GoodhartChecker::new(
        HeldOutSet::sealed("issue428-heldout", 80, held_before, held_after),
        Arc::new(StaticWard { in_region: 0.40 }),
    );
    let report = checker
        .check(
            &before,
            &after_train,
            &[LensContributionDelta {
                lens_id: LensId::from_bytes([0x85; 16]),
                delta: train_delta * 0.90,
            }],
        )
        .expect("goodhart check");
    assert!(!report.passed);
    assert!(
        report
            .violations
            .iter()
            .any(|v| matches!(v, GoodhartViolation::HeldOutRegression { .. }))
    );
    assert!(
        report
            .violations
            .iter()
            .any(|v| matches!(v, GoodhartViolation::CrossLensAnomaly { .. }))
    );
    assert!(
        report
            .violations
            .iter()
            .any(|v| matches!(v, GoodhartViolation::GtauViolation { .. }))
    );

    let rollback = rollback_gamed_candidate(vault, &report);
    let state = add_goodhart_penalty_to_vault(vault_dir, report.p_goodhart_increment)
        .expect("persist Goodhart penalty");
    let mut ledger = open_ledger(vault);
    let ledger_ref = record_goodhart_report(
        &report,
        GoodhartLedgerContext {
            change_id: rollback.snapshot.change_id,
            artifact_id: "issue428-correlated-lens".to_string(),
            prior_ptr_hash: [0x11; 32],
            candidate_ptr_hash: [0x22; 32],
            ts: FSV_TS + 2,
        },
        &mut ledger,
    )
    .expect("record Goodhart ledger");
    vault.flush().expect("flush ledger rows");
    let ledger_entry = ledger
        .find_by_change_id(rollback.snapshot.change_id)
        .expect("read ledger")
        .expect("ledger row");
    assert_eq!(ledger_entry.action, AnnealLedgerAction::GoodhartFailed);
    assert_eq!(
        rollback.live_ptr,
        ArtifactPtr::ConfigCacheKeyHash([0x11; 32])
    );
    assert!(rollback.snapshot.promoted);
    assert!(rollback.snapshot.reverted);

    json!({
        "expected": {
            "passed": false,
            "action": "GoodhartFailed",
            "live_ptr": "prior",
            "p_goodhart_positive": true
        },
        "report": report,
        "rollback_readback": rollback,
        "goodhart_state_after": state,
        "ledger_ref": ledger_ref,
        "ledger_entry": ledger_entry,
        "ledger_rows": cf_rows(vault, ColumnFamily::Ledger),
        "rollback_rows": cf_rows(vault, ColumnFamily::AnnealRollback)
    })
}
