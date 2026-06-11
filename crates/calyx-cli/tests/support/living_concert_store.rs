use std::collections::BTreeMap;

use calyx_anneal::AsterAnnealLedgerStore;
use calyx_aster::cf::{
    ColumnFamily, OnlineKeyKind, anchor_key, base_key, ledger_key, online_key, slot_key,
};
use calyx_aster::recurrence::OccurrenceContext;
use calyx_aster::vault::{AsterVault, encode};
use calyx_core::{
    Anchor, Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef, Modality, SlotId,
    SlotVector, VaultId,
};
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, SubjectId};
use calyx_registry::Input;
use serde_json::Value;

const ACTOR: &str = "calyx-living-concert-fsv";

pub fn write_constellation(vault: &AsterVault, mut cx: Constellation, payload: Value, ts: u64) {
    let (seq, bytes, ledger_ref) = prepared_ledger(
        vault,
        EntryKind::Ingest,
        SubjectId::Cx(cx.cx_id),
        payload,
        ts,
    );
    cx.provenance = ledger_ref;
    let mut rows = vec![(ColumnFamily::Ledger, ledger_key(seq), bytes)];
    rows.push((
        ColumnFamily::Base,
        base_key(cx.cx_id),
        encode::encode_constellation_base(&cx).unwrap(),
    ));
    for (slot_id, vector) in &cx.slots {
        rows.push((
            ColumnFamily::slot(*slot_id),
            slot_key(cx.cx_id),
            encode::encode_slot_vector(vector).unwrap(),
        ));
    }
    for anchor in &cx.anchors {
        rows.push((
            ColumnFamily::Anchors,
            anchor_key(cx.cx_id, &anchor.kind),
            encode::encode_anchor(anchor).unwrap(),
        ));
    }
    vault
        .write_cf_batch(rows)
        .expect("write constellation batch");
}

pub fn write_event_with_rows(
    vault: &AsterVault,
    kind: EntryKind,
    subject: SubjectId,
    payload: Value,
    mut rows: Vec<(ColumnFamily, Vec<u8>, Vec<u8>)>,
    ts: u64,
) -> LedgerRef {
    let (seq, bytes, ledger_ref) = prepared_ledger(vault, kind, subject, payload, ts);
    rows.insert(0, (ColumnFamily::Ledger, ledger_key(seq), bytes));
    vault.write_cf_batch(rows).expect("write event batch");
    ledger_ref
}

pub fn append_event(
    vault: &AsterVault,
    kind: EntryKind,
    subject: SubjectId,
    payload: Value,
    ts: u64,
) -> LedgerRef {
    write_event_with_rows(vault, kind, subject, payload, Vec::new(), ts)
}

pub fn constellation(
    vault_id: VaultId,
    cx_id: CxId,
    panel: u32,
    input: &Input,
    slots: BTreeMap<SlotId, SlotVector>,
    anchors: Vec<Anchor>,
    flags: CxFlags,
) -> Constellation {
    Constellation {
        cx_id,
        vault_id,
        panel_version: panel,
        created_at: crate::support::living_concert::START_TS,
        input_ref: InputRef {
            hash: *blake3::hash(&input.bytes).as_bytes(),
            pointer: input.pointer.clone(),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors,
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags,
    }
}

pub fn online_row(id: u64, value: Value) -> (ColumnFamily, Vec<u8>, Vec<u8>) {
    (
        ColumnFamily::Online,
        online_key(OnlineKeyKind::HeadState, id),
        serde_json::to_vec(&value).unwrap(),
    )
}

pub fn ctx(value: &str) -> OccurrenceContext {
    OccurrenceContext::new(value.as_bytes().to_vec()).expect("occurrence context")
}

pub fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}

pub fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}

fn prepared_ledger(
    vault: &AsterVault,
    kind: EntryKind,
    subject: SubjectId,
    payload: Value,
    ts: u64,
) -> (u64, Vec<u8>, LedgerRef) {
    let appender =
        LedgerAppender::open(AsterAnnealLedgerStore::new(vault), FixedClock::new(ts)).unwrap();
    let prepared = appender
        .prepare(
            kind,
            subject,
            serde_json::to_vec(&payload).unwrap(),
            ActorId::Service(ACTOR.to_string()),
        )
        .unwrap();
    (
        prepared.seq(),
        prepared.bytes().to_vec(),
        prepared.ledger_ref(),
    )
}
