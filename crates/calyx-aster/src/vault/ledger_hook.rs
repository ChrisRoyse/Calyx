use super::durable::RecoveredBatches;
use super::encode::WriteRow;
use crate::cf::ColumnFamily;
use crate::ledger_view::AsterLedgerCfStore;
use calyx_core::{
    CalyxError, Constellation, LedgerRef, METADATA_CHUNK_ID, METADATA_DATABASE_NAME, Result,
    SystemClock,
};
use calyx_ledger::{
    ActorId, CheckpointConfig, DefaultLedgerHook, EntryKind, LedgerAppender, LedgerCfStore,
    MemoryLedgerStore, PayloadBuilder, StagedLedgerRow, SubjectId,
};
use serde_json::json;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

pub(super) type AsterLedgerHook = Mutex<DefaultLedgerHook<MemoryLedgerStore, SystemClock>>;
pub(super) type AsterLedgerHookGuard<'a> =
    MutexGuard<'a, DefaultLedgerHook<MemoryLedgerStore, SystemClock>>;

pub(super) fn recover_hook(
    recovery: &RecoveredBatches,
    checkpoint: Option<CheckpointConfig>,
) -> Result<AsterLedgerHook> {
    recover_hook_from_store(recovered_ledger_store(recovery)?, checkpoint)
}

pub(super) fn recover_hook_from_vault_dir(
    vault_dir: &Path,
    recovery: &RecoveredBatches,
    checkpoint: Option<CheckpointConfig>,
) -> Result<AsterLedgerHook> {
    let store = match physical_ledger_store(vault_dir)? {
        Some(store) => store,
        None => recovered_ledger_store(recovery)?,
    };
    recover_hook_from_store(store, checkpoint)
}

fn recover_hook_from_store(
    store: MemoryLedgerStore,
    checkpoint: Option<CheckpointConfig>,
) -> Result<AsterLedgerHook> {
    let appender = LedgerAppender::open(store, SystemClock)?;
    let hook = match checkpoint {
        Some(config) => DefaultLedgerHook::with_checkpoint_config(appender, config)?,
        None => DefaultLedgerHook::new(appender),
    };
    Ok(Mutex::new(hook))
}

fn recovered_ledger_store(recovery: &RecoveredBatches) -> Result<MemoryLedgerStore> {
    let mut store = MemoryLedgerStore::default();
    for batch in &recovery.batches {
        for row in &batch.rows {
            if row.cf == ColumnFamily::Ledger {
                store.insert_raw(parse_ledger_seq(&row.key)?, row.value.clone());
            }
        }
    }
    Ok(store)
}

fn physical_ledger_store(vault_dir: &Path) -> Result<Option<MemoryLedgerStore>> {
    let view = match AsterLedgerCfStore::open(vault_dir) {
        Ok(view) => view,
        Err(error)
            if error.code == "CALYX_LEDGER_CORRUPT"
                && error.message.contains("requires real Aster ledger state") =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let rows = view.scan()?;
    let anchor = view.head_anchor()?;
    if rows.is_empty() && anchor.is_none() {
        return Ok(None);
    }
    let mut store = MemoryLedgerStore::default();
    for row in rows {
        store.insert_raw(row.seq, row.bytes);
    }
    if let Some(anchor) = anchor {
        store.put_head_anchor(&anchor)?;
    }
    Ok(Some(store))
}

pub(super) fn lock_hook(hook: &AsterLedgerHook) -> Result<AsterLedgerHookGuard<'_>> {
    hook.lock()
        .map_err(|_| CalyxError::ledger_group_commit_failed("ledger hook lock poisoned"))
}

pub(super) fn refresh_hook(
    hook: &AsterLedgerHook,
    vault_dir: &Path,
    recovery: &RecoveredBatches,
    checkpoint: Option<CheckpointConfig>,
) -> Result<()> {
    let replacement = recover_hook_from_vault_dir(vault_dir, recovery, checkpoint)?
        .into_inner()
        .map_err(|_| CalyxError::ledger_group_commit_failed("new ledger hook lock poisoned"))?;
    let mut guard = lock_hook(hook)?;
    *guard = replacement;
    Ok(())
}

pub(super) fn stage_ingest(
    hook: &DefaultLedgerHook<MemoryLedgerStore, SystemClock>,
    rows: &mut Vec<WriteRow>,
    constellation: &Constellation,
) -> Result<Vec<StagedLedgerRow>> {
    stage_ingest_payload(
        hook,
        rows,
        constellation.cx_id,
        ingest_payload(constellation),
    )
}

pub(super) fn stage_ingest_payload(
    hook: &DefaultLedgerHook<MemoryLedgerStore, SystemClock>,
    rows: &mut Vec<WriteRow>,
    subject: calyx_core::CxId,
    payload: Vec<u8>,
) -> Result<Vec<StagedLedgerRow>> {
    stage_entry_payload(
        hook,
        rows,
        EntryKind::Ingest,
        SubjectId::Cx(subject),
        payload,
        ActorId::Service("calyx-aster".to_string()),
    )
}

pub(super) fn stage_entry_payload(
    hook: &DefaultLedgerHook<MemoryLedgerStore, SystemClock>,
    rows: &mut Vec<WriteRow>,
    kind: EntryKind,
    subject: SubjectId,
    payload: Vec<u8>,
    actor: ActorId,
) -> Result<Vec<StagedLedgerRow>> {
    let staged = hook.stage_with_checkpoints(kind, subject, payload, actor)?;
    for row in &staged {
        rows.push(WriteRow {
            cf: ColumnFamily::Ledger,
            key: row.key().to_vec(),
            value: row.value().to_vec(),
        });
    }
    Ok(staged)
}

pub(super) fn commit_staged(
    hook: &mut DefaultLedgerHook<MemoryLedgerStore, SystemClock>,
    staged: &[StagedLedgerRow],
) -> Result<LedgerRef> {
    let data_ref = staged
        .first()
        .ok_or_else(|| CalyxError::ledger_group_commit_failed("no staged ledger rows"))?
        .ledger_ref();
    for row in staged {
        hook.commit_staged(row)?;
    }
    Ok(data_ref)
}

fn ingest_payload(constellation: &Constellation) -> Vec<u8> {
    let mut payload = PayloadBuilder::default();
    let mut metadata = serde_json::Map::new();
    for key in [METADATA_CHUNK_ID, METADATA_DATABASE_NAME] {
        if let Some(value) = constellation.metadata.get(key) {
            metadata.insert(key.to_string(), json!(value));
        }
    }
    payload
        .insert_str("cx_id", constellation.cx_id.to_string())
        .insert_str("input_hash", hex(&constellation.input_ref.hash))
        .insert_value(
            "input_ref",
            json!({
                "hash": constellation.input_ref.hash,
                "redacted": true,
            }),
        )
        .insert_u64("ts", constellation.created_at);
    if !metadata.is_empty() {
        payload.insert_value("metadata", serde_json::Value::Object(metadata));
    }
    calyx_ledger::RedactionPolicy::default().apply_to_payload(&payload)
}

fn parse_ledger_seq(key: &[u8]) -> Result<u64> {
    let bytes: [u8; 8] = key
        .try_into()
        .map_err(|_| CalyxError::ledger_corrupt(format!("ledger key length {} != 8", key.len())))?;
    Ok(u64::from_be_bytes(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
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
                torn_tail: None,
                temporal_policy: None,
                dedup_policy: None,
                retention_horizon: crate::timetravel::RetentionHorizon::default(),
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
            torn_tail: None,
            temporal_policy: None,
            dedup_policy: None,
            retention_horizon: crate::timetravel::RetentionHorizon::default(),
        };

        let manifest_only_error = recover_hook(&gapped_recovery, None).unwrap_err();
        assert_eq!(manifest_only_error.code, "CALYX_LEDGER_CHAIN_BROKEN");
        let mut recovered =
            recover_hook_from_vault_dir(&dir, &gapped_recovery, None).expect("physical recovery");
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
        let Some(root) = std::env::var_os("CALYX_FSV_ROOT").map(PathBuf::from) else {
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
}
