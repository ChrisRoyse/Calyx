use super::*;
use crate::cf::ledger_key;
use crate::ledger_view::AsterLedgerCfStore;
use crate::vault::{AsterVault, VaultOptions};
use calyx_core::VaultStore;
use calyx_ledger::{LedgerCfStore, VerifyResult, decode, verify_chain};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[test]
fn aster_batch_uses_big_endian_ledger_keys() {
    let rows = [WriteRow {
        cf: ColumnFamily::Ledger,
        key: ledger_key(7),
        value: b"entry".to_vec(),
    }];

    assert_eq!(rows[0].cf, ColumnFamily::Ledger);
    assert_eq!(rows[0].key, ledger_key(7));
    assert_eq!(rows[0].value, b"entry");
}

#[test]
fn recovered_hook_continues_existing_ledger_sequence() {
    let mut rows = Vec::new();
    let mut hook = recover_hook(
        &RecoveredBatches {
            batches: Vec::new(),
            last_recovered_seq: 0,
            wal_replay_floor_seq: 0,
            derived_content_floor_seq: 0,
            torn_tail: None,
            temporal_policy: None,
            dedup_policy: None,
            retention_horizon: crate::timetravel::RetentionHorizon::default(),
            router_latest_readback: false,
        },
        None,
    )
    .expect("recover empty hook");
    let guard = hook.get_mut().unwrap();
    let first = stage_ingest(guard, &mut rows, &sample_constellation()).expect("stage first");

    assert_eq!(first[0].ledger_ref().seq, 0);
    assert_eq!(guard.appender().next_seq(), 0);
    assert!(guard.appender().store().scan().unwrap().is_empty());
    let decoded = decode(&rows[0].value).unwrap();
    assert_eq!(decoded.kind, EntryKind::Ingest);
    let payload: serde_json::Value = serde_json::from_slice(&decoded.payload).unwrap();
    assert_eq!(payload["metadata"][METADATA_CHUNK_ID], "chunk-7");
    assert_eq!(payload["metadata"][METADATA_DATABASE_NAME], "db/main");

    let committed = commit_staged(guard, &first).expect("commit first");

    assert_eq!(committed.seq, 0);
    assert_eq!(guard.appender().next_seq(), 1);
    assert_eq!(guard.appender().store().scan().unwrap().len(), 1);
}

#[test]
fn physical_ledger_rows_recover_hook_when_manifest_view_has_gap() {
    let dir = test_dir("issue866-physical-ledger");
    let vault = AsterVault::new_durable(
        &dir,
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
        b"issue866-salt",
        VaultOptions::default(),
    )
    .expect("open durable vault");
    for seed in 0..4 {
        vault
            .put(sample_constellation_with_seed(seed))
            .expect("put sample");
    }
    vault.flush().expect("flush physical ledger");
    drop(vault);

    let physical = AsterLedgerCfStore::open(&dir).expect("open physical ledger");
    let physical_rows = physical.scan().expect("scan physical ledger");
    let physical_anchor = physical.head_anchor().expect("read physical head anchor");
    assert_eq!(physical_rows.len(), 4);
    assert_eq!(
        verify_chain(&physical, 0..4).expect("verify physical ledger"),
        VerifyResult::Intact { count: 4 }
    );
    let last = physical_rows.last().expect("last ledger row");
    let gapped_recovery = RecoveredBatches {
        batches: vec![crate::vault::durable::RecoveredBatch {
            seq: 4,
            rows: vec![WriteRow {
                cf: ColumnFamily::Ledger,
                key: ledger_key(last.seq),
                value: last.bytes.clone(),
            }],
        }],
        last_recovered_seq: 4,
        wal_replay_floor_seq: 0,
        derived_content_floor_seq: 0,
        torn_tail: None,
        temporal_policy: None,
        dedup_policy: None,
        retention_horizon: crate::timetravel::RetentionHorizon::default(),
        router_latest_readback: false,
    };

    let manifest_only_error = recover_hook(&gapped_recovery, None).unwrap_err();
    assert_eq!(manifest_only_error.code, "CALYX_LEDGER_CHAIN_BROKEN");
    let mut recovered =
        recover_hook_from_vault_dir(&dir, &gapped_recovery, None, None).expect("physical recovery");
    let guard = recovered.get_mut().expect("hook guard");

    assert_eq!(guard.appender().next_seq(), 4);
    write_issue866_artifact(
        physical_rows.len(),
        physical_anchor.as_ref().map(|anchor| anchor.height),
        manifest_only_error.code,
        guard.appender().next_seq(),
    );
    cleanup(dir);
}

#[test]
fn anchored_checkpoint_recovery_hydrates_bounded_tail_window() {
    let dir = test_dir("checkpoint-bounded-ledger-hook");
    let vault = AsterVault::new_durable(
        &dir,
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
        b"checkpoint-bounded-salt",
        VaultOptions {
            ledger_checkpoint: Some(CheckpointConfig::new(3)),
            ..VaultOptions::default()
        },
    )
    .expect("open durable vault");
    for seed in 0..24 {
        vault
            .put(sample_constellation_with_seed(seed))
            .expect("put sample");
    }
    vault.flush().expect("flush physical ledger");
    drop(vault);

    let physical = AsterLedgerCfStore::open(&dir).expect("open physical ledger");
    let physical_rows = physical.scan().expect("scan physical ledger");
    let recovery = RecoveredBatches {
        batches: Vec::new(),
        last_recovered_seq: 0,
        wal_replay_floor_seq: 0,
        derived_content_floor_seq: 0,
        torn_tail: None,
        temporal_policy: None,
        dedup_policy: None,
        retention_horizon: crate::timetravel::RetentionHorizon::default(),
        router_latest_readback: false,
    };

    let mut recovered =
        recover_hook_from_vault_dir(&dir, &recovery, Some(CheckpointConfig::new(3)), None)
            .expect("bounded physical recovery");
    let guard = recovered.get_mut().expect("hook guard");
    let hydrated_rows = guard.appender().store().scan().unwrap().len();

    assert!(
        physical_rows.len() > 20,
        "physical_rows={}",
        physical_rows.len()
    );
    assert!(
        hydrated_rows < physical_rows.len(),
        "hydrated_rows={hydrated_rows} physical_rows={}",
        physical_rows.len()
    );
    assert_eq!(guard.appender().next_seq(), physical_rows.len() as u64);
    cleanup(dir);
}

fn sample_constellation() -> Constellation {
    sample_constellation_with_seed(7)
}

fn sample_constellation_with_seed(seed: u8) -> Constellation {
    Constellation {
        cx_id: calyx_core::CxId::from_bytes([seed; 16]),
        vault_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
        panel_version: 1,
        created_at: 42 + u64::from(seed),
        input_ref: calyx_core::InputRef {
            hash: [seed; 32],
            pointer: Some(format!("synthetic://ledger-hook/{seed}")),
            redacted: false,
        },
        modality: calyx_core::Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::from([
            (METADATA_CHUNK_ID.to_string(), "chunk-7".to_string()),
            (METADATA_DATABASE_NAME.to_string(), "db/main".to_string()),
        ]),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: u64::from(seed),
            hash: [seed; 32],
        },
        flags: calyx_core::CxFlags::default(),
    }
}

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).unwrap();
}

fn write_issue866_artifact(
    physical_rows: usize,
    head_anchor_height: Option<u64>,
    manifest_only_error_code: &'static str,
    recovered_next_seq: u64,
) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    fs::create_dir_all(&root).unwrap();
    let artifact = serde_json::json!({
        "schema": "calyx-issue866-manifest-ledger-recovery-v1",
        "physical_ledger_rows": physical_rows,
        "head_anchor_height": head_anchor_height,
        "manifest_only_error_code": manifest_only_error_code,
        "recovered_next_seq": recovered_next_seq,
        "physical_recovery_used": true
    });
    fs::write(
        root.join("issue866_manifest_ledger_recovery_readback.json"),
        serde_json::to_vec_pretty(&artifact).unwrap(),
    )
    .unwrap();
}
