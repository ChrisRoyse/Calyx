use super::{AsterVault, VaultOptions, encode};
use crate::cf::{ColumnFamily, ledger_key};
use calyx_core::{
    Constellation, CxFlags, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId, VaultStore,
};
use calyx_ledger::{CheckpointConfig, CheckpointPayload, EntryKind, decode as decode_ledger};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn durable_vault_writes_periodic_checkpoint_admin_rows_in_same_batch() {
    let dir = test_dir("ledger-checkpoints");
    reset_dir(&dir);
    let options = VaultOptions {
        ledger_checkpoint: Some(CheckpointConfig::new(2)),
        ..VaultOptions::default()
    };
    let vault = AsterVault::new_durable(&dir, vault_id(), b"salt".to_vec(), options)
        .expect("open durable vault");

    let ids = (0..5)
        .map(|seed| {
            let cx = sample_constellation(&vault, seed);
            let id = cx.cx_id;
            vault.put(cx).expect("put");
            id
        })
        .collect::<Vec<_>>();
    vault.flush().expect("flush");

    let entries = read_entries(&vault, 0..7);
    let checkpoints = checkpoint_payloads(&entries);
    let wal = inspect_wal(&dir);
    let got = vault.get(ids[2], vault.snapshot()).expect("get third");

    assert_eq!(got.provenance.seq, 3);
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::Ingest)
            .count(),
        5
    );
    assert_eq!(checkpoints.len(), 2);
    assert_eq!(checkpoints[0].range_start, 0);
    assert_eq!(checkpoints[0].range_end, 2);
    assert_eq!(checkpoints[1].range_start, 3);
    assert_eq!(checkpoints[1].range_end, 5);
    assert_eq!(wal.records_with_two_ledger_rows, 2);
    assert_eq!(wal.records_with_base, 5);
    assert_eq!(wal.ledger_rows_before_base, 5);

    cleanup(dir);
}

fn read_entries(vault: &AsterVault, range: std::ops::Range<u64>) -> Vec<calyx_ledger::LedgerEntry> {
    range
        .map(|seq| {
            let row = vault
                .read_cf_at(vault.snapshot(), ColumnFamily::Ledger, &ledger_key(seq))
                .expect("read ledger")
                .expect("ledger row present");
            decode_ledger(&row).expect("decode ledger")
        })
        .collect()
}

fn checkpoint_payloads(entries: &[calyx_ledger::LedgerEntry]) -> Vec<CheckpointPayload> {
    entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Admin)
        .map(|entry| CheckpointPayload::decode(&entry.payload).expect("decode checkpoint"))
        .collect()
}

#[derive(Default)]
struct WalInspection {
    records_with_base: usize,
    records_with_two_ledger_rows: usize,
    ledger_rows_before_base: usize,
}

fn inspect_wal(dir: &Path) -> WalInspection {
    let replay = crate::wal::replay_dir(dir.join("wal")).expect("replay wal");
    let mut out = WalInspection::default();
    for record in replay.records {
        let rows = encode::decode_write_batch(&record.payload).expect("decode batch");
        let ledger_indexes = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.cf == ColumnFamily::Ledger)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let base_index = rows.iter().position(|row| row.cf == ColumnFamily::Base);
        if base_index.is_some() {
            out.records_with_base += 1;
        }
        if ledger_indexes.len() == 2 {
            out.records_with_two_ledger_rows += 1;
        }
        if let (Some(first_ledger), Some(base)) = (ledger_indexes.first(), base_index)
            && *first_ledger < base
        {
            out.ledger_rows_before_base += 1;
        }
    }
    out
}

fn sample_constellation(vault: &AsterVault, seed: u16) -> Constellation {
    let input = format!("ph36-checkpoint-{seed}");
    let cx_id = vault.cx_id_for_input(input.as_bytes(), 7);
    let mut input_hash = [0_u8; 32];
    input_hash[..input.len()].copy_from_slice(input.as_bytes());
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![f32::from(seed), 0.25],
        },
    );
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 7,
        created_at: 1_785_300_000 + u64::from(seed),
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some(format!("synthetic://ph36/checkpoint/{seed}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 99,
            hash: [9; 32],
        },
        flags: CxFlags::default(),
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn test_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("calyx-aster-{name}-{}", std::process::id()))
}

fn reset_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).unwrap();
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).unwrap();
}
