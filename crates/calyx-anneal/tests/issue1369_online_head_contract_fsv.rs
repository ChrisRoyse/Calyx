use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use calyx_anneal::{
    ActionMetricSnapshot, ArtifactKey, ArtifactPtr, ArtifactReplayMeasurer, AsterHeadStorage,
    AsterMistakeStorage, AsterReplayStorage, HeadKind, MistakeLog, OnlineHead, OnlineHeadState,
    ReplayBuffer, ReplayQuery, TripwireMetric, decode_online_head, decode_replay_snapshot,
    record_mistake_for_replay,
};
use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::AsterVault;
use calyx_core::{
    AnchorKind, Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef, Modality, Result,
    VaultStore,
};
use calyx_ledger::{EntryKind, decode as decode_ledger};
use serde_json::{Value, json};

#[path = "fsv_support/mod.rs"]
mod fsv_support;
#[allow(dead_code)]
#[path = "support/fsv_bad_change.rs"]
mod support;

const TEST_TS: u64 = 1_783_800_369;
const TEST_NAME: &str = "issue1369_online_head_contract_manual_fsv";
const ROOT_ENV: &str = "CALYX_ISSUE1369_FSV_ROOT";
const READBACK_ENV: &str = "CALYX_ISSUE1369_FSV_READBACK_ONLY";
const FIXTURE_KEY: &[u8] = b"issue1369/unrelated-graph-fixture";
const FIXTURE_VALUE: &[u8] = b"graph-bytes-must-survive-online-learning";

#[test]
#[ignore = "requires CALYX_ISSUE1369_FSV_ROOT in a manual verification run"]
fn issue1369_online_head_contract_manual_fsv() -> Result<()> {
    let root = PathBuf::from(env::var(ROOT_ENV).expect("set CALYX_ISSUE1369_FSV_ROOT"));
    if env::var_os(READBACK_ENV).is_some() {
        println!(
            "{}",
            serde_json::to_string_pretty(&readback(&root)?).expect("serialize readback")
        );
        return Ok(());
    }

    fsv_support::reset_dir(&root);
    let (vault_dir, vault) = support::open_durable_vault(&root, "vault");
    vault.write_cf(
        ColumnFamily::Graph,
        FIXTURE_KEY.to_vec(),
        FIXTURE_VALUE.to_vec(),
    )?;
    let context = context(0.25);
    vault.put(context.clone())?;

    let clock = FixedClock::new(TEST_TS);
    let log = MistakeLog::open(AsterMistakeStorage::new(&vault), 16, Arc::new(clock))?;
    let mut replay = ReplayBuffer::open(AsterReplayStorage::new(&vault), 16, Arc::new(clock))?;
    let replay_record = record_mistake_for_replay(
        &log,
        &mut replay,
        context.cx_id,
        0.6,
        0.8,
        AnchorKind::Reward,
        0.0,
    )?;
    let batch = replay.entries_by_priority();
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0].target, 0.8);
    assert!((batch[0].surprise - 0.2).abs() < 1.0e-12);

    let mut state = OnlineHeadState::open(
        AsterHeadStorage::new(&vault),
        support::durable_substrate(&clock, &vault, &vault_dir)
            .with_replay_measurer(Arc::new(PassingArtifactMeasurer)),
        Arc::new(clock),
        [OnlineHead::new(HeadKind::Predictor, vec![0.0, 0.0])?],
    )?;
    let update = state.update(&batch, &vault, 1.0, 0.0)?;
    let predictor = state.head(HeadKind::Predictor).expect("predictor head");
    assert!(update.promoted);
    assert!((predictor.params[0] - 0.8).abs() < 1.0e-6);
    assert!((predictor.params[1] - 0.2).abs() < 1.0e-6);
    assert!((state.predict(&context) - 0.85).abs() < 1.0e-6);

    drop(state);
    drop(replay);
    drop(log);
    vault.flush()?;
    drop(vault);

    let child = Command::new(env::current_exe().expect("current test binary"))
        .args(["--ignored", "--exact", TEST_NAME, "--nocapture"])
        .env(ROOT_ENV, &root)
        .env(READBACK_ENV, "1")
        .output()
        .expect("spawn separate-process readback");
    fs::write(root.join("separate-process-stdout.txt"), &child.stdout).expect("write child stdout");
    fs::write(root.join("separate-process-stderr.txt"), &child.stderr).expect("write child stderr");
    assert!(
        child.status.success(),
        "readback process failed: {}",
        String::from_utf8_lossy(&child.stderr)
    );

    let persisted = readback(&root)?;
    assert_eq!(persisted["replay"]["decoded"]["entries"][0]["target"], 0.8);
    assert!(
        (persisted["replay"]["decoded"]["entries"][0]["surprise"]
            .as_f64()
            .expect("persisted surprise")
            - 0.2)
            .abs()
            < 1.0e-12
    );
    assert!(
        (persisted["predictor"]["served_prediction"]
            .as_f64()
            .expect("persisted prediction")
            - 0.85)
            .abs()
            < 1.0e-6
    );
    assert_eq!(persisted["graph_fixture"]["value_hex"], hex(FIXTURE_VALUE));
    assert!(replay_record.replay_added);

    fsv_support::write_json(&root.join("issue1369-readback.json"), &persisted);
    fsv_support::write_physical_size_list(&root.join("physical-files.txt"), &root);
    fsv_support::write_tree_manifest(&root, fsv_support::ManifestPathStyle::Slash);
    Ok(())
}

#[derive(Clone)]
struct PassingArtifactMeasurer;

impl ArtifactReplayMeasurer for PassingArtifactMeasurer {
    fn measure(
        &self,
        _key: &ArtifactKey,
        _artifact: &ArtifactPtr,
        _query: &ReplayQuery,
    ) -> Result<ActionMetricSnapshot> {
        Ok(ActionMetricSnapshot::from_values([
            (TripwireMetric::RecallAtK, 0.95),
            (TripwireMetric::GuardFAR, 0.001),
            (TripwireMetric::GuardFRR, 0.001),
            (TripwireMetric::SearchP99, 50.0),
            (TripwireMetric::IngestP95, 80.0),
        ]))
    }
}

fn readback(root: &Path) -> Result<Value> {
    let (vault_dir, vault) = support::open_durable_vault(root, "vault");
    let seq = vault.latest_seq();
    let fixture = vault
        .read_cf_at(seq, ColumnFamily::Graph, FIXTURE_KEY)?
        .expect("graph fixture persisted");
    assert_eq!(fixture, FIXTURE_VALUE);

    let replay_rows = vault.scan_cf_at(seq, ColumnFamily::AnnealReplay)?;
    assert_eq!(replay_rows.len(), 1);
    let replay = decode_replay_snapshot(&replay_rows[0].1)?;
    assert_eq!(replay.entries.len(), 1);
    assert_eq!(replay.entries[0].target, 0.8);
    assert!((replay.entries[0].surprise - 0.2).abs() < 1.0e-12);

    let head_rows = vault.scan_cf_at(seq, ColumnFamily::AnnealHeads)?;
    assert_eq!(head_rows.len(), 1);
    let predictor = decode_online_head(&head_rows[0].1)?;
    assert_eq!(predictor.kind, HeadKind::Predictor);
    assert_eq!(predictor.version, 1);
    assert!((predictor.params[0] - 0.8).abs() < 1.0e-6);
    assert!((predictor.params[1] - 0.2).abs() < 1.0e-6);
    let served_prediction = f64::from(predictor.params[0] + predictor.params[1] * 0.25);
    assert!((served_prediction - 0.85).abs() < 1.0e-6);

    let ledger = read_anneal_ledger_rows(&vault);
    assert!(
        ledger
            .iter()
            .any(|row| row["payload_json"]["action"] == "head_update")
    );

    Ok(json!({
        "issue": 1369,
        "source_of_truth": "reopened durable Aster Graph, AnnealReplay, AnnealHeads, and Ledger CF bytes",
        "vault": vault_dir.display().to_string(),
        "latest_seq": seq,
        "graph_fixture": {
            "key_hex": hex(FIXTURE_KEY),
            "value_hex": hex(&fixture),
        },
        "replay": {
            "key_hex": hex(&replay_rows[0].0),
            "value_hex": hex(&replay_rows[0].1),
            "decoded": replay,
        },
        "predictor": {
            "key_hex": hex(&head_rows[0].0),
            "value_hex": hex(&head_rows[0].1),
            "decoded": predictor,
            "constellation_signal": 0.25,
            "served_prediction": served_prediction,
        },
        "ledger": ledger,
    }))
}

fn read_anneal_ledger_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .expect("scan durable ledger")
        .into_iter()
        .filter_map(|(key, bytes)| {
            let entry = decode_ledger(&bytes).expect("decode durable ledger row");
            if entry.kind != EntryKind::Anneal {
                return None;
            }
            assert_eq!(key, ledger_key(entry.seq));
            Some(json!({
                "seq": entry.seq,
                "key_hex": hex(&key),
                "kind": entry.kind.as_str(),
                "entry_hash": hex(&entry.entry_hash),
                "payload_json": serde_json::from_slice::<Value>(&entry.payload)
                    .expect("decode anneal ledger payload"),
            }))
        })
        .collect()
}

fn context(signal: f64) -> Constellation {
    let mut scalars = BTreeMap::new();
    scalars.insert("signal".to_string(), signal);
    Constellation {
        cx_id: CxId::from_bytes([0x13; 16]),
        vault_id: fsv_support::vault_id(),
        panel_version: 1,
        created_at: TEST_TS,
        input_ref: InputRef {
            hash: [0x69; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars,
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1369,
            hash: [0x13; 32],
        },
        flags: CxFlags::default(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
