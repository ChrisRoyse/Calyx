use super::{AsterVault, VaultOptions, encode};
use crate::cf::{ColumnFamily, ledger_key};
use calyx_core::{CxFlags, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId, VaultStore};
use calyx_ledger::{EntryKind, SubjectId, decode as decode_ledger};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[test]
#[ignore = "manual FSV for PH35 ledger integration smoke"]
fn ph35_ledger_integration_smoke_manual_fsv() {
    let root = fsv_root().join("ledger-integration-smoke");
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
    let ids = ingest_unique(&vault, 100);
    vault.flush().expect("flush durable vault");

    let snapshot = vault.snapshot();
    let entries = read_ledger_entries(&vault, snapshot, &ids);
    let replay = crate::wal::replay_dir(vault_dir.join("wal")).expect("replay wal");
    let wal = inspect_wal(&replay.records);
    let no_secrets = entries
        .iter()
        .all(|entry| payload_has_no_secret(&entry.payload));
    let chain_ok = chain_links(&entries);
    let first_five = entries
        .iter()
        .take(5)
        .map(|entry| {
            serde_json::json!({
                "seq": entry.seq,
                "prev_hash": hex(&entry.prev_hash),
                "entry_hash": hex(&entry.entry_hash),
                "kind": entry.kind.as_str(),
            })
        })
        .collect::<Vec<_>>();
    let readback = serde_json::json!({
        "before_ledger_row_present": before.is_some(),
        "snapshot": snapshot,
        "ledger_entry_count": entries.len(),
        "chain_ok": chain_ok,
        "no_secret_payloads": no_secrets,
        "wal_record_count": replay.records.len(),
        "wal_records_with_ledger": wal.records_with_ledger,
        "wal_records_with_base": wal.records_with_base,
        "wal_ledger_rows_before_base": wal.ledger_before_base,
        "wal_ledger_count": wal.ledger_count,
        "wal_base_count": wal.base_count,
        "first_five": first_five,
        "vault_root": vault_dir,
    });
    let readback_path = root.join("ledger-integration-smoke-readback.json");
    fs::write(
        &readback_path,
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();

    println!("PH35_INTEGRATION_FSV_ROOT={}", root.display());
    println!("PH35_INTEGRATION_READBACK={}", readback_path.display());
    println!("chain OK: 100 entries, all links verified");
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(before, None);
    assert_eq!(entries.len(), 100);
    assert!(chain_ok);
    assert!(no_secrets);
    assert_eq!(wal.ledger_count, 100);
    assert_eq!(wal.base_count, 100);
    assert_eq!(wal.records_with_ledger, 100);
    assert_eq!(wal.records_with_base, 100);
    assert_eq!(wal.ledger_before_base, 100);
}

fn ingest_unique(vault: &AsterVault, n: usize) -> Vec<calyx_core::CxId> {
    (0..n)
        .map(|seed| {
            let cx = sample_constellation(vault, seed as u16);
            let id = cx.cx_id;
            vault.put(cx).expect("put constellation");
            id
        })
        .collect()
}

fn read_ledger_entries(
    vault: &AsterVault,
    snapshot: u64,
    ids: &[calyx_core::CxId],
) -> Vec<calyx_ledger::LedgerEntry> {
    ids.iter()
        .enumerate()
        .map(|(seq, id)| {
            let row = vault
                .read_cf_at(snapshot, ColumnFamily::Ledger, &ledger_key(seq as u64))
                .expect("read ledger row")
                .expect("ledger row present");
            let entry = decode_ledger(&row).expect("decode ledger entry");
            let got = vault.get(*id, snapshot).expect("get constellation");
            assert_eq!(entry.seq, seq as u64);
            assert_eq!(entry.kind, EntryKind::Ingest);
            assert!(matches!(entry.subject, SubjectId::Cx(value) if value == *id));
            assert_eq!(got.provenance.seq, entry.seq);
            assert_eq!(got.provenance.hash, entry.entry_hash);
            entry
        })
        .collect()
}

#[derive(Default)]
struct WalInspection {
    ledger_count: usize,
    base_count: usize,
    records_with_ledger: usize,
    records_with_base: usize,
    ledger_before_base: usize,
}

fn inspect_wal(records: &[crate::wal::ReplayRecord]) -> WalInspection {
    let mut out = WalInspection::default();
    for record in records {
        let rows = encode::decode_write_batch(&record.payload).expect("decode wal batch");
        let ledger_index = rows.iter().position(|row| row.cf == ColumnFamily::Ledger);
        let base_index = rows.iter().position(|row| row.cf == ColumnFamily::Base);
        out.ledger_count += rows
            .iter()
            .filter(|row| row.cf == ColumnFamily::Ledger)
            .count();
        out.base_count += rows
            .iter()
            .filter(|row| row.cf == ColumnFamily::Base)
            .count();
        if ledger_index.is_some() {
            out.records_with_ledger += 1;
        }
        if base_index.is_some() {
            out.records_with_base += 1;
        }
        if matches!((ledger_index, base_index), (Some(ledger), Some(base)) if ledger < base) {
            out.ledger_before_base += 1;
        }
    }
    out
}

fn chain_links(entries: &[calyx_ledger::LedgerEntry]) -> bool {
    entries
        .first()
        .is_some_and(|entry| entry.prev_hash == [0; 32])
        && entries
            .windows(2)
            .all(|pair| pair[1].prev_hash == pair[0].entry_hash)
}

fn payload_has_no_secret(payload: &[u8]) -> bool {
    let lower = String::from_utf8_lossy(payload).to_ascii_lowercase();
    !["secret", "password", "token"]
        .iter()
        .any(|needle| lower.contains(needle))
}

fn sample_constellation(vault: &AsterVault, seed: u16) -> calyx_core::Constellation {
    let input = format!("ph35-integration-{seed:03}");
    let cx_id = vault.cx_id_for_input(input.as_bytes(), 7);
    let mut input_hash = [0_u8; 32];
    input_hash[..input.len()].copy_from_slice(input.as_bytes());
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![f32::from(seed) + 0.25, 0.75],
        },
    );
    calyx_core::Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 7,
        created_at: 1_785_000_000 + u64::from(seed),
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some(format!("synthetic://ph35/integration/{seed}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 9000 + u64::from(seed),
            hash: [seed as u8; 32],
        },
        flags: CxFlags::default(),
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph35-integration-fsv"))
}

fn reset_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).expect("create fsv dir");
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
