use super::{AsterVault, VaultOptions};
use crate::cf::{ColumnFamily, ledger_key};
use calyx_core::{CxFlags, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId, VaultStore};
use calyx_ledger::{ActorId, decode as decode_ledger};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[test]
#[ignore = "manual FSV for PH35 ledger actor and monotonic timestamp rows"]
fn ph35_actor_monotonic_ts_manual_fsv() {
    let root = fsv_root().join("actor-monotonic-ts");
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault");
    let before = vault
        .read_cf_at(0, ColumnFamily::Ledger, &ledger_key(0))
        .expect("read before");

    for seed in 0..3 {
        vault
            .put(sample_constellation(&vault, seed))
            .expect("durable ingest");
    }
    vault.flush().expect("flush vault");

    let snapshot = vault.snapshot();
    let mut entries = Vec::new();
    for seq in 0..3 {
        let row = vault
            .read_cf_at(snapshot, ColumnFamily::Ledger, &ledger_key(seq))
            .expect("read ledger row")
            .expect("ledger row present");
        let entry = decode_ledger(&row).expect("decode ledger entry");
        entries.push(serde_json::json!({
            "seq": entry.seq,
            "ts": entry.ts,
            "actor": actor_json(&entry.actor),
            "actor_non_empty": actor_non_empty(&entry.actor),
            "kind": entry.kind.as_str(),
            "hash": hex(&entry.entry_hash),
        }));
    }
    let ts = entries
        .iter()
        .map(|entry| entry["ts"].as_u64().unwrap())
        .collect::<Vec<_>>();
    let actor_non_empty = entries
        .iter()
        .all(|entry| entry["actor_non_empty"].as_bool().unwrap());
    let readback = serde_json::json!({
        "vault_root": vault_dir,
        "before_ledger_row_present": before.is_some(),
        "ledger_rows": entries,
        "timestamps_strictly_increase": ts.windows(2).all(|pair| pair[0] < pair[1]),
        "actors_non_empty": actor_non_empty,
        "snapshot": snapshot,
    });
    let readback_path = root.join("ledger-actor-ts-readback.json");
    fs::write(
        &readback_path,
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
    println!("PH35_ACTOR_TS_FSV_ROOT={}", root.display());
    println!("PH35_ACTOR_TS_READBACK={}", readback_path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(before, None);
    assert!(actor_non_empty);
    assert!(ts.windows(2).all(|pair| pair[0] < pair[1]));
}

fn sample_constellation(vault: &AsterVault, seed: u8) -> calyx_core::Constellation {
    let input = format!("ph35-actor-ts-{seed}");
    let cx_id = vault.cx_id_for_input(input.as_bytes(), 7);
    let mut input_hash = [0_u8; 32];
    input_hash[..input.len()].copy_from_slice(input.as_bytes());
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![0.25 + f32::from(seed), 0.75],
        },
    );
    calyx_core::Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 7,
        created_at: 1780940000 + u64::from(seed),
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some(format!("synthetic://ph35/actor-ts/{seed}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 9000 + u64::from(seed),
            hash: [seed; 32],
        },
        flags: CxFlags::default(),
    }
}

fn actor_json(actor: &ActorId) -> serde_json::Value {
    match actor {
        ActorId::Agent(value) => serde_json::json!({"agent": value}),
        ActorId::Service(value) => serde_json::json!({"service": value}),
        ActorId::System => serde_json::json!({"system": true}),
    }
}

fn actor_non_empty(actor: &ActorId) -> bool {
    match actor {
        ActorId::Agent(value) | ActorId::Service(value) => !value.is_empty(),
        ActorId::System => true,
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph35-actor-ts-fsv"))
}

fn reset_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).expect("create fsv dir");
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
