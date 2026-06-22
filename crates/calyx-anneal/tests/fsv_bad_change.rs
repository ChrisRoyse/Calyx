use std::env;
use std::path::{Path, PathBuf};

use calyx_anneal::{
    AnnealLedgerAction, CALYX_LEDGER_WRITE_FAIL, ChangeId, ChangeOutcome, ShadowRevertReason,
    TripwireMetric,
};
use calyx_aster::cf::ColumnFamily;
use calyx_core::FixedClock;
use calyx_ledger::MemoryLedgerStore;
use serde_json::json;

#[path = "support/fsv_bad_change.rs"]
mod support;
use support::*;

#[test]
fn bad_recall_reverts_and_leaves_live_pointer_unchanged() {
    let clock = FixedClock::new(TEST_TS);
    let mut substrate = memory_substrate(&clock, budget_config(1.0), MemoryLedgerStore::default());
    install_prior(&substrate.rollback);

    let outcome = substrate
        .propose_change(
            artifact_key(),
            candidate_ptr(),
            &action(0.70),
            &action(0.95),
        )
        .unwrap();

    assert!(matches!(
        outcome,
        ChangeOutcome::Reverted {
            reason: ShadowRevertReason::TripwireCrossed(TripwireMetric::RecallAtK),
            ..
        }
    ));
    assert_eq!(
        substrate.rollback.live_ptr(&artifact_key()).unwrap(),
        Some(prior_ptr())
    );
    let recent = substrate.ledger.read_recent(1).unwrap();
    assert_eq!(recent[0].action, AnnealLedgerAction::Revert);
}

#[test]
fn good_candidate_promotes_and_updates_live_pointer() {
    let clock = FixedClock::new(TEST_TS);
    let mut substrate = memory_substrate(&clock, budget_config(1.0), MemoryLedgerStore::default());
    install_prior(&substrate.rollback);

    let outcome = substrate
        .propose_change(
            artifact_key(),
            candidate_ptr(),
            &action(0.96),
            &action(0.91),
        )
        .unwrap();

    let ChangeOutcome::Promoted(change_id) = outcome else {
        panic!("expected promotion");
    };
    assert_eq!(
        substrate.rollback.live_ptr(&artifact_key()).unwrap(),
        Some(candidate_ptr())
    );
    let recent = substrate.ledger.read_recent(1).unwrap();
    assert_eq!(recent[0].action, AnnealLedgerAction::Promote);
    assert_eq!(recent[0].change_id, change_id);
}

#[test]
fn explicit_rollback_restores_prior_and_writes_second_revert() {
    let clock = FixedClock::new(TEST_TS);
    let mut substrate = memory_substrate(&clock, budget_config(1.0), MemoryLedgerStore::default());
    install_prior(&substrate.rollback);
    let ChangeOutcome::Promoted(change_id) = substrate
        .propose_change(
            artifact_key(),
            candidate_ptr(),
            &action(0.96),
            &action(0.91),
        )
        .unwrap()
    else {
        panic!("expected promotion");
    };

    substrate.rollback_explicit(change_id).unwrap();

    assert_eq!(
        substrate.rollback.live_ptr(&artifact_key()).unwrap(),
        Some(prior_ptr())
    );
    let recent = substrate.ledger.read_recent(2).unwrap();
    assert_eq!(recent[0].action, AnnealLedgerAction::Promote);
    assert_eq!(recent[1].action, AnnealLedgerAction::Revert);
    assert_eq!(recent[1].change_id, change_id);
}

#[test]
fn budget_exhaustion_reverts_without_promotion_entry() {
    let clock = FixedClock::new(TEST_TS);
    let mut substrate = memory_substrate(&clock, budget_config(0.0), MemoryLedgerStore::default());
    install_prior(&substrate.rollback);

    let outcome = substrate
        .propose_change(
            artifact_key(),
            candidate_ptr(),
            &action(0.96),
            &action(0.91),
        )
        .unwrap();

    assert_eq!(
        outcome,
        ChangeOutcome::Reverted {
            reason: ShadowRevertReason::BudgetExhausted,
            change_id: ChangeId(TEST_TS * 1_000_000 + 8)
        }
    );
    assert_eq!(
        substrate.rollback.live_ptr(&artifact_key()).unwrap(),
        Some(prior_ptr())
    );
    let recent = substrate.ledger.read_recent(1).unwrap();
    assert_eq!(recent[0].action, AnnealLedgerAction::Revert);
}

#[test]
fn ledger_write_failure_prevents_promotion() {
    let clock = FixedClock::new(TEST_TS);
    let mut substrate = memory_substrate(&clock, budget_config(1.0), FailingLedgerStore);
    install_prior(&substrate.rollback);

    let error = substrate
        .propose_change(
            artifact_key(),
            candidate_ptr(),
            &action(0.96),
            &action(0.91),
        )
        .unwrap_err();

    assert_eq!(error.code, CALYX_LEDGER_WRITE_FAIL);
    assert_eq!(
        substrate.rollback.live_ptr(&artifact_key()).unwrap(),
        Some(prior_ptr())
    );
}

#[test]
fn status_reports_budget_tripwires_and_recent_changes() {
    let clock = FixedClock::new(TEST_TS);
    let mut substrate = memory_substrate(&clock, budget_config(1.0), MemoryLedgerStore::default());
    install_prior(&substrate.rollback);
    substrate
        .propose_change(
            artifact_key(),
            candidate_ptr(),
            &action(0.96),
            &action(0.91),
        )
        .unwrap();

    let status = substrate.status().unwrap();

    assert_eq!(status.tripwire_states.len(), 5);
    assert_eq!(status.budget.handles_active, 0);
    assert_eq!(status.recent_changes.len(), 1);
}

#[test]
#[ignore = "requires CALYX_ISSUE399_FSV_ROOT in a manual verification run"]
fn fsv_bad_change_manual() {
    let root =
        PathBuf::from(env::var("CALYX_ISSUE399_FSV_ROOT").expect("set CALYX_ISSUE399_FSV_ROOT"));
    reset_dir(&root);
    let (vault_dir, vault) = open_durable_vault(&root, "vault");
    let clock = FixedClock::new(TEST_TS);
    let mut substrate = durable_substrate(&clock, &vault, &vault_dir);
    install_prior(&substrate.rollback);
    let before_ledger = read_ledger_rows(&vault);
    let before_rollback = substrate
        .rollback
        .readback(ChangeId(0))
        .err()
        .map(|error| error.code.to_string());

    let outcome = substrate
        .propose_change_with_description(
            artifact_key(),
            candidate_ptr(),
            &action(0.70),
            &action(0.95),
            "synthetic bad recall fsv",
        )
        .expect("bad change proposal");
    let ChangeOutcome::Reverted { change_id, reason } = outcome else {
        panic!("expected revert");
    };
    vault.flush().expect("flush durable vault");
    let rollback_readback = substrate.rollback.readback(change_id).unwrap();
    let after_ledger = read_ledger_rows(&vault);
    let after_rollback_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealRollback)
        .expect("scan rollback CF");

    assert!(before_ledger.is_empty());
    assert_eq!(after_ledger.len(), 1);
    assert_eq!(after_ledger[0]["payload_json"]["action"], "revert");
    assert_eq!(
        substrate.rollback.live_ptr(&artifact_key()).unwrap(),
        Some(prior_ptr())
    );
    assert_eq!(rollback_readback.live_ptr, prior_ptr());
    assert!(rollback_readback.snapshot.reverted);
    assert!(!rollback_readback.snapshot.promoted);

    write_json(
        &root.join("bad-change-readback.json"),
        &json!({
            "surface": "anneal.substrate.bad_change",
            "source_of_truth": "Aster ledger CF rows plus anneal_rollback CF live pointer",
            "vault": vault_dir.display().to_string(),
            "trigger": "bad recall candidate recall=0.70 below recall_at_k tripwire 0.90",
            "before": {
                "ledger_rows": before_ledger,
                "unknown_change_probe": before_rollback
            },
            "outcome": {
                "change_id": change_id.0,
                "reason": reason,
            },
            "after": {
                "ledger_rows": after_ledger,
                "rollback_snapshot": rollback_readback.snapshot,
                "rollback_live_ptr": rollback_readback.live_ptr,
                "rollback_row_count": after_rollback_rows.len()
            },
            "expected": {
                "ledger_action": "revert",
                "live_ptr": prior_ptr(),
                "candidate_ptr": candidate_ptr(),
                "promoted": false,
                "reverted": true
            }
        }),
    );

    write_json(
        &root.join("edge-readback.json"),
        &json!({
            "surface": "anneal.substrate.integration_edges",
            "source_of_truth": [
                "Aster ledger CF rows",
                "Aster anneal_rollback CF rows",
                "RollbackStore readback over injected failing ledger store"
            ],
            "edges": [
                good_promote_and_explicit_rollback_edge(&root, &clock),
                budget_exhaustion_edge(&root, &clock),
                ledger_write_failure_edge(&clock)
            ]
        }),
    );
}

fn good_promote_and_explicit_rollback_edge(root: &Path, clock: &FixedClock) -> serde_json::Value {
    let (vault_dir, vault) = open_durable_vault(root, "good-vault");
    let mut substrate = durable_substrate(clock, &vault, &vault_dir);
    install_prior(&substrate.rollback);
    let before = json!({
        "ledger_rows": read_ledger_rows(&vault),
        "rollback_rows": read_rollback_rows(&vault),
        "live_ptr": substrate.rollback.live_ptr(&artifact_key()).unwrap()
    });

    let ChangeOutcome::Promoted(change_id) = substrate
        .propose_change_with_description(
            artifact_key(),
            candidate_ptr(),
            &action(0.96),
            &action(0.91),
            "synthetic good candidate fsv",
        )
        .expect("good proposal")
    else {
        panic!("expected promotion");
    };
    vault.flush().expect("flush good promote vault");
    let promote_readback = substrate.rollback.readback(change_id).unwrap();
    let after_promote_ledger = read_ledger_rows(&vault);
    let after_promote = json!({
        "ledger_rows": after_promote_ledger,
        "rollback_rows": read_rollback_rows(&vault),
        "rollback_snapshot": promote_readback.snapshot,
        "rollback_live_ptr": promote_readback.live_ptr,
        "live_ptr": substrate.rollback.live_ptr(&artifact_key()).unwrap()
    });

    substrate
        .rollback_explicit(change_id)
        .expect("explicit rollback");
    vault.flush().expect("flush explicit rollback vault");
    let rollback_readback = substrate.rollback.readback(change_id).unwrap();
    let after_rollback_ledger = read_ledger_rows(&vault);
    let after_explicit_rollback = json!({
        "ledger_rows": after_rollback_ledger,
        "rollback_rows": read_rollback_rows(&vault),
        "rollback_snapshot": rollback_readback.snapshot,
        "rollback_live_ptr": rollback_readback.live_ptr,
        "live_ptr": substrate.rollback.live_ptr(&artifact_key()).unwrap()
    });

    assert_eq!(
        after_promote["ledger_rows"][0]["payload_json"]["action"],
        "promote"
    );
    assert_eq!(
        substrate.rollback.live_ptr(&artifact_key()).unwrap(),
        Some(prior_ptr())
    );
    assert_eq!(
        after_explicit_rollback["ledger_rows"][1]["payload_json"]["action"],
        "revert"
    );

    json!({
        "edge": "good candidate promotes, then explicit rollback restores prior",
        "vault": vault_dir.display().to_string(),
        "before": before,
        "after_promote": after_promote,
        "after_explicit_rollback": after_explicit_rollback,
        "expected": {
            "first_ledger_action": "promote",
            "second_ledger_action": "revert",
            "live_after_promote": candidate_ptr(),
            "live_after_rollback": prior_ptr()
        }
    })
}

fn budget_exhaustion_edge(root: &Path, clock: &FixedClock) -> serde_json::Value {
    let (vault_dir, vault) = open_durable_vault(root, "budget-vault");
    let mut substrate =
        durable_substrate_with_budget(clock, &vault, &vault_dir, budget_config(0.0));
    install_prior(&substrate.rollback);
    let before = json!({
        "ledger_rows": read_ledger_rows(&vault),
        "rollback_rows": read_rollback_rows(&vault),
        "live_ptr": substrate.rollback.live_ptr(&artifact_key()).unwrap()
    });

    let outcome = substrate
        .propose_change_with_description(
            artifact_key(),
            candidate_ptr(),
            &action(0.96),
            &action(0.91),
            "synthetic budget exhausted fsv",
        )
        .expect("budget proposal");
    let ChangeOutcome::Reverted { change_id, reason } = outcome else {
        panic!("expected budget revert");
    };
    assert_eq!(reason, ShadowRevertReason::BudgetExhausted);
    vault.flush().expect("flush budget vault");
    let rollback_readback = substrate.rollback.readback(change_id).unwrap();
    let after_ledger = read_ledger_rows(&vault);
    assert_eq!(after_ledger.len(), 1);
    assert_eq!(after_ledger[0]["payload_json"]["action"], "revert");
    assert_eq!(
        substrate.rollback.live_ptr(&artifact_key()).unwrap(),
        Some(prior_ptr())
    );

    json!({
        "edge": "budget exhaustion reverts without a promote ledger entry",
        "vault": vault_dir.display().to_string(),
        "before": before,
        "outcome": {
            "change_id": change_id.0,
            "reason": reason
        },
        "after": {
            "ledger_rows": after_ledger,
            "rollback_rows": read_rollback_rows(&vault),
            "rollback_snapshot": rollback_readback.snapshot,
            "rollback_live_ptr": rollback_readback.live_ptr,
            "live_ptr": substrate.rollback.live_ptr(&artifact_key()).unwrap()
        },
        "expected": {
            "ledger_action": "revert",
            "no_promote_row": true,
            "live_ptr": prior_ptr()
        }
    })
}

fn ledger_write_failure_edge(clock: &FixedClock) -> serde_json::Value {
    let mut substrate = memory_substrate(clock, budget_config(1.0), FailingLedgerStore);
    install_prior(&substrate.rollback);
    let before_live = substrate.rollback.live_ptr(&artifact_key()).unwrap();
    let error = substrate
        .propose_change_with_description(
            artifact_key(),
            candidate_ptr(),
            &action(0.96),
            &action(0.91),
            "synthetic ledger failure fsv",
        )
        .unwrap_err();
    let change_id = ChangeId(TEST_TS * 1_000_000 + 8);
    let readback = substrate.rollback.readback(change_id).unwrap();
    let after_live = substrate.rollback.live_ptr(&artifact_key()).unwrap();

    assert_eq!(error.code, CALYX_LEDGER_WRITE_FAIL);
    assert_eq!(before_live, Some(prior_ptr()));
    assert_eq!(after_live, Some(prior_ptr()));
    assert!(!readback.snapshot.promoted);
    assert!(!readback.snapshot.reverted);

    json!({
        "edge": "ledger write failure occurs before live pointer promotion",
        "source_of_truth": "MemoryRollbackStorage row readback after injected LedgerCfStore failure",
        "before": {
            "live_ptr": before_live
        },
        "outcome": {
            "error_code": error.code,
            "change_id": change_id.0
        },
        "after": {
            "rollback_snapshot": readback.snapshot,
            "rollback_live_ptr": readback.live_ptr,
            "live_ptr": after_live
        },
        "expected": {
            "error_code": CALYX_LEDGER_WRITE_FAIL,
            "promoted": false,
            "reverted": false,
            "live_ptr": prior_ptr()
        }
    })
}
