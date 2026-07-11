use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use calyx_anneal::{
    AsterHeadStorage, AsterOutcomeStorage, GoodhartChecker, HeadKind, HeldOutSet, JTerms, JValue,
    JWeights, LensContributionDelta, OnlineHead, OnlineHeadState, OutcomePrediction, OutcomeQueue,
    RecordOutcomeConfig, RecordOutcomeContext, RecordOutcomeResult, RegressionConfig,
    SleepPassConfig, SleepPassOutcome, WardGtau, assert_no_regression, record_mistake_for_replay,
    record_outcome, regression_rate, run_sleep_pass,
};
use calyx_aster::cf::ColumnFamily;
use calyx_core::{Anchor, AnchorKind, AnchorValue, FixedClock, LensId, Result, VaultStore};
use serde_json::{Value, json};

#[allow(dead_code)]
#[path = "support/fsv_bad_change.rs"]
mod support;

#[allow(dead_code)]
#[path = "support/fsv_mistake_closure_support.rs"]
mod closure;

const ISSUE: u64 = 1246;

#[test]
#[ignore = "requires CALYX_ISSUE1246_FSV_ROOT in a manual verification run"]
fn issue1246_online_learning_loop_manual_fsv() -> Result<()> {
    let root = PathBuf::from(env::var("CALYX_ISSUE1246_FSV_ROOT").unwrap());
    support::reset_dir(&root);

    let outcome = outcome_ingress(&root)?;
    let sleep = sleep_pass_closes_mistake(&root)?;
    let rollback = regression_rollback(&root)?;
    let goodhart = goodhart_guard();

    support::write_json(
        &root.join("issue1246-online-learning-fsv.json"),
        &json!({
            "issue": ISSUE,
            "surface": "G8 Anneal online learner engine",
            "research_verdict": "worth keeping at current data scale as a small online head over frozen features: rewards update the head, trusted contradictions enter surprise-prioritized replay, sleep-pass closes mistakes, regression rollback protects prior behavior, and Goodhart checks reject train-only gains",
            "source_of_truth": [
                "Aster anchors CF rows",
                "Aster online CF outcome queue rows",
                "Aster anneal_mistakes CF rows",
                "Aster anneal_replay CF snapshot rows",
                "Aster anneal_heads CF rows",
                "Aster anneal_rollback CF rows",
                "Aster anneal_health CF rows",
                "Aster ledger CF rows"
            ],
            "outcome_ingress": outcome,
            "sleep_pass": sleep,
            "regression_rollback": rollback,
            "goodhart": goodhart
        }),
    );
    Ok(())
}

fn outcome_ingress(root: &Path) -> Result<Value> {
    let (vault_dir, vault) = support::open_durable_vault(root, "issue1246-outcome-vault");
    let reward_cx = closure::cx(10);
    let contradiction_cx = closure::cx(11);
    vault.put(reward_cx.clone())?;
    vault.put(contradiction_cx.clone())?;

    let log = closure::mistake_log(&vault);
    let mut replay = closure::replay_buffer(&vault, 32);
    let clock = FixedClock::new(closure::TEST_TS + 10);
    let guard = closure::frozen_guard("issue1246-outcome-byte")?;
    let mut heads = OnlineHeadState::open_with_guard(
        AsterHeadStorage::new(&vault),
        support::durable_substrate(&clock, &vault, &vault_dir),
        Arc::new(FixedClock::new(closure::TEST_TS + 10)),
        [OnlineHead::new(HeadKind::Predictor, vec![0.0])?],
        guard.clone(),
    )?;
    let outcomes = OutcomeQueue::open(
        AsterOutcomeStorage::new(&vault),
        Arc::new(FixedClock::new(closure::TEST_TS + 10)),
    )?;
    let mut context = RecordOutcomeContext::new(&log, &mut replay, &mut heads, &outcomes, &vault);

    let reward = record_outcome(
        reward_cx.cx_id,
        reward_anchor(1.0),
        Some(OutcomePrediction {
            value: 0.8,
            trusted: true,
        }),
        &mut context,
        RecordOutcomeConfig::default(),
    )?;
    let contradiction = record_outcome(
        contradiction_cx.cx_id,
        reward_anchor(0.1),
        Some(OutcomePrediction {
            value: 0.9,
            trusted: true,
        }),
        &mut context,
        RecordOutcomeConfig::default(),
    )?;
    let frozen_after = guard.check()?;
    vault.flush()?;

    let reward = match reward {
        RecordOutcomeResult::Reward(value) => value,
        other => panic!("expected reward, got {other:?}"),
    };
    let contradiction = match contradiction {
        RecordOutcomeResult::Contradiction(value) => value,
        other => panic!("expected contradiction, got {other:?}"),
    };
    assert_eq!(reward.queue_seq, 1);
    assert!(reward.head_update.promoted);
    assert_eq!(contradiction.mistake_ref.seq, 1);
    assert!(contradiction.replay_added);
    assert!(frozen_after.violations.is_empty());

    Ok(json!({
        "vault": vault_dir.display().to_string(),
        "reward": reward,
        "contradiction": contradiction,
        "replay_len": replay.len(),
        "frozen_after": frozen_after,
        "anchors": closure::cf_rows(&vault, ColumnFamily::Anchors),
        "outcome_queue": closure::cf_rows(&vault, ColumnFamily::Online),
        "mistakes": closure::cf_rows(&vault, ColumnFamily::AnnealMistakes),
        "replay": closure::cf_rows(&vault, ColumnFamily::AnnealReplay),
        "heads": closure::decoded_heads(&vault),
        "ledger": closure::ledger_rows(&vault)
    }))
}

fn sleep_pass_closes_mistake(root: &Path) -> Result<Value> {
    let (vault_dir, vault) = support::open_durable_vault(root, "issue1246-sleep-vault");
    let cx = closure::cx(12);
    vault.put(cx.clone())?;
    let guard = closure::frozen_guard("issue1246-sleep-byte")?;
    let frozen_before = guard.check()?;
    let log = closure::mistake_log(&vault);
    let mut replay = closure::replay_buffer(&vault, 16);
    let replay_record = record_mistake_for_replay(
        &log,
        &mut replay,
        cx.cx_id,
        0.9,
        0.1,
        AnchorKind::Reward,
        calyx_anneal::DEFAULT_SLEEP_PASS_MIN_SURPRISE,
    )?;
    let batch_before = replay.entries_by_priority();
    let before = {
        let clock = FixedClock::new(closure::TEST_TS + 20);
        let state = OnlineHeadState::open_with_guard(
            AsterHeadStorage::new(&vault),
            support::durable_substrate(&clock, &vault, &vault_dir),
            Arc::new(FixedClock::new(closure::TEST_TS + 20)),
            [OnlineHead::new(HeadKind::Predictor, vec![1.0])?],
            guard.clone(),
        )?;
        assert_no_regression(&state, &batch_before, &log, &vault)?
    };
    let clock = FixedClock::new(closure::TEST_TS + 21);
    let registry = closure::health_registry(&vault);
    let mut state = OnlineHeadState::open_with_guard(
        AsterHeadStorage::new(&vault),
        support::durable_substrate(&clock, &vault, &vault_dir),
        Arc::new(FixedClock::new(closure::TEST_TS + 21)),
        [OnlineHead::new(HeadKind::Predictor, vec![1.0])?],
        guard.clone(),
    )?;
    let outcome = run_sleep_pass(
        &mut state,
        &replay,
        &log,
        &vault,
        &registry,
        SleepPassConfig {
            batch_size: 1,
            seed: 0x1246,
            ..SleepPassConfig::default()
        },
    )?;
    let after = assert_no_regression(&state, &batch_before, &log, &vault)?;
    let frozen_after = guard.check()?;
    vault.flush()?;

    assert!(replay_record.replay_added);
    assert!(!before.passed);
    assert!(matches!(outcome, SleepPassOutcome::Promoted { .. }));
    assert!(after.passed);
    assert_eq!(regression_rate(&after)?, 0.0);
    assert_eq!(frozen_before.rows, frozen_after.rows);

    Ok(json!({
        "vault": vault_dir.display().to_string(),
        "replay_record": replay_record,
        "before_report": before,
        "sleep_pass_outcome": outcome,
        "after_report": after,
        "frozen_before": frozen_before,
        "frozen_after": frozen_after,
        "cf_hashes": closure::cf_hashes(&vault),
        "mistakes": closure::cf_rows(&vault, ColumnFamily::AnnealMistakes),
        "replay": closure::cf_rows(&vault, ColumnFamily::AnnealReplay),
        "heads": closure::decoded_heads(&vault),
        "ledger": closure::ledger_rows(&vault)
    }))
}

fn regression_rollback(root: &Path) -> Result<Value> {
    let (vault_dir, vault) = support::open_durable_vault(root, "issue1246-rollback-vault");
    let cx = closure::cx(13);
    vault.put(cx.clone())?;
    let log = closure::mistake_log(&vault);
    let reference = log.append(cx.cx_id, 0.2, 0.0, AnchorKind::Reward)?;
    let batch = [replay(reference, cx.cx_id, 0.0)?];
    let clock = FixedClock::new(closure::TEST_TS + 30);
    let mut state = OnlineHeadState::open(
        AsterHeadStorage::new(&vault),
        support::durable_substrate(&clock, &vault, &vault_dir),
        Arc::new(FixedClock::new(closure::TEST_TS + 30)),
        [OnlineHead::new(HeadKind::Predictor, vec![0.2])?],
    )?;
    let candidate = assert_no_regression(&state, &batch, &log, &vault)?;
    let error = state
        .update_with_regression(&batch, &log, &vault, 1.0, 0.0, RegressionConfig::strict())
        .unwrap_err();
    vault.flush()?;

    assert_eq!(error.code, calyx_anneal::CALYX_ANNEAL_REGRESSION_RECURRED);
    assert_eq!(regression_rate(&candidate)?, 1.0);
    assert!(closure::cf_rows(&vault, ColumnFamily::AnnealHeads).is_empty());

    Ok(json!({
        "vault": vault_dir.display().to_string(),
        "candidate_report": candidate,
        "error_code": error.code,
        "head_after_error": state.head(HeadKind::Predictor),
        "mistakes": closure::cf_rows(&vault, ColumnFamily::AnnealMistakes),
        "heads": closure::cf_rows(&vault, ColumnFamily::AnnealHeads),
        "rollback": closure::cf_rows(&vault, ColumnFamily::AnnealRollback),
        "ledger": closure::ledger_rows(&vault)
    }))
}

fn goodhart_guard() -> Value {
    let passed = GoodhartChecker::new(
        HeldOutSet::sealed("issue1246-heldout-pass", 8, j(10.0), j(10.3)),
        Arc::new(StaticWard { frac: 0.98 }),
    )
    .check(
        &j(10.0),
        &j(11.0),
        &[lens_delta(1, 0.4), lens_delta(2, 0.3)],
    )
    .unwrap();
    let failed = GoodhartChecker::new(
        HeldOutSet::sealed("issue1246-heldout-fail", 8, j(10.0), j(9.5)),
        Arc::new(StaticWard { frac: 0.98 }),
    )
    .check(&j(10.0), &j(11.0), &[lens_delta(3, 0.2)])
    .unwrap();

    assert!(passed.passed);
    assert!(!failed.passed);

    json!({
        "passed_report": passed,
        "failed_report": failed
    })
}

fn replay(
    reference: calyx_anneal::MistakeRef,
    cx_id: calyx_core::CxId,
    target: f64,
) -> Result<calyx_anneal::ReplayEntry> {
    calyx_anneal::ReplayEntry::new(
        cx_id,
        target,
        reference.surprise,
        reference,
        closure::TEST_TS,
    )
}

fn reward_anchor(value: f64) -> Anchor {
    Anchor {
        kind: AnchorKind::Reward,
        value: AnchorValue::Number(value),
        source: "issue1246-fsv".to_string(),
        observed_at: closure::TEST_TS,
        confidence: 1.0,
    }
}

fn j(value: f64) -> JValue {
    JValue {
        j: value,
        terms: JTerms {
            w1_info: value.abs(),
            w2_n_eff: 0.0,
            w3_sufficiency: 0.0,
            w4_kernel_recall: 0.0,
            w5_oracle_accuracy: 0.0,
            w6_mistake_rate: 0.0,
            w7_compression: 0.0,
            w8_coverage: 0.0,
            p_redundant: 0.0,
            p_ungrounded: 0.0,
            p_goodhart: 0.0,
        },
        dpi_ceiling: value.abs() + 10.0,
        dpi_headroom: 10.0,
        provisional_excluded: 0,
        weights: JWeights::default(),
    }
}

fn lens_delta(byte: u8, delta: f64) -> LensContributionDelta {
    LensContributionDelta {
        lens_id: LensId::from_bytes([byte; 16]),
        delta,
    }
}

struct StaticWard {
    frac: f64,
}

impl WardGtau for StaticWard {
    fn in_region_fraction(&self, _held_out_set: &HeldOutSet) -> Result<Option<f64>> {
        Ok(Some(self.frac))
    }
}
