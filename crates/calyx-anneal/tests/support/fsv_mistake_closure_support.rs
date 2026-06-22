use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use calyx_anneal::{
    AnnealLedger, AsterAnnealLedgerStore, AsterHealthStore, AsterMistakeStorage,
    AsterReplayStorage, DegradeRegistry, FrozenLensGuard, MistakeLog, ReplayBuffer,
    decode_online_head,
};
use calyx_aster::cf::{ColumnFamily, full_content_hash, ledger_key};
use calyx_aster::vault::AsterVault;
use calyx_core::{Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef, Modality, Result};
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, decode as decode_ledger};
use calyx_registry::{AlgorithmicLens, Registry};
use serde_json::{Value, json};

#[allow(clippy::duplicate_mod)]
#[path = "../fsv_support/mod.rs"]
mod fsv_support;

pub const TEST_TS: u64 = 1_785_500_412;

pub fn mistake_log(
    vault: &AsterVault,
) -> MistakeLog<AsterMistakeStorage<'_, calyx_core::SystemClock>> {
    MistakeLog::open(
        AsterMistakeStorage::new(vault),
        128,
        Arc::new(FixedClock::new(TEST_TS)),
    )
    .unwrap()
}

pub fn replay_buffer(
    vault: &AsterVault,
    capacity: usize,
) -> ReplayBuffer<AsterReplayStorage<'_, calyx_core::SystemClock>> {
    ReplayBuffer::open(
        AsterReplayStorage::new(vault),
        capacity,
        Arc::new(FixedClock::new(TEST_TS)),
    )
    .unwrap()
}

pub fn health_registry(
    vault: &AsterVault,
) -> DegradeRegistry<AsterHealthStore<'_, calyx_core::SystemClock>> {
    DegradeRegistry::open(
        Arc::new(FixedClock::new(TEST_TS)),
        AsterHealthStore::new(vault),
    )
    .unwrap()
}

pub fn anneal_ledger(
    vault: &AsterVault,
    clock: FixedClock,
) -> Result<AnnealLedger<AsterAnnealLedgerStore<'_, calyx_core::SystemClock>, FixedClock>> {
    let appender = LedgerAppender::open(AsterAnnealLedgerStore::new(vault), clock)?;
    AnnealLedger::new(appender, ActorId::Service("calyx-anneal-fsv".to_string()))
}

pub fn frozen_guard(label: &str) -> Result<FrozenLensGuard<Registry>> {
    let mut registry = Registry::new();
    let lens = AlgorithmicLens::byte_features(label, Modality::Text);
    registry.register_frozen(lens.clone(), lens.contract().clone())?;
    let mut guard = FrozenLensGuard::new(Arc::new(registry));
    guard.initialize()?;
    Ok(guard)
}

pub fn decoded_heads(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealHeads)
        .unwrap()
        .into_iter()
        .map(|(key, bytes)| {
            json!({
                "key_hex": hex(&key),
                "value_len": bytes.len(),
                "value_hex": hex(&bytes),
                "head": decode_online_head(&bytes).unwrap()
            })
        })
        .collect()
}

pub fn cf_rows(vault: &AsterVault, cf: ColumnFamily) -> Vec<Value> {
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

pub fn cf_snapshot(vault: &AsterVault) -> Value {
    json!({
        "mistakes": cf_rows(vault, ColumnFamily::AnnealMistakes),
        "replay": cf_rows(vault, ColumnFamily::AnnealReplay),
        "heads": cf_rows(vault, ColumnFamily::AnnealHeads),
        "health": cf_rows(vault, ColumnFamily::AnnealHealth),
        "ledger": ledger_rows(vault)
    })
}

pub fn cf_hashes(vault: &AsterVault) -> Value {
    json!({
        "base": cf_hash(vault, ColumnFamily::Base),
        "mistakes": cf_hash(vault, ColumnFamily::AnnealMistakes),
        "replay": cf_hash(vault, ColumnFamily::AnnealReplay),
        "heads": cf_hash(vault, ColumnFamily::AnnealHeads),
        "ledger": cf_hash(vault, ColumnFamily::Ledger)
    })
}

pub fn has_ledger_action(rows: &[Value], action: &str, description_contains: &str) -> bool {
    rows.iter().any(|row| {
        row["payload_json"]["action"].as_str() == Some(action)
            && row["payload_json"]["description"]
                .as_str()
                .is_some_and(|description| description.contains(description_contains))
    })
}

pub fn ledger_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .unwrap()
        .into_iter()
        .map(|(key, value)| {
            let entry = decode_ledger(&value).unwrap();
            assert_eq!(key, ledger_key(entry.seq));
            json!({
                "seq": entry.seq,
                "key_hex": hex(&key),
                "value_len": value.len(),
                "value_hex": hex(&value),
                "kind": entry.kind.as_str(),
                "is_anneal": entry.kind == EntryKind::Anneal,
                "entry_hash": hex(&entry.entry_hash),
                "payload_json": serde_json::from_slice::<Value>(&entry.payload).ok(),
                "payload_hex": hex(&entry.payload)
            })
        })
        .collect()
}

pub fn write_json<T: serde::Serialize>(root: &Path, name: &str, value: &T) {
    fs::write(root.join(name), serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

pub fn cx(seed: u8) -> Constellation {
    Constellation {
        cx_id: CxId::from_bytes([seed; 16]),
        vault_id: fsv_support::vault_id(),
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

fn cf_hash(vault: &AsterVault, cf: ColumnFamily) -> String {
    let mut parts = Vec::new();
    for (key, value) in vault.scan_cf_at(vault.latest_seq(), cf).unwrap() {
        parts.push(key);
        parts.push(value);
    }
    hex(&full_content_hash(parts.iter().map(Vec::as_slice)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
