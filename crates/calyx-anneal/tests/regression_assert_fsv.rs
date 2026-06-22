use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use calyx_anneal::{
    AsterHeadStorage, AsterMistakeStorage, CALYX_ANNEAL_REGRESSION_NAN_PREDICTION,
    CALYX_ANNEAL_REGRESSION_RECURRED, CALYX_ANNEAL_REGRESSION_SOURCE_UNAVAILABLE, HeadKind,
    MistakeLog, MistakeRef, OnlineHead, OnlineHeadState, RegressionConfig, RegressionPredictor,
    RegressionReport, ReplayEntry, assert_no_regression, record_regression, regression_rate,
};
use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::AsterVault;
use calyx_core::{
    AnchorKind, Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef, Modality, Result,
    VaultStore,
};
use calyx_ledger::{EntryKind, decode as decode_ledger};
use serde_json::{Value, json};

mod fsv_support;
use fsv_support::vault_id;

#[allow(dead_code)]
#[path = "support/fsv_bad_change.rs"]
mod support;

const TEST_TS: u64 = 1_785_500_410;

#[test]
#[ignore = "requires CALYX_ISSUE410_FSV_ROOT in a manual verification run"]
fn fsv_regression_assert_manual() -> Result<()> {
    let root = std::path::PathBuf::from(env::var("CALYX_ISSUE410_FSV_ROOT").unwrap());
    support::reset_dir(&root);

    let happy = happy_update_path(&root)?;
    let strict_revert = strict_noop_revert_path(&root)?;
    let edges = edge_audit(&root)?;

    support::write_json(
        &root.join("issue410-fsv-artifact.json"),
        &json!({
            "issue": 410,
            "surface": "PH45 T05 RegressionAssert",
            "source_of_truth": [
                "RegressionReport JSON artifacts under CALYX_ISSUE410_FSV_ROOT",
                "Aster base CF constellation rows addressed through VaultStore::get",
                "Aster anneal_mistakes CF rows",
                "Aster anneal_heads CF rows",
                "Aster anneal_rollback CF rows",
                "Aster ledger CF rows decoded from on-disk bytes"
            ],
            "trigger_to_outcome": {
                "trigger": "OnlineHeadState::update_with_regression over replay batch",
                "expected_outcome": "non-recurrent updates persist head rows and HeadUpdate ledger rows; recurrent strict updates rollback and relog exact failure evidence"
            },
            "happy": happy,
            "strict_revert": strict_revert,
            "edges": edges
        }),
    );
    Ok(())
}

fn happy_update_path(root: &Path) -> Result<Value> {
    let (vault_dir, vault) = support::open_durable_vault(root, "happy-vault");
    let cx = cx(1);
    vault.put(cx.clone())?;
    let stored = vault.get(cx.cx_id, vault.snapshot())?;
    let log = MistakeLog::open(
        AsterMistakeStorage::new(&vault),
        16,
        Arc::new(FixedClock::new(TEST_TS)),
    )?;
    let reference = log.append(cx.cx_id, 0.0, 1.0, AnchorKind::Reward)?;
    let batch = [replay(reference, cx.cx_id)?];
    let clock = FixedClock::new(TEST_TS);
    let mut state = OnlineHeadState::open(
        AsterHeadStorage::new(&vault),
        support::durable_substrate(&clock, &vault, &vault_dir),
        Arc::new(FixedClock::new(TEST_TS)),
        [OnlineHead::new(HeadKind::Predictor, vec![0.0])?],
    )?;

    let before = assert_no_regression(&state, &batch, &log, &vault)?;
    write_report(root, "happy-before-report.json", &before);
    let outcome =
        state.update_with_regression(&batch, &log, &vault, 1.0, 0.0, RegressionConfig::strict())?;
    let after = assert_no_regression(&state, &batch, &log, &vault)?;
    write_report(root, "happy-after-report.json", &after);
    vault.flush()?;

    let ledger = ledger_rows(&vault);
    assert!(!before.passed);
    assert!(outcome.report.passed);
    assert!(after.passed);
    assert_eq!(regression_rate(&after)?, 0.0);
    assert!(has_ledger_action(
        &ledger,
        "head_update",
        "regression_rate=0.000000"
    ));
    assert_eq!(state.head(HeadKind::Predictor).unwrap().params, vec![1.0]);

    Ok(json!({
        "vault": vault_dir.display().to_string(),
        "stored_context_cx": stored.cx_id,
        "known_input": {"old_prediction": 0.0, "observed": 1.0, "old_surprise": 1.0},
        "hand_expected": {"updated_prediction": 1.0, "new_surprise": 0.0, "regression_rate": 0.0},
        "before_report": before,
        "update_outcome": outcome,
        "after_report": after,
        "head_readback": state.readback()?,
        "head_rows": cf_rows(&vault, ColumnFamily::AnnealHeads),
        "mistake_rows": cf_rows(&vault, ColumnFamily::AnnealMistakes),
        "rollback_rows": cf_rows(&vault, ColumnFamily::AnnealRollback),
        "ledger_rows": ledger
    }))
}

fn strict_noop_revert_path(root: &Path) -> Result<Value> {
    let (vault_dir, vault) = support::open_durable_vault(root, "revert-vault");
    let cx = cx(2);
    vault.put(cx.clone())?;
    let log = MistakeLog::open(
        AsterMistakeStorage::new(&vault),
        16,
        Arc::new(FixedClock::new(TEST_TS)),
    )?;
    let reference = log.append(cx.cx_id, 0.2, 0.0, AnchorKind::Reward)?;
    let batch = [replay(reference, cx.cx_id)?];
    let clock = FixedClock::new(TEST_TS);
    let mut state = OnlineHeadState::open(
        AsterHeadStorage::new(&vault),
        support::durable_substrate(&clock, &vault, &vault_dir),
        Arc::new(FixedClock::new(TEST_TS)),
        [OnlineHead::new(HeadKind::Predictor, vec![0.2])?],
    )?;

    let before = assert_no_regression(&state, &batch, &log, &vault)?;
    write_report(root, "revert-candidate-report.json", &before);
    let error = state
        .update_with_regression(&batch, &log, &vault, 1.0, 0.0, RegressionConfig::strict())
        .unwrap_err();
    vault.flush()?;

    let ledger = ledger_rows(&vault);
    assert_eq!(error.code, CALYX_ANNEAL_REGRESSION_RECURRED);
    assert_eq!(regression_rate(&before)?, 1.0);
    assert!(has_ledger_action(
        &ledger,
        "head_update",
        "regression_rate=1.000000"
    ));
    assert!(has_ledger_action(
        &ledger,
        "head_update_reverted",
        "regression_rate=1.000000"
    ));
    assert!(cf_rows(&vault, ColumnFamily::AnnealHeads).is_empty());

    Ok(json!({
        "vault": vault_dir.display().to_string(),
        "known_input": {"old_prediction": 0.2, "observed": 0.0, "old_surprise": 0.2},
        "hand_expected": {"noop_prediction": 0.2, "new_surprise": 0.2, "recurred": true, "regression_rate": 1.0},
        "candidate_report": before,
        "error_code": error.code,
        "head_after_error": state.head(HeadKind::Predictor),
        "head_rows_after_error": cf_rows(&vault, ColumnFamily::AnnealHeads),
        "mistake_rows": cf_rows(&vault, ColumnFamily::AnnealMistakes),
        "rollback_rows": cf_rows(&vault, ColumnFamily::AnnealRollback),
        "ledger_rows": ledger
    }))
}

fn edge_audit(root: &Path) -> Result<Value> {
    let (vault_dir, vault) = support::open_durable_vault(root, "edge-vault");
    let log = MistakeLog::open(
        AsterMistakeStorage::new(&vault),
        16,
        Arc::new(FixedClock::new(TEST_TS)),
    )?;

    let empty = assert_no_regression(&FixedPredictor(0.0), &[], &log, &vault)?;
    write_report(root, "edge-empty-report.json", &empty);

    let all_batch = append_cases(
        &vault,
        &log,
        &[(10, 0.1, 0.0), (11, 0.2, 0.0), (12, 0.3, 0.0)],
    )?;
    let all = assert_no_regression(&FixedPredictor(1.0), &all_batch, &log, &vault)?;
    write_report(root, "edge-all-recur-report.json", &all);

    let relog_before = cf_rows(&vault, ColumnFamily::AnnealMistakes).len();
    let relogged = record_regression(&all.results[0], &log)?.unwrap();
    vault.flush()?;
    let relog_after = cf_rows(&vault, ColumnFamily::AnnealMistakes);

    let nan_batch = append_cases(&vault, &log, &[(20, 0.5, 0.0)])?;
    let nan = assert_no_regression(&NanPredictor, &nan_batch, &log, &vault)?;
    write_report(root, "edge-nan-report.json", &nan);

    let missing_ref = log.append(cx_id(99), 1.0, 0.0, AnchorKind::Reward)?;
    let missing_batch = [replay(missing_ref, cx_id(99))?];
    let missing_error =
        assert_no_regression(&FixedPredictor(0.0), &missing_batch, &log, &vault).unwrap_err();

    assert!(empty.passed);
    assert_eq!(regression_rate(&empty)?, 0.0);
    assert_eq!(all.regression_count, 3);
    assert_eq!(regression_rate(&all)?, 1.0);
    assert!(relogged.surprise > all.results[0].old_surprise);
    assert!(relog_after.len() > relog_before);
    assert_eq!(nan.results[0].new_surprise, f64::MAX);
    assert_eq!(
        nan.results[0].prediction_error.as_deref(),
        Some(CALYX_ANNEAL_REGRESSION_NAN_PREDICTION)
    );
    assert_eq!(
        missing_error.code,
        CALYX_ANNEAL_REGRESSION_SOURCE_UNAVAILABLE
    );

    Ok(json!({
        "vault": vault_dir.display().to_string(),
        "empty": empty,
        "all_recur": all,
        "record_regression": {
            "before_mistake_row_count": relog_before,
            "after_mistake_row_count": relog_after.len(),
            "relogged": relogged
        },
        "nan": nan,
        "missing_context_error_code": missing_error.code,
        "mistake_rows": relog_after,
        "ledger_rows": ledger_rows(&vault)
    }))
}

fn append_cases(
    vault: &AsterVault,
    log: &MistakeLog<AsterMistakeStorage<'_, calyx_core::SystemClock>>,
    cases: &[(u8, f64, f64)],
) -> Result<Vec<ReplayEntry>> {
    cases
        .iter()
        .map(|(seed, predicted, observed)| {
            let cx = cx(*seed);
            vault.put(cx.clone())?;
            let reference = log.append(cx.cx_id, *predicted, *observed, AnchorKind::Reward)?;
            replay(reference, cx.cx_id)
        })
        .collect()
}

fn write_report(root: &Path, name: &str, report: &RegressionReport) {
    fs::write(
        root.join(name),
        serde_json::to_vec_pretty(report).expect("serialize regression report"),
    )
    .expect("write regression report");
}

fn cf_rows(vault: &AsterVault, cf: ColumnFamily) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), cf)
        .unwrap()
        .into_iter()
        .map(|(key, value)| {
            json!({
                "cf": cf.name(),
                "key_hex": hex(&key),
                "value_len": value.len(),
                "value_hex": hex(&value)
            })
        })
        .collect()
}

fn ledger_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .unwrap()
        .into_iter()
        .map(|(key, value)| {
            let entry = decode_ledger(&value).unwrap();
            assert_eq!(key, ledger_key(entry.seq));
            let payload_json = serde_json::from_slice::<Value>(&entry.payload).ok();
            json!({
                "seq": entry.seq,
                "key_hex": hex(&key),
                "value_len": value.len(),
                "value_hex": hex(&value),
                "kind": entry.kind.as_str(),
                "is_anneal": entry.kind == EntryKind::Anneal,
                "entry_hash": hex(&entry.entry_hash),
                "payload_json": payload_json,
                "payload_hex": hex(&entry.payload)
            })
        })
        .collect()
}

fn has_ledger_action(rows: &[Value], action: &str, description_contains: &str) -> bool {
    rows.iter().any(|row| {
        row["payload_json"]["action"].as_str() == Some(action)
            && row["payload_json"]["description"]
                .as_str()
                .is_some_and(|description| description.contains(description_contains))
    })
}

fn replay(reference: MistakeRef, cx_id: CxId) -> Result<ReplayEntry> {
    ReplayEntry::new(cx_id, reference.surprise, reference, TEST_TS)
}

fn cx(seed: u8) -> Constellation {
    Constellation {
        cx_id: cx_id(seed),
        vault_id: vault_id(),
        panel_version: 1,
        created_at: TEST_TS,
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: u64::from(seed),
            hash: [seed; 32],
        },
        flags: CxFlags::default(),
    }
}

fn cx_id(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct FixedPredictor(f64);

impl RegressionPredictor for FixedPredictor {
    fn predict_regression(&self, _cx: &Constellation) -> f64 {
        self.0
    }
}

struct NanPredictor;

impl RegressionPredictor for NanPredictor {
    fn predict_regression(&self, _cx: &Constellation) -> f64 {
        f64::NAN
    }
}
