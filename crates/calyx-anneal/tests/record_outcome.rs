use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use calyx_anneal::{
    AsterHeadStorage, AsterMistakeStorage, AsterOutcomeStorage, AsterReplayStorage, HeadKind,
    OnlineHead, OnlineHeadState, OutcomePrediction, OutcomeQueue, RecordOutcomeConfig,
    RecordOutcomeContext, RecordOutcomeResult, decode_online_head, decode_outcome_queue_entry,
    record_outcome,
};
use calyx_aster::cf::{ColumnFamily, anchor_key, ledger_key};
use calyx_aster::vault::AsterVault;
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef,
    Modality, Result, VaultStore,
};
use calyx_ledger::{EntryKind, decode as decode_ledger};
use serde_json::{Value, json};

mod fsv_support;
use fsv_support::vault_id;

#[allow(dead_code)]
#[path = "support/fsv_bad_change.rs"]
mod support;

const TS: u64 = 1_785_500_580;

#[test]
fn record_outcome_reward_writes_anchor_queue_head_and_ledger() -> Result<()> {
    let root = test_root("reward");
    let result = reward_scenario(&root)?;

    assert_eq!(result["result"]["queue_seq"], json!(1));
    assert_eq!(result["queue"][0]["entry"]["observed"], json!(1.0));
    assert_eq!(result["heads"][0]["head"]["params"], json!([1.0]));
    assert!(has_action(&result["ledger"], "outcome_reward"));
    assert!(has_action(&result["ledger"], "head_update"));
    Ok(())
}

#[test]
fn record_outcome_trusted_contradiction_writes_mistake_not_queue() -> Result<()> {
    let root = test_root("contradiction");
    let result = contradiction_scenario(&root)?;

    assert_eq!(result["result"]["surprise"], json!(0.8));
    assert_eq!(result["mistakes"].as_array().unwrap().len(), 1);
    assert_eq!(result["queue"].as_array().unwrap().len(), 0);
    assert!(has_action(&result["ledger"], "outcome_contradiction"));
    Ok(())
}

#[test]
fn record_outcome_fail_closed_edges_do_not_write_rows() -> Result<()> {
    let root = test_root("edges");
    let invalid_config = invalid_config_edge(&root)?;
    let invalid_anchor = invalid_anchor_edge(&root)?;
    let untrusted = untrusted_disagreement_edge(&root)?;

    assert_eq!(
        invalid_config["error_code"],
        json!("CALYX_ANNEAL_OUTCOME_INVALID_CONFIG")
    );
    assert_eq!(
        invalid_config["after_counts"],
        invalid_config["before_counts"]
    );
    assert_eq!(
        invalid_anchor["error_code"],
        json!("CALYX_ANNEAL_OUTCOME_INVALID_ANCHOR")
    );
    assert_eq!(
        invalid_anchor["after_counts"],
        invalid_anchor["before_counts"]
    );
    assert_eq!(untrusted["mistake_count"], json!(0));
    assert_eq!(untrusted["queue_count"], json!(1));
    Ok(())
}

#[test]
#[ignore = "requires CALYX_ISSUE580_FSV_ROOT in a manual verification run"]
fn fsv_record_outcome_manual() -> Result<()> {
    let root = PathBuf::from(env::var("CALYX_ISSUE580_FSV_ROOT").unwrap());
    support::reset_dir(&root);

    let reward = reward_scenario(&root)?;
    let contradiction = contradiction_scenario(&root)?;
    let invalid_config = invalid_config_edge(&root)?;
    let invalid_anchor = invalid_anchor_edge(&root)?;
    let untrusted = untrusted_disagreement_edge(&root)?;

    support::write_json(
        &root.join("issue580-record-outcome-fsv.json"),
        &json!({
            "issue": 580,
            "surface": "PH45 record_outcome(cx, anchor) reward and contradiction ingress",
            "source_of_truth": [
                "Aster anchors CF rows",
                "Aster online CF DeltaJQueue rows",
                "Aster anneal_heads CF rows",
                "Aster anneal_mistakes CF rows",
                "Aster anneal_replay CF rows",
                "Aster ledger CF rows",
                "raw WAL/SST bytes under each durable vault"
            ],
            "trigger_to_outcome": {
                "trigger": "record_outcome(cx_id, anchor, prediction)",
                "expected": "anchors persist; trusted contradictions become mistakes/replay; non-contradictions queue deltaJ reward and update heads; every call writes Anneal ledger evidence"
            },
            "hand_expected": {
                "reward": "observed pass=true -> 1.0; predicted=0.8 trusted -> surprise=0.2 < 0.3, queue seq 1, head param 0->1.0",
                "contradiction": "predicted=0.9 observed=0.1 -> surprise=0.8 >= 0.3, MistakeLog seq 1 and replay len 1",
                "untrusted": "predicted=0.0 observed=1.0 but trusted=false -> reward queue, no mistake"
            },
            "reward": reward,
            "contradiction": contradiction,
            "edges": {
                "invalid_config": invalid_config,
                "invalid_anchor": invalid_anchor,
                "untrusted_disagreement": untrusted
            }
        }),
    );
    Ok(())
}

fn reward_scenario(root: &Path) -> Result<Value> {
    let (vault_dir, vault) = support::open_durable_vault(root, "reward-vault");
    let cx = insert_cx(&vault, 1)?;
    let outcome = record(
        &vault,
        &vault_dir,
        cx.cx_id,
        pass_anchor(),
        Some(OutcomePrediction {
            value: 0.8,
            trusted: true,
        }),
        RecordOutcomeConfig::default(),
    )?;
    vault.flush()?;
    Ok(json!({
        "vault": display(&vault_dir),
        "result": match outcome {
            RecordOutcomeResult::Reward(reward) => json!(reward),
            other => json!({"unexpected": format!("{other:?}")}),
        },
        "anchors": cf_rows(&vault, ColumnFamily::Anchors),
        "queue": outcome_rows(&vault),
        "heads": head_rows(&vault),
        "mistakes": cf_rows(&vault, ColumnFamily::AnnealMistakes),
        "replay": cf_rows(&vault, ColumnFamily::AnnealReplay),
        "ledger": ledger_rows(&vault),
        "anchor_key_hex": hex(&anchor_key(cx.cx_id, &AnchorKind::TestPass)),
    }))
}

fn contradiction_scenario(root: &Path) -> Result<Value> {
    let (vault_dir, vault) = support::open_durable_vault(root, "contradiction-vault");
    let cx = insert_cx(&vault, 2)?;
    let outcome = record(
        &vault,
        &vault_dir,
        cx.cx_id,
        reward_anchor(0.1),
        Some(OutcomePrediction {
            value: 0.9,
            trusted: true,
        }),
        RecordOutcomeConfig::default(),
    )?;
    vault.flush()?;
    Ok(json!({
        "vault": display(&vault_dir),
        "result": match outcome {
            RecordOutcomeResult::Contradiction(value) => json!(value),
            other => json!({"unexpected": format!("{other:?}")}),
        },
        "anchors": cf_rows(&vault, ColumnFamily::Anchors),
        "queue": outcome_rows(&vault),
        "heads": head_rows(&vault),
        "mistakes": cf_rows(&vault, ColumnFamily::AnnealMistakes),
        "replay": cf_rows(&vault, ColumnFamily::AnnealReplay),
        "ledger": ledger_rows(&vault),
    }))
}

fn invalid_config_edge(root: &Path) -> Result<Value> {
    let (vault_dir, vault) = support::open_durable_vault(root, "invalid-config-vault");
    let cx = insert_cx(&vault, 3)?;
    let before = counts(&vault);
    let config = RecordOutcomeConfig {
        action_cost: 0.0,
        ..Default::default()
    };
    let error = record(&vault, &vault_dir, cx.cx_id, pass_anchor(), None, config)
        .expect_err("invalid config must fail closed");
    let after = counts(&vault);
    Ok(json!({
        "error_code": error.code,
        "before_counts": before,
        "after_counts": after,
    }))
}

fn invalid_anchor_edge(root: &Path) -> Result<Value> {
    let (vault_dir, vault) = support::open_durable_vault(root, "invalid-anchor-vault");
    let cx = insert_cx(&vault, 4)?;
    let before = counts(&vault);
    let error = record(
        &vault,
        &vault_dir,
        cx.cx_id,
        Anchor {
            kind: AnchorKind::Reward,
            value: AnchorValue::Number(f64::NAN),
            source: "issue580-fsv".to_string(),
            observed_at: TS,
            confidence: 1.0,
        },
        None,
        RecordOutcomeConfig::default(),
    )
    .expect_err("invalid anchor must fail closed");
    let after = counts(&vault);
    Ok(json!({
        "error_code": error.code,
        "before_counts": before,
        "after_counts": after,
    }))
}

fn untrusted_disagreement_edge(root: &Path) -> Result<Value> {
    let (vault_dir, vault) = support::open_durable_vault(root, "untrusted-vault");
    let cx = insert_cx(&vault, 5)?;
    let outcome = record(
        &vault,
        &vault_dir,
        cx.cx_id,
        pass_anchor(),
        Some(OutcomePrediction {
            value: 0.0,
            trusted: false,
        }),
        RecordOutcomeConfig::default(),
    )?;
    vault.flush()?;
    Ok(json!({
        "result": match outcome {
            RecordOutcomeResult::Reward(reward) => json!(reward),
            other => json!({"unexpected": format!("{other:?}")}),
        },
        "queue_count": outcome_rows(&vault).len(),
        "mistake_count": cf_rows(&vault, ColumnFamily::AnnealMistakes).len(),
        "ledger": ledger_rows(&vault),
    }))
}

fn record(
    vault: &AsterVault,
    vault_dir: &Path,
    cx_id: CxId,
    anchor: Anchor,
    prediction: Option<OutcomePrediction>,
    config: RecordOutcomeConfig,
) -> Result<RecordOutcomeResult> {
    let clock = FixedClock::new(TS);
    let log = calyx_anneal::MistakeLog::open(AsterMistakeStorage::new(vault), 16, Arc::new(clock))?;
    let mut replay =
        calyx_anneal::ReplayBuffer::open(AsterReplayStorage::new(vault), 16, Arc::new(clock))?;
    let outcomes = OutcomeQueue::open(AsterOutcomeStorage::new(vault), Arc::new(clock))?;
    let substrate = support::durable_substrate(&clock, vault, vault_dir);
    let mut heads = OnlineHeadState::open(
        AsterHeadStorage::new(vault),
        substrate,
        Arc::new(clock),
        [OnlineHead::new(HeadKind::Predictor, vec![0.0])?],
    )?;
    let mut context = RecordOutcomeContext::new(&log, &mut replay, &mut heads, &outcomes);
    record_outcome(cx_id, anchor, prediction, &mut context, config)
}

fn insert_cx(vault: &AsterVault, seed: u8) -> Result<Constellation> {
    let cx = Constellation {
        cx_id: CxId::from_bytes([seed; 16]),
        vault_id: vault_id(),
        panel_version: 1,
        created_at: TS,
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
    };
    vault.put(cx.clone())?;
    Ok(cx)
}

fn pass_anchor() -> Anchor {
    Anchor {
        kind: AnchorKind::TestPass,
        value: AnchorValue::Bool(true),
        source: "issue580-fsv".to_string(),
        observed_at: TS,
        confidence: 1.0,
    }
}

fn reward_anchor(value: f64) -> Anchor {
    Anchor {
        kind: AnchorKind::Reward,
        value: AnchorValue::Number(value),
        source: "issue580-fsv".to_string(),
        observed_at: TS,
        confidence: 1.0,
    }
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
                "value_hex": hex(&value),
            })
        })
        .collect()
}

fn outcome_rows(vault: &AsterVault) -> Vec<Value> {
    cf_rows(vault, ColumnFamily::Online)
        .into_iter()
        .filter_map(|mut row| {
            let value = hex_to_bytes(row["value_hex"].as_str().unwrap()).ok()?;
            let entry = decode_outcome_queue_entry(&value).ok()?;
            row["entry"] = json!(entry);
            Some(row)
        })
        .collect()
}

fn head_rows(vault: &AsterVault) -> Vec<Value> {
    cf_rows(vault, ColumnFamily::AnnealHeads)
        .into_iter()
        .filter_map(|mut row| {
            let value = hex_to_bytes(row["value_hex"].as_str().unwrap()).ok()?;
            let head = decode_online_head(&value).ok()?;
            row["head"] = json!(head);
            Some(row)
        })
        .collect()
}

fn ledger_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .unwrap()
        .into_iter()
        .map(|(key, value)| {
            let decoded = decode_ledger(&value).ok();
            json!({
                "key_hex": hex(&key),
                "key_is_expected": decoded.as_ref().is_none_or(|entry| key == ledger_key(entry.seq)),
                "value_hex": hex(&value),
                "kind": decoded.as_ref().map(|entry| entry.kind.as_str()).unwrap_or("unknown"),
                "is_anneal": decoded.as_ref().is_some_and(|entry| entry.kind == EntryKind::Anneal),
                "payload_json": decoded
                    .and_then(|entry| serde_json::from_slice::<Value>(&entry.payload).ok()),
            })
        })
        .collect()
}

fn counts(vault: &AsterVault) -> Value {
    json!({
        "anchors": cf_rows(vault, ColumnFamily::Anchors).len(),
        "online": cf_rows(vault, ColumnFamily::Online).len(),
        "heads": cf_rows(vault, ColumnFamily::AnnealHeads).len(),
        "mistakes": cf_rows(vault, ColumnFamily::AnnealMistakes).len(),
        "replay": cf_rows(vault, ColumnFamily::AnnealReplay).len(),
    })
}

fn has_action(rows: &Value, action: &str) -> bool {
    rows.as_array().unwrap().iter().any(|row| {
        row["payload_json"]["action"]
            .as_str()
            .is_some_and(|value| value == action)
    })
}

fn hex_to_bytes(value: &str) -> std::result::Result<Vec<u8>, ()> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| {
            let high = (chunk[0] as char).to_digit(16).ok_or(())?;
            let low = (chunk[1] as char).to_digit(16).ok_or(())?;
            Ok(((high << 4) | low) as u8)
        })
        .collect()
}

fn test_root(label: &str) -> PathBuf {
    let root = env::temp_dir().join(format!(
        "calyx-record-outcome-{label}-{}",
        std::process::id()
    ));
    support::reset_dir(&root);
    root
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
