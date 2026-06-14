//! Durable reactive rows stored in Aster's `reactive` CF.

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, CxId, LedgerRef, Result};
use calyx_ledger::{ActorId, EntryKind, RedactionPolicy, SubjectId};
use serde_json::json;
use uuid::Uuid;

use super::{
    AuditEntry, ReactiveEngine, ReactiveSignals, TriggerFired, TriggerId, queue_full_error,
};
use crate::error::{CALYX_REACTIVE_QUEUE_FULL, CALYX_REACTIVE_ROW_CORRUPT, loom_error};

const AUDIT_TAG: u8 = 0x01;
const FIRED_TAG: u8 = 0x02;
const TRIGGER_ID_LEN: usize = 16;
const LEDGER_SEQ_LEN: usize = 8;
const TAIL_ID_LEN: usize = 16;
const REACTIVE_KEY_LEN: usize = 1 + TRIGGER_ID_LEN + LEDGER_SEQ_LEN + TAIL_ID_LEN;
const REACTIVE_LEDGER_TAG: &str = "reactive_state_v1";

/// Canonical row type inside the `reactive` CF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReactiveRowKind {
    Audit,
    Fired,
}

/// Parsed durable reactive CF key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactiveRowKey {
    pub kind: ReactiveRowKind,
    pub trigger_id: TriggerId,
    pub ledger_seq: u64,
    pub tail_id: [u8; TAIL_ID_LEN],
}

pub fn reactive_audit_key(entry: &AuditEntry) -> Vec<u8> {
    build_key(
        AUDIT_TAG,
        entry.trigger_id,
        entry.ledger_ref.seq,
        *entry.eval_id.as_bytes(),
    )
}

pub fn reactive_fired_key(event: &TriggerFired) -> Vec<u8> {
    build_key(
        FIRED_TAG,
        event.trigger_id,
        event.ledger_ref.seq,
        *event.cx_id.as_bytes(),
    )
}

pub fn reactive_audit_prefix(trigger_id: TriggerId) -> Vec<u8> {
    let mut key = Vec::with_capacity(1 + TRIGGER_ID_LEN);
    key.push(AUDIT_TAG);
    key.extend_from_slice(trigger_id.as_bytes());
    key
}

pub fn reactive_row_key(key: &[u8]) -> Result<ReactiveRowKey> {
    if key.len() != REACTIVE_KEY_LEN {
        return Err(reactive_row_error(format!(
            "reactive CF key length {} != {REACTIVE_KEY_LEN}",
            key.len()
        )));
    }
    let kind = match key[0] {
        AUDIT_TAG => ReactiveRowKind::Audit,
        FIRED_TAG => ReactiveRowKind::Fired,
        tag => {
            return Err(reactive_row_error(format!(
                "unknown reactive row tag {tag}"
            )));
        }
    };
    let trigger_id = Uuid::from_bytes(key[1..17].try_into().expect("slice len"));
    let ledger_seq = u64::from_be_bytes(key[17..25].try_into().expect("slice len"));
    let tail_id = key[25..41].try_into().expect("slice len");
    Ok(ReactiveRowKey {
        kind,
        trigger_id,
        ledger_seq,
        tail_id,
    })
}

pub fn decode_audit_entry(bytes: &[u8]) -> Result<AuditEntry> {
    serde_json::from_slice(bytes)
        .map_err(|error| reactive_row_error(format!("decode durable reactive audit row: {error}")))
}

pub fn decode_trigger_fired(bytes: &[u8]) -> Result<TriggerFired> {
    serde_json::from_slice(bytes)
        .map_err(|error| reactive_row_error(format!("decode durable reactive fired row: {error}")))
}

impl ReactiveEngine {
    /// Evaluates triggers and persists each audit/fired row to the vault's
    /// `reactive` CF. Queue-full warnings are persisted before the overflow
    /// error is returned.
    pub fn evaluate_post_ingest_durable<C, S>(
        &mut self,
        vault: &AsterVault<C>,
        cx_id: CxId,
        ingest_ledger_ref: LedgerRef,
        signals: &S,
    ) -> Result<usize>
    where
        C: Clock,
        S: ReactiveSignals,
    {
        let now = self.clock.now();
        let mut fired = 0usize;
        let mut overflow = None;
        let mut rows = Vec::new();
        let mut counts = ReactivePersistCounts::default();

        for def in self.registry.defs().to_vec() {
            let matched = match self.evaluate_condition(&def, cx_id, signals) {
                Ok(matched) => matched,
                Err(error) => {
                    persist_rows(vault, rows, cx_id, &ingest_ledger_ref, counts)?;
                    return Err(error);
                }
            };
            let audit = AuditEntry {
                eval_id: Uuid::now_v7(),
                trigger_id: def.id,
                cx_id,
                matched,
                ts: now,
                ledger_ref: ingest_ledger_ref.clone(),
                code: None,
            };
            self.audit_log.append(audit.clone());
            push_audit_row(&mut rows, &audit)?;
            counts.audit_rows += 1;
            if !matched {
                continue;
            }
            fired += 1;
            let event = TriggerFired {
                trigger_id: def.id,
                cx_id,
                fired_at: now,
                ledger_ref: ingest_ledger_ref.clone(),
                condition_snapshot: def.condition.clone(),
            };
            self.dispatch_to_subscriptions(&event);
            if let Some(dropped) = self.queue.push(event.clone()) {
                let warning = AuditEntry {
                    eval_id: Uuid::now_v7(),
                    trigger_id: def.id,
                    cx_id,
                    matched: true,
                    ts: now,
                    ledger_ref: ingest_ledger_ref.clone(),
                    code: Some(CALYX_REACTIVE_QUEUE_FULL.to_string()),
                };
                self.audit_log.append(warning.clone());
                push_audit_row(&mut rows, &warning)?;
                counts.audit_rows += 1;
                counts.warning_rows += 1;
                overflow.get_or_insert_with(|| queue_full_error(&dropped));
            }
            push_fired_row(&mut rows, &event)?;
            counts.fired_rows += 1;
        }
        persist_rows(vault, rows, cx_id, &ingest_ledger_ref, counts)?;
        match overflow {
            Some(error) => Err(error),
            None => Ok(fired),
        }
    }
}

fn build_key(
    tag: u8,
    trigger_id: TriggerId,
    ledger_seq: u64,
    tail_id: [u8; TAIL_ID_LEN],
) -> Vec<u8> {
    let mut key = Vec::with_capacity(REACTIVE_KEY_LEN);
    key.push(tag);
    key.extend_from_slice(trigger_id.as_bytes());
    key.extend_from_slice(&ledger_seq.to_be_bytes());
    key.extend_from_slice(&tail_id);
    key
}

#[derive(Clone, Copy, Default)]
struct ReactivePersistCounts {
    audit_rows: u64,
    fired_rows: u64,
    warning_rows: u64,
}

fn push_audit_row(
    rows: &mut Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>,
    entry: &AuditEntry,
) -> Result<()> {
    rows.push((
        ColumnFamily::Reactive,
        reactive_audit_key(entry),
        serde_json::to_vec(entry)
            .map_err(|error| reactive_row_error(format!("encode reactive audit row: {error}")))?,
    ));
    Ok(())
}

fn push_fired_row(
    rows: &mut Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>,
    event: &TriggerFired,
) -> Result<()> {
    rows.push((
        ColumnFamily::Reactive,
        reactive_fired_key(event),
        serde_json::to_vec(event)
            .map_err(|error| reactive_row_error(format!("encode reactive fired row: {error}")))?,
    ));
    Ok(())
}

fn persist_rows<C: Clock>(
    vault: &AsterVault<C>,
    rows: Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>,
    cx_id: CxId,
    ingest_ledger_ref: &LedgerRef,
    counts: ReactivePersistCounts,
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    vault
        .write_cf_batch_with_ledger_entry(
            rows,
            EntryKind::Guard,
            SubjectId::Cx(cx_id),
            reactive_ledger_payload(cx_id, ingest_ledger_ref, counts)?,
            ActorId::Service("calyx-loom-reactive".to_string()),
        )
        .map(|_| ())
}

fn reactive_ledger_payload(
    cx_id: CxId,
    ingest_ledger_ref: &LedgerRef,
    counts: ReactivePersistCounts,
) -> Result<Vec<u8>> {
    let payload = serde_json::to_vec(&json!({
        "tag": REACTIVE_LEDGER_TAG,
        "cx_id": cx_id.to_string(),
        "ingest_ledger_seq": ingest_ledger_ref.seq,
        "ingest_ledger_hash": hex(&ingest_ledger_ref.hash),
        "row_count": counts.audit_rows + counts.fired_rows,
        "audit_count": counts.audit_rows,
        "fired_count": counts.fired_rows,
        "warning_count": counts.warning_rows,
    }))
    .map_err(|error| reactive_row_error(format!("encode reactive ledger payload: {error}")))?;
    RedactionPolicy::check_payload(&payload)?;
    Ok(payload)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn reactive_row_error(message: impl Into<String>) -> calyx_core::CalyxError {
    loom_error(CALYX_REACTIVE_ROW_CORRUPT, message)
}
