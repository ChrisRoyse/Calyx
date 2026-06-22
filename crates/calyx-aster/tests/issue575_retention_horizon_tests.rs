use calyx_aster::ledger_view::AsterLedgerCfStore;
use calyx_aster::manifest::ManifestStore;
use calyx_aster::timetravel::{CALYX_TIMETRAVEL_BEFORE_HORIZON, RetentionHorizon};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId, VaultStore,
};
use calyx_ledger::{EntryKind, LedgerCfStore, decode};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

mod fsv_support;
use fsv_support::prepared_temp_root;

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
}

fn test_dir(name: &str) -> PathBuf {
    prepared_temp_root("calyx-issue575", name)
}

fn constellation(vault: &AsterVault, input: &[u8], tag: f32) -> Constellation {
    let cx_id = vault.cx_id_for_input(input, 1);
    let mut input_hash = [0_u8; 32];
    input_hash[..input.len().min(32)].copy_from_slice(&input[..input.len().min(32)]);
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![tag, tag + 1.0],
        },
    );
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 10,
        input_ref: InputRef {
            hash: input_hash,
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [7; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

#[test]
fn durable_retention_horizon_persists_and_is_audited() {
    let root = test_dir("durable");
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue575-retention".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault");

    let c1 = vault
        .put(constellation(&vault, b"before-horizon", 1.0))
        .expect("ingest c1");
    vault
        .set_retention_horizon(RetentionHorizon::absolute(u64::MAX - 1))
        .expect("set high horizon");
    vault.flush().expect("flush durable state");

    let manifest = ManifestStore::open(&vault_dir)
        .load_current()
        .expect("read manifest");
    assert_eq!(
        manifest.retention_horizon,
        RetentionHorizon::absolute(u64::MAX - 1)
    );

    let before = vault.as_of(0).unwrap_err();
    assert_eq!(before.code, CALYX_TIMETRAVEL_BEFORE_HORIZON);
    assert!(
        before
            .message
            .contains("horizon_millis=18446744073709551614")
    );

    let reopened = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue575-retention".to_vec(),
        VaultOptions::default(),
    )
    .expect("reopen durable vault");
    assert_eq!(
        reopened.retention_horizon(),
        RetentionHorizon::absolute(u64::MAX - 1)
    );
    assert_eq!(
        reopened.as_of(0).unwrap_err().code,
        CALYX_TIMETRAVEL_BEFORE_HORIZON
    );

    reopened
        .set_retention_horizon(RetentionHorizon::absolute(0))
        .expect("lower horizon");
    let snapshot = reopened
        .as_of(u64::MAX)
        .expect("as_of after lowered horizon");
    assert!(snapshot.get_cx(c1).is_ok());

    let store = AsterLedgerCfStore::open(&vault_dir).expect("open ledger view");
    let rows = store.scan().expect("scan ledger");
    let entries = rows
        .iter()
        .map(|row| decode(&row.bytes).expect("decode ledger row"))
        .collect::<Vec<_>>();
    let retention_entries = entries
        .iter()
        .filter(|entry| {
            entry.kind == EntryKind::Admin
                && serde_json::from_slice::<Value>(&entry.payload)
                    .ok()
                    .and_then(|payload| payload.get("event").cloned())
                    == Some(Value::String("RETENTION_HORIZON_CHANGED".to_string()))
        })
        .collect::<Vec<_>>();
    assert_eq!(retention_entries.len(), 2);
    let payload: Value = serde_json::from_slice(&retention_entries[0].payload).unwrap();
    assert_eq!(payload["old"], serde_json::json!({"kind": "None"}));
    assert_eq!(
        payload["new"],
        serde_json::json!({"kind": "Absolute", "horizon_millis": u64::MAX - 1})
    );

    let _ = fs::remove_dir_all(&root);
}
