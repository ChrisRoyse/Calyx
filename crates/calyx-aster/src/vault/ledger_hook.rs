use super::durable::RecoveredBatches;
use super::encode::WriteRow;
use crate::cf::ColumnFamily;
use crate::compaction::TieringPolicy;
use crate::ledger_view::{AsterLedgerCfStore, read_ledger_seqs_unlocked_with_tiering};
use calyx_core::{
    CalyxError, Constellation, LedgerRef, METADATA_CHUNK_ID, METADATA_DATABASE_NAME, Result,
    SystemClock,
};
use calyx_ledger::{
    ActorId, CheckpointConfig, CheckpointPayload, DefaultLedgerHook, EntryKind, LedgerAppender,
    LedgerCfStore, LedgerHeadAnchor, MemoryLedgerStore, PayloadBuilder, StagedLedgerRow, SubjectId,
    decode,
};
use serde_json::json;
use std::collections::BTreeSet;
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
    tiering_policy: Option<&TieringPolicy>,
) -> Result<AsterLedgerHook> {
    let store = match physical_ledger_store(
        vault_dir,
        LedgerViewLock::Acquire,
        checkpoint.as_ref(),
        tiering_policy,
    )? {
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

#[derive(Clone, Copy)]
enum LedgerViewLock {
    Acquire,
    AlreadyHeld,
}

fn physical_ledger_store(
    vault_dir: &Path,
    lock: LedgerViewLock,
    checkpoint: Option<&CheckpointConfig>,
    tiering_policy: Option<&TieringPolicy>,
) -> Result<Option<MemoryLedgerStore>> {
    let _commit_guard = match lock {
        LedgerViewLock::Acquire => Some(crate::file_lock::FileLockGuard::acquire(
            &durable_commit_lock_path(vault_dir),
        )?),
        LedgerViewLock::AlreadyHeld => None,
    };
    if let Some(anchor) = crate::ledger_head::read_head_anchor(vault_dir)? {
        return anchored_physical_ledger_store(vault_dir, &anchor, checkpoint, tiering_policy);
    }

    let view = match AsterLedgerCfStore::open_unlocked_with_tiering(vault_dir, tiering_policy) {
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
    if rows.is_empty() {
        return Ok(None);
    }
    let mut store = MemoryLedgerStore::default();
    for row in rows {
        store.insert_raw(row.seq, row.bytes);
    }
    Ok(Some(store))
}

fn anchored_physical_ledger_store(
    vault_dir: &Path,
    anchor: &LedgerHeadAnchor,
    checkpoint: Option<&CheckpointConfig>,
    tiering_policy: Option<&TieringPolicy>,
) -> Result<Option<MemoryLedgerStore>> {
    let mut store = MemoryLedgerStore::default();
    store.put_head_anchor(anchor)?;
    if anchor.height == 0 {
        return Ok(Some(store));
    }
    let start = match checkpoint {
        Some(config) => {
            checkpoint_hydration_start(vault_dir, anchor.height, config, tiering_policy)?
        }
        None => anchor.height - 1,
    };
    hydrate_physical_ledger_rows(vault_dir, start, anchor.height, &mut store, tiering_policy)?;
    Ok(Some(store))
}

fn checkpoint_hydration_start(
    vault_dir: &Path,
    head_height: u64,
    config: &CheckpointConfig,
    tiering_policy: Option<&TieringPolicy>,
) -> Result<u64> {
    if config.interval_entries == 0 {
        return Err(CalyxError::ledger_corrupt(
            "checkpoint interval_entries must be greater than zero",
        ));
    }
    if head_height == 0 {
        return Ok(0);
    }
    let scan_limit = config
        .interval_entries
        .saturating_mul(2)
        .saturating_add(16)
        .min(head_height);
    let start = head_height - scan_limit;
    let rows = read_physical_ledger_rows(vault_dir, start, head_height, tiering_policy)?;
    for row in rows.iter().rev() {
        let entry = decode(&row.bytes).map_err(|error| {
            CalyxError::ledger_corrupt(format!(
                "decode ledger row {} during checkpoint recovery: {error}",
                row.seq
            ))
        })?;
        if entry.kind == EntryKind::Admin
            && CheckpointPayload::decode_optional(&entry.payload)?.is_some()
        {
            return Ok(row.seq);
        }
    }
    if head_height <= config.interval_entries {
        return Ok(0);
    }
    Err(CalyxError {
        code: "CALYX_LEDGER_CHECKPOINT_RECOVERY_UNBOUNDED",
        message: format!(
            "anchored ledger head {head_height} has no checkpoint row in the last {scan_limit} rows; refusing full ledger hook scan during vault open"
        ),
        remediation: "rebuild or persist a ledger checkpoint pointer, then reopen the vault; do not bypass by disabling checkpoint recovery",
    })
}

fn hydrate_physical_ledger_rows(
    vault_dir: &Path,
    start: u64,
    end: u64,
    store: &mut MemoryLedgerStore,
    tiering_policy: Option<&TieringPolicy>,
) -> Result<()> {
    for row in read_physical_ledger_rows(vault_dir, start, end, tiering_policy)? {
        store.insert_raw(row.seq, row.bytes);
    }
    Ok(())
}

fn read_physical_ledger_rows(
    vault_dir: &Path,
    start: u64,
    end: u64,
    tiering_policy: Option<&TieringPolicy>,
) -> Result<Vec<calyx_ledger::LedgerRow>> {
    if start > end {
        return Err(CalyxError::ledger_corrupt(format!(
            "invalid physical ledger hydration range {start}..{end}"
        )));
    }
    let wanted = (start..end).collect::<BTreeSet<_>>();
    let rows = read_ledger_seqs_unlocked_with_tiering(vault_dir, &wanted, tiering_policy)?;
    let mut out = Vec::with_capacity(wanted.len());
    for seq in wanted {
        let row = rows.get(&seq).ok_or_else(|| {
            CalyxError::ledger_chain_broken(format!(
                "anchored physical ledger hydration missing seq {seq}"
            ))
        })?;
        out.push(row.clone());
    }
    Ok(out)
}

fn durable_commit_lock_path(vault_dir: &Path) -> std::path::PathBuf {
    vault_dir.join("locks").join("durable.commit.lock")
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
    tiering_policy: Option<&TieringPolicy>,
) -> Result<()> {
    let store = match physical_ledger_store(
        vault_dir,
        LedgerViewLock::AlreadyHeld,
        checkpoint.as_ref(),
        tiering_policy,
    )? {
        Some(store) => store,
        None => recovered_ledger_store(recovery)?,
    };
    let replacement = recover_hook_from_store(store, checkpoint)?
        .into_inner()
        .map_err(|_| CalyxError::ledger_group_commit_failed("new ledger hook lock poisoned"))?;
    let mut guard = lock_hook(hook)?;
    *guard = replacement;
    Ok(())
}

pub(super) fn refresh_hook_from_recovery(
    hook: &AsterLedgerHook,
    recovery: &RecoveredBatches,
    checkpoint: Option<CheckpointConfig>,
) -> Result<()> {
    let replacement = recover_hook_from_store(recovered_ledger_store(recovery)?, checkpoint)?
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

pub(super) fn ingest_payload(constellation: &Constellation) -> Vec<u8> {
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
mod tests;
