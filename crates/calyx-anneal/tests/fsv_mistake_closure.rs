use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use calyx_anneal::{
    AsterHeadStorage, ComponentHealth, ComponentKind, DEFAULT_SLEEP_PASS_MIN_SURPRISE, HeadKind,
    OnlineHead, OnlineHeadState, SleepPassConfig, SleepPassOutcome, assert_no_regression,
    record_mistake_for_replay, regression_rate, run_sleep_pass,
};
use calyx_aster::cf::ColumnFamily;
use calyx_core::{AnchorKind, FixedClock, Result, SlotId, VaultStore};
use serde_json::{Value, json};

#[allow(dead_code)]
#[path = "support/fsv_bad_change.rs"]
mod support;

#[path = "support/fsv_mistake_closure_support.rs"]
mod closure;

use closure::{
    TEST_TS, anneal_ledger, cf_hashes, cf_rows, cf_snapshot, cx, decoded_heads, frozen_guard,
    has_ledger_action, health_registry, ledger_rows, mistake_log, replay_buffer, write_json,
};

#[test]
#[ignore = "requires CALYX_ISSUE411_FSV_ROOT in a manual verification run"]
fn fsv_mistake_closure_manual() -> Result<()> {
    let root = PathBuf::from(env::var("CALYX_ISSUE411_FSV_ROOT").unwrap());
    support::reset_dir(&root);

    let happy = mistake_closure_loop(&root)?;
    let load = no_frozen_mutation_under_load(&root)?;
    let zero = zero_surprise_edge(&root)?;
    let deferred = degraded_defers_edge(&root)?;
    let reverted = reverted_update_edge(&root)?;

    support::write_json(
        &root.join("issue411-fsv-artifact.json"),
        &json!({
            "issue": 411,
            "surface": "PH45 T06 mistake-closure integration FSV",
            "source_of_truth": [
                "Aster base CF constellation rows",
                "Aster anneal_mistakes CF rows",
                "Aster anneal_replay CF snapshot row",
                "Aster anneal_heads CF rows",
                "Aster anneal_health CF rows",
                "Aster ledger CF rows",
                "FrozenLensGuard report JSON artifacts"
            ],
            "trigger_to_outcome": {
                "trigger": "record_mistake_for_replay followed by run_sleep_pass",
                "expected": "mistakes enter replay above threshold, sleep pass promotes non-recurrent head updates, degraded components defer, budget reverts keep replay intact, frozen lens hashes stay stable"
            },
            "happy": happy,
            "load": load,
            "edges": {
                "zero_surprise": zero,
                "degraded_deferred": deferred,
                "reverted_update": reverted
            }
        }),
    );
    Ok(())
}

fn mistake_closure_loop(root: &Path) -> Result<Value> {
    let (vault_dir, vault) = support::open_durable_vault(root, "closure-vault");
    let cx = cx(1);
    vault.put(cx.clone())?;
    let guard = frozen_guard("issue411-happy-byte")?;
    let frozen_before = guard.check()?;
    write_json(root, "happy-frozen-before.json", &frozen_before);

    let log = mistake_log(&vault);
    let mut buffer = replay_buffer(&vault, 16);
    let replay_record = record_mistake_for_replay(
        &log,
        &mut buffer,
        cx.cx_id,
        0.9,
        0.1,
        AnchorKind::Reward,
        DEFAULT_SLEEP_PASS_MIN_SURPRISE,
    )?;
    let batch_before = buffer.entries_by_priority();
    let before_report = {
        let clock = FixedClock::new(TEST_TS);
        let state = OnlineHeadState::open_with_guard(
            AsterHeadStorage::new(&vault),
            support::durable_substrate(&clock, &vault, &vault_dir),
            Arc::new(FixedClock::new(TEST_TS)),
            [OnlineHead::new(HeadKind::Predictor, vec![1.0])?],
            guard.clone(),
        )?;
        assert_no_regression(&state, &batch_before, &log, &vault)?
    };

    let clock = FixedClock::new(TEST_TS + 1);
    let registry = health_registry(&vault);
    let mut state = OnlineHeadState::open_with_guard(
        AsterHeadStorage::new(&vault),
        support::durable_substrate(&clock, &vault, &vault_dir),
        Arc::new(FixedClock::new(TEST_TS + 1)),
        [OnlineHead::new(HeadKind::Predictor, vec![1.0])?],
        guard.clone(),
    )?;
    let outcome = run_sleep_pass(
        &mut state,
        &buffer,
        &log,
        &vault,
        &registry,
        SleepPassConfig {
            batch_size: 1,
            seed: 0xCAFE,
            ..SleepPassConfig::default()
        },
    )?;
    let after_report = assert_no_regression(&state, &batch_before, &log, &vault)?;
    let frozen_after = guard.check()?;
    write_json(root, "happy-frozen-after.json", &frozen_after);
    vault.flush()?;

    assert!(replay_record.replay_added);
    assert_eq!(batch_before[0].cx_id, cx.cx_id);
    assert!(!before_report.passed);
    assert!(matches!(outcome, SleepPassOutcome::Promoted { .. }));
    assert!(after_report.passed);
    assert_eq!(regression_rate(&after_report)?, 0.0);
    assert_eq!(frozen_before.rows, frozen_after.rows);
    assert!(has_ledger_action(
        &ledger_rows(&vault),
        "head_update",
        "regression_rate=0.000000"
    ));
    assert_eq!(decoded_heads(&vault)[0]["head"]["version"], 1);

    Ok(json!({
        "vault": vault_dir.display().to_string(),
        "known_input": {"predicted": 0.9, "observed": 0.1, "old_surprise": 0.8},
        "hand_expected": {"updated_prediction": 0.8, "new_surprise": 0.7, "regression_rate": 0.0},
        "stored_context": vault.get(cx.cx_id, vault.snapshot())?.cx_id,
        "replay_record": replay_record,
        "before_report": before_report,
        "sleep_pass_outcome": outcome,
        "after_report": after_report,
        "frozen_before": frozen_before,
        "frozen_after": frozen_after,
        "cf_hashes": cf_hashes(&vault),
        "mistake_rows": cf_rows(&vault, ColumnFamily::AnnealMistakes),
        "replay_rows": cf_rows(&vault, ColumnFamily::AnnealReplay),
        "head_rows": decoded_heads(&vault),
        "ledger_rows": ledger_rows(&vault)
    }))
}

fn no_frozen_mutation_under_load(root: &Path) -> Result<Value> {
    let (vault_dir, vault) = support::open_durable_vault(root, "load-vault");
    let guard = frozen_guard("issue411-load-byte")?;
    let frozen_before = guard.check()?;
    let log = mistake_log(&vault);
    let mut buffer = replay_buffer(&vault, 128);
    for index in 0..100_u8 {
        let cx = cx(index.wrapping_add(10));
        vault.put(cx.clone())?;
        let observed = 0.1 + f64::from(index % 3) * 0.1;
        record_mistake_for_replay(
            &log,
            &mut buffer,
            cx.cx_id,
            0.9,
            observed,
            AnchorKind::Reward,
            DEFAULT_SLEEP_PASS_MIN_SURPRISE,
        )?;
    }
    let clock = FixedClock::new(TEST_TS + 2);
    let registry = health_registry(&vault);
    let mut state = OnlineHeadState::open_with_guard(
        AsterHeadStorage::new(&vault),
        support::durable_substrate(&clock, &vault, &vault_dir),
        Arc::new(FixedClock::new(TEST_TS + 2)),
        [OnlineHead::new(HeadKind::Predictor, vec![0.9])?],
        guard.clone(),
    )?;
    let outcome = run_sleep_pass(
        &mut state,
        &buffer,
        &log,
        &vault,
        &registry,
        SleepPassConfig {
            batch_size: 16,
            seed: 0xCAFE,
            lr: 0.25,
            ..SleepPassConfig::default()
        },
    )?;
    let frozen_after = guard.check()?;
    vault.flush()?;

    assert!(matches!(outcome, SleepPassOutcome::Promoted { .. }));
    assert_eq!(buffer.len(), 100);
    assert_eq!(frozen_before.rows, frozen_after.rows);
    assert!(frozen_after.violations.is_empty());

    Ok(json!({
        "vault": vault_dir.display().to_string(),
        "cycles": 100,
        "buffer_len": buffer.len(),
        "sleep_pass_outcome": outcome,
        "frozen_before": frozen_before,
        "frozen_after": frozen_after,
        "head_rows": decoded_heads(&vault),
        "ledger_rows": ledger_rows(&vault)
    }))
}

fn zero_surprise_edge(root: &Path) -> Result<Value> {
    let (vault_dir, vault) = support::open_durable_vault(root, "zero-vault");
    let cx = cx(2);
    vault.put(cx.clone())?;
    let log = mistake_log(&vault);
    let mut buffer = replay_buffer(&vault, 16);
    let before = cf_snapshot(&vault);
    let replay_record = record_mistake_for_replay(
        &log,
        &mut buffer,
        cx.cx_id,
        0.5,
        0.5,
        AnchorKind::Reward,
        DEFAULT_SLEEP_PASS_MIN_SURPRISE,
    )?;
    let clock = FixedClock::new(TEST_TS + 3);
    let registry = health_registry(&vault);
    let mut state = OnlineHeadState::open(
        AsterHeadStorage::new(&vault),
        support::durable_substrate(&clock, &vault, &vault_dir),
        Arc::new(FixedClock::new(TEST_TS + 3)),
        [OnlineHead::new(HeadKind::Predictor, vec![0.5])?],
    )?;
    let outcome = run_sleep_pass(
        &mut state,
        &buffer,
        &log,
        &vault,
        &registry,
        SleepPassConfig::default(),
    )?;
    vault.flush()?;

    assert!(!replay_record.replay_added);
    assert_eq!(buffer.len(), 0);
    assert!(matches!(outcome, SleepPassOutcome::Idle { .. }));
    assert!(cf_rows(&vault, ColumnFamily::AnnealHeads).is_empty());

    Ok(json!({
        "vault": vault_dir.display().to_string(),
        "before": before,
        "after": cf_snapshot(&vault),
        "replay_record": replay_record,
        "sleep_pass_outcome": outcome
    }))
}

fn degraded_defers_edge(root: &Path) -> Result<Value> {
    let (vault_dir, vault) = support::open_durable_vault(root, "deferred-vault");
    let cx = cx(3);
    vault.put(cx.clone())?;
    let log = mistake_log(&vault);
    let mut buffer = replay_buffer(&vault, 16);
    record_mistake_for_replay(
        &log,
        &mut buffer,
        cx.cx_id,
        0.9,
        0.1,
        AnchorKind::Reward,
        DEFAULT_SLEEP_PASS_MIN_SURPRISE,
    )?;
    let mut registry = health_registry(&vault);
    let clock = FixedClock::new(TEST_TS + 4);
    let mut ledger = anneal_ledger(&vault, clock)?;
    registry.set_health(
        ComponentKind::ann_index(SlotId::new(0)),
        ComponentHealth::degraded(TEST_TS + 4, "synthetic PH45 deferred edge"),
        &mut ledger,
    )?;
    drop(ledger);
    let before = cf_snapshot(&vault);

    let mut state = OnlineHeadState::open(
        AsterHeadStorage::new(&vault),
        support::durable_substrate(&clock, &vault, &vault_dir),
        Arc::new(FixedClock::new(TEST_TS + 4)),
        [OnlineHead::new(HeadKind::Predictor, vec![0.9])?],
    )?;
    let outcome = run_sleep_pass(
        &mut state,
        &buffer,
        &log,
        &vault,
        &registry,
        SleepPassConfig::default(),
    )?;
    vault.flush()?;
    let ledger_rows = ledger_rows(&vault);

    assert!(matches!(outcome, SleepPassOutcome::Deferred { .. }));
    assert!(cf_rows(&vault, ColumnFamily::AnnealHeads).is_empty());
    assert!(has_ledger_action(
        &ledger_rows,
        "sleep_pass_deferred",
        "degraded_count=1"
    ));

    Ok(json!({
        "vault": vault_dir.display().to_string(),
        "before": before,
        "after": cf_snapshot(&vault),
        "degraded_components": registry.degraded_components(),
        "sleep_pass_outcome": outcome,
        "ledger_rows": ledger_rows
    }))
}

fn reverted_update_edge(root: &Path) -> Result<Value> {
    let (vault_dir, vault) = support::open_durable_vault(root, "reverted-vault");
    let cx = cx(4);
    vault.put(cx.clone())?;
    let log = mistake_log(&vault);
    let mut buffer = replay_buffer(&vault, 16);
    record_mistake_for_replay(
        &log,
        &mut buffer,
        cx.cx_id,
        0.9,
        0.1,
        AnchorKind::Reward,
        DEFAULT_SLEEP_PASS_MIN_SURPRISE,
    )?;
    let before_mistakes = cf_rows(&vault, ColumnFamily::AnnealMistakes);
    let before_replay = buffer.snapshot();
    let clock = FixedClock::new(TEST_TS + 5);
    let registry = health_registry(&vault);
    let mut state = OnlineHeadState::open(
        AsterHeadStorage::new(&vault),
        support::durable_substrate_with_budget(
            &clock,
            &vault,
            &vault_dir,
            support::budget_config(0.0),
        ),
        Arc::new(FixedClock::new(TEST_TS + 5)),
        [OnlineHead::new(HeadKind::Predictor, vec![0.9])?],
    )?;
    let outcome = run_sleep_pass(
        &mut state,
        &buffer,
        &log,
        &vault,
        &registry,
        SleepPassConfig::default(),
    )?;
    vault.flush()?;
    let ledger_rows = ledger_rows(&vault);

    assert!(matches!(outcome, SleepPassOutcome::Reverted { .. }));
    assert_eq!(
        before_mistakes,
        cf_rows(&vault, ColumnFamily::AnnealMistakes)
    );
    assert_eq!(before_replay, buffer.snapshot());
    assert!(cf_rows(&vault, ColumnFamily::AnnealHeads).is_empty());
    assert!(has_ledger_action(&ledger_rows, "head_update_reverted", ""));

    Ok(json!({
        "vault": vault_dir.display().to_string(),
        "before_mistake_rows": before_mistakes,
        "after_mistake_rows": cf_rows(&vault, ColumnFamily::AnnealMistakes),
        "before_replay": before_replay,
        "after_replay": buffer.snapshot(),
        "sleep_pass_outcome": outcome,
        "head_rows": decoded_heads(&vault),
        "ledger_rows": ledger_rows
    }))
}
