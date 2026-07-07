//! Issue #233 - grounded-resolution feedback controller.
//!
//! Source of truth: durable AsterVault Ledger CF rows, score artifacts/ledger rows, and the
//! meta-learning ledger, all read back from disk.

#[path = "issue233_feedback/support.rs"]
mod issue233_feedback_support;
#[path = "fsv_support.rs"]
mod support;

use std::fs;
use std::path::Path;

use calyx_poly::{
    ERR_BACKFILL_CONTRADICTION, ERR_SELF_EVOLUTION_TRIPWIRE, PendingForecastStatus,
    SelfEvolutionStatus, read_meta_learning_ledger_entries, run_feedback_controller_cycle,
};
use serde_json::{Value, json};

use issue233_feedback_support::*;
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

#[test]
fn issue233_feedback_controller_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE233_FSV_ROOT", "issue233-feedback-controller");
    reset_dir(&root);

    let happy = happy_idempotent_no_lookahead(&root);
    let no_match = no_match_noop(&root);
    let voided = voided_never_scores(&root);
    let contradiction = proxy_contradiction_fails_closed(&root);
    let rejected = rejected_update_is_reverted_and_ledged(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 233,
        "proof_claim": "The feedback controller runs grounded-resolution cycles through join, score, backfill preflight, relearning/adaptation, and meta-ledger audit; reruns are idempotent and failing branches do not silently promote.",
        "minimum_sufficient_proof_corpus": {
            "cases": 5,
            "why_this_is_sufficient": "One happy cycle also proves idempotent replay and no-lookahead blocking; one no-match proves no-op logging; one voided case proves no scoring/backfill; one proxy contradiction proves fail-closed backfill; one rejected candidate proves rollback/guardrail/meta-ledger behavior.",
            "why_larger_is_wasteful": "The controller proof is over orchestration states, not corpus volume; extra markets would repeat the same branches."
        },
        "source_of_truth": "durable AsterVault Ledger CF rows, score artifacts, score ledger rows, and meta-learning ledger JSONL",
        "cases": {
            "happy_idempotent_no_lookahead": happy,
            "no_match": no_match,
            "voided": voided,
            "proxy_contradiction": contradiction,
            "rejected_update": rejected
        },
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue233_feedback_controller_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback, report);
    write_blake3sums(&root);
}

fn happy_idempotent_no_lookahead(root: &Path) -> Value {
    let mut fx = fixture(root, "happy");
    let reg_seq = record(
        &fx.vault,
        &mut fx.register,
        forecast("f233happy", "cond233happy", 0, 100),
    );
    record(
        &fx.vault,
        &mut fx.register,
        forecast("f233future", "cond233happy", 0, 250),
    );
    let paths = learning_paths(&fx.root, "approved");
    let request = cycle_request(
        &fx.report_dir,
        &fx.score_root,
        "cycle233happy",
        vec![input("cond233happy", 0, 200, false, false)],
        vec![score("score233happy", "f233happy", "cond233happy", true)],
        vec![backfill("happy-backfill", true, true)],
        Some(learning(&paths, false)),
    );
    let first =
        run_feedback_controller_cycle(&request, &fx.vault, &mut fx.register, &mut fx.score_ledger)
            .expect("happy cycle");
    let second =
        run_feedback_controller_cycle(&request, &fx.vault, &mut fx.register, &mut fx.score_ledger)
            .expect("happy replay");
    assert_eq!(first.report.score_manifests.len(), 1);
    assert_eq!(
        second.report.skipped_existing_score_ids,
        vec!["score233happy"]
    );
    assert_eq!(score_payloads(&fx.score_ledger_dir).len(), 1);
    assert_eq!(
        read_meta_learning_ledger_entries(&paths.meta_dir.join("meta_learning_ledger.jsonl"))
            .expect("read meta ledger")
            .len(),
        1
    );
    assert_eq!(
        second.report.join_results[0].lookahead_blocked_forecast_ids,
        vec!["f233future"]
    );
    persist_case(
        &fx.root,
        json!({
            "registered_ledger": vault_payload(&fx.vault, reg_seq),
            "first": first.report,
            "second": second.report,
            "score_ledger": score_payloads(&fx.score_ledger_dir),
            "meta_ledger": read_meta_learning_ledger_entries(&paths.meta_dir.join("meta_learning_ledger.jsonl")).expect("meta readback"),
            "future_status": fx.register.entries.iter().find(|entry| entry.forecast_id == "f233future").unwrap().status
        }),
    )
}

fn no_match_noop(root: &Path) -> Value {
    let mut fx = fixture(root, "no-match");
    record(
        &fx.vault,
        &mut fx.register,
        forecast("f233never", "cond233never", 0, 100),
    );
    let paths = learning_paths(&fx.root, "unused");
    let request = cycle_request(
        &fx.report_dir,
        &fx.score_root,
        "cycle233nomatch",
        vec![input("cond233absent", 0, 200, false, false)],
        Vec::new(),
        Vec::new(),
        Some(learning(&paths, false)),
    );
    let run =
        run_feedback_controller_cycle(&request, &fx.vault, &mut fx.register, &mut fx.score_ledger)
            .expect("no-match cycle");
    assert!(run.report.join_results[0].selected_forecast_ids.is_empty());
    assert_eq!(run.report.join_results[0].pending_after, 1);
    assert!(run.report.score_manifests.is_empty());
    assert!(run.report.learning.is_none());
    assert!(!paths.meta_dir.join("meta_learning_ledger.jsonl").exists());
    persist_case(
        &fx.root,
        json!({
            "report": run.report,
            "score_ledger": score_payloads(&fx.score_ledger_dir),
            "meta_ledger_exists": paths.meta_dir.join("meta_learning_ledger.jsonl").exists(),
            "pending_status": fx.register.entries[0].status
        }),
    )
}

fn voided_never_scores(root: &Path) -> Value {
    let mut fx = fixture(root, "voided");
    record(
        &fx.vault,
        &mut fx.register,
        forecast("f233void", "cond233void", 0, 100),
    );
    let request = cycle_request(
        &fx.report_dir,
        &fx.score_root,
        "cycle233void",
        vec![input("cond233void", 0, 200, true, false)],
        Vec::new(),
        Vec::new(),
        None,
    );
    let run =
        run_feedback_controller_cycle(&request, &fx.vault, &mut fx.register, &mut fx.score_ledger)
            .expect("voided cycle");
    assert_eq!(fx.register.entries[0].status, PendingForecastStatus::Void);
    assert!(run.report.score_manifests.is_empty());
    assert!(run.report.backfills.is_empty());
    persist_case(
        &fx.root,
        json!({
            "report": run.report,
            "score_ledger": score_payloads(&fx.score_ledger_dir),
            "terminal_status": fx.register.entries[0].status
        }),
    )
}

fn proxy_contradiction_fails_closed(root: &Path) -> Value {
    let mut fx = fixture(root, "proxy-contradiction");
    record(
        &fx.vault,
        &mut fx.register,
        forecast("f233bad", "cond233bad", 0, 100),
    );
    let request = cycle_request(
        &fx.report_dir,
        &fx.score_root,
        "cycle233bad",
        vec![input("cond233bad", 0, 200, false, false)],
        vec![score("score233bad", "f233bad", "cond233bad", true)],
        vec![backfill("bad-backfill", true, false)],
        None,
    );
    let err =
        run_feedback_controller_cycle(&request, &fx.vault, &mut fx.register, &mut fx.score_ledger)
            .expect_err("contradiction must fail closed");
    assert_eq!(err.code(), ERR_BACKFILL_CONTRADICTION);
    assert!(score_payloads(&fx.score_ledger_dir).is_empty());
    persist_case(
        &fx.root,
        json!({
            "error_code": err.code(),
            "error_message": err.message(),
            "score_ledger": score_payloads(&fx.score_ledger_dir),
            "score_artifact_exists": fx.score_root.join("score233bad").exists()
        }),
    )
}

fn rejected_update_is_reverted_and_ledged(root: &Path) -> Value {
    let mut fx = fixture(root, "rejected-update");
    record(
        &fx.vault,
        &mut fx.register,
        forecast("f233reject", "cond233reject", 0, 100),
    );
    let paths = learning_paths(&fx.root, "rejected");
    let request = cycle_request(
        &fx.report_dir,
        &fx.score_root,
        "cycle233reject",
        vec![input("cond233reject", 0, 200, false, false)],
        vec![score("score233reject", "f233reject", "cond233reject", true)],
        vec![backfill("reject-backfill", true, true)],
        Some(learning(&paths, true)),
    );
    let run =
        run_feedback_controller_cycle(&request, &fx.vault, &mut fx.register, &mut fx.score_ledger)
            .expect("rejected cycle");
    let learning = run.report.learning.as_ref().expect("learning result");
    assert!(!learning.promoted);
    assert_eq!(
        learning.rejection_code.as_deref(),
        Some(ERR_SELF_EVOLUTION_TRIPWIRE)
    );
    assert_eq!(
        learning.guardrail_report.status,
        SelfEvolutionStatus::Rejected
    );
    assert!(learning.meta_learning_appended);
    persist_case(
        &fx.root,
        json!({
            "report": run.report,
            "score_ledger": score_payloads(&fx.score_ledger_dir),
            "meta_ledger": read_meta_learning_ledger_entries(&paths.meta_dir.join("meta_learning_ledger.jsonl")).expect("meta readback")
        }),
    )
}
