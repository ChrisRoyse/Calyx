use super::{AsterVault, VaultOptions, VaultRecoveryReport, durable, encode, ledger_hook};
use crate::cf::{CfRouter, ColumnFamily, base_key, ledger_key};
use crate::dedup::DedupPolicy;
use crate::mvcc::VersionedCfStore;
use calyx_core::{
    Clock, CxFlags, FixedClock, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId,
    VaultStore,
};
use calyx_ledger::{EntryKind, LedgerCfStore, SubjectId, decode as decode_ledger};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn ledger_hook_stays_unadvanced_when_vault_commit_fails() {
    let dir = test_dir("ledger-atomic-router-failure");
    let vault = router_failure_vault(&dir);
    let cx = sample_constellation(&vault, "failure-unit", 0);

    let before = read_ledger_row(&vault, 0);
    let error = vault.put(cx).expect_err("router cap rejects commit");
    let after = read_ledger_row(&vault, 0);
    let physical = CfRouter::open(&dir, 1)
        .unwrap()
        .iter_cf(ColumnFamily::Ledger)
        .unwrap();
    let hook = hook_state(&vault);

    assert_eq!(error.code, "CALYX_BACKPRESSURE");
    assert_eq!(before, None);
    assert_eq!(after, None);
    assert!(physical.is_empty());
    assert_eq!(vault.snapshot(), 0);
    assert_eq!(hook.next_seq, 0);
    assert_eq!(hook.store_rows, 0);
    cleanup(dir);
}

#[test]
#[ignore = "manual FSV for PH35 group-commit atomicity"]
fn ph35_group_commit_atomicity_manual_fsv() {
    let root = fsv_root().join("group-commit-atomicity");
    reset_dir(&root);

    let failure_dir = root.join("failure").join("vault");
    let failure_vault = router_failure_vault(&failure_dir);
    let failure_cx = sample_constellation(&failure_vault, "failure", 1);
    let failure_before = read_ledger_row(&failure_vault, 0);
    let failure_hook_before = hook_state(&failure_vault);
    let failure_error = failure_vault
        .put(failure_cx)
        .expect_err("router cap rejects commit");
    let failure_after = read_ledger_row(&failure_vault, 0);
    let failure_physical_rows = CfRouter::open(&failure_dir, 1)
        .unwrap()
        .iter_cf(ColumnFamily::Ledger)
        .unwrap();
    let failure_hook_after = hook_state(&failure_vault);

    let success_dir = root.join("success").join("vault");
    let success_vault = AsterVault::new_durable(
        &success_dir,
        vault_id(),
        b"salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable success vault");
    let success_cx = sample_constellation(&success_vault, "success", 2);
    let success_id = success_cx.cx_id;
    let success_before = read_ledger_row(&success_vault, 0);
    let success_hook_before = hook_state(&success_vault);
    success_vault.put(success_cx).expect("success put");
    success_vault.flush().expect("flush success vault");

    let success_after = read_ledger_row(&success_vault, success_vault.snapshot())
        .expect("ledger row after success");
    let got = success_vault
        .get(success_id, success_vault.snapshot())
        .expect("read success constellation");
    let replay = crate::wal::replay_dir(success_dir.join("wal")).expect("replay success WAL");
    let wal_rows = encode::decode_write_batch(&replay.records[0].payload).expect("decode WAL");
    let ledger_index = row_index(&wal_rows, ColumnFamily::Ledger);
    let base_index = row_index(&wal_rows, ColumnFamily::Base);
    let ledger_entry = decode_ledger(&wal_rows[ledger_index].value).expect("decode ledger");
    let success_hook_after = hook_state(&success_vault);

    let readback = serde_json::json!({
        "failure": {
            "before_ledger_row_present": failure_before.is_some(),
            "error_code": failure_error.code,
            "after_ledger_row_present": failure_after.is_some(),
            "physical_ledger_rows_after": failure_physical_rows.len(),
            "snapshot_after": failure_vault.snapshot(),
            "hook_before": failure_hook_before.to_json(),
            "hook_after": failure_hook_after.to_json(),
        },
        "success": {
            "before_ledger_row_present": success_before.is_some(),
            "after_ledger_row_present": true,
            "ledger_cf_matches_wal_row": success_after == wal_rows[ledger_index].value,
            "wal_record_seq": replay.records[0].seq,
            "ledger_row_index": ledger_index,
            "base_row_index": base_index,
            "ledger_before_base": ledger_index < base_index,
            "base_row_present": success_vault.read_cf_at(
                success_vault.snapshot(),
                ColumnFamily::Base,
                &base_key(success_id),
            ).unwrap().is_some(),
            "entry": {
                "seq": ledger_entry.seq,
                "prev_hash": hex(&ledger_entry.prev_hash),
                "entry_hash": hex(&ledger_entry.entry_hash),
                "kind": ledger_entry.kind.as_str(),
                "subject_is_cx": matches!(ledger_entry.subject, SubjectId::Cx(value) if value == success_id),
            },
            "stored_constellation_provenance": {
                "seq": got.provenance.seq,
                "hash": hex(&got.provenance.hash),
            },
            "hook_before": success_hook_before.to_json(),
            "hook_after": success_hook_after.to_json(),
        }
    });
    let readback_path = root.join("group-commit-atomicity-readback.json");
    fs::write(
        &readback_path,
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();

    println!("PH35_GROUP_COMMIT_ATOMICITY_FSV_ROOT={}", root.display());
    println!(
        "PH35_GROUP_COMMIT_ATOMICITY_READBACK={}",
        readback_path.display()
    );
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(failure_before, None);
    assert_eq!(failure_error.code, "CALYX_BACKPRESSURE");
    assert_eq!(failure_after, None);
    assert!(failure_physical_rows.is_empty());
    assert_eq!(failure_vault.snapshot(), 0);
    assert_eq!(failure_hook_before.next_seq, 0);
    assert_eq!(failure_hook_after.next_seq, 0);
    assert_eq!(failure_hook_after.store_rows, 0);
    assert_eq!(success_before, None);
    assert_eq!(success_vault.snapshot(), 1);
    assert_eq!(success_after, wal_rows[ledger_index].value);
    assert!(ledger_index < base_index);
    assert_eq!(ledger_entry.seq, 0);
    assert_eq!(ledger_entry.prev_hash, [0; 32]);
    assert_eq!(ledger_entry.kind, EntryKind::Ingest);
    assert!(matches!(ledger_entry.subject, SubjectId::Cx(value) if value == success_id));
    assert_eq!(got.provenance.seq, ledger_entry.seq);
    assert_eq!(got.provenance.hash, ledger_entry.entry_hash);
    assert_eq!(success_hook_before.next_seq, 0);
    assert_eq!(success_hook_after.next_seq, 1);
    assert_eq!(success_hook_after.store_rows, 1);
}

fn router_failure_vault(dir: &Path) -> AsterVault<FixedClock> {
    let router = CfRouter::open(dir, 1).unwrap();
    let ledger_hook = ledger_hook::recover_hook(
        &durable::RecoveredBatches {
            batches: Vec::new(),
            last_recovered_seq: 0,
            torn_tail: None,
            temporal_policy: None,
            dedup_policy: None,
            retention_horizon: crate::timetravel::RetentionHorizon::default(),
        },
        None,
    )
    .unwrap();
    AsterVault {
        vault_id: vault_id(),
        vault_salt: b"salt".to_vec(),
        clock: FixedClock::new(123),
        rows: VersionedCfStore::new_with_router(0, router),
        durable: None,
        dedup_policy: DedupPolicy::default(),
        retention_horizon: std::sync::Mutex::new(crate::timetravel::RetentionHorizon::default()),
        ledger_hook: Some(ledger_hook),
        recurrence_write_lock: std::sync::Mutex::new(()),
        recovery_report: VaultRecoveryReport {
            last_recovered_seq: 0,
            torn_tail: None,
        },
        residency: None,
    }
}

fn read_ledger_row<C: Clock>(vault: &AsterVault<C>, snapshot: u64) -> Option<Vec<u8>> {
    vault
        .read_cf_at(snapshot, ColumnFamily::Ledger, &ledger_key(0))
        .expect("read ledger row")
}

fn hook_state<C: Clock>(vault: &AsterVault<C>) -> HookState {
    let guard = vault.ledger_hook.as_ref().unwrap().lock().unwrap();
    HookState {
        next_seq: guard.appender().next_seq(),
        prev_hash: hex(&guard.appender().prev_hash()),
        store_rows: guard.appender().store().scan().unwrap().len(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HookState {
    next_seq: u64,
    prev_hash: String,
    store_rows: usize,
}

impl HookState {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "next_seq": self.next_seq,
            "prev_hash": self.prev_hash,
            "store_rows": self.store_rows,
        })
    }
}

fn sample_constellation<C: Clock>(
    vault: &AsterVault<C>,
    label: &str,
    seed: u16,
) -> calyx_core::Constellation {
    let input = format!("ph35-atomicity-{label}-{seed}");
    let cx_id = vault.cx_id_for_input(input.as_bytes(), 7);
    let mut input_hash = [0_u8; 32];
    input_hash[..input.len()].copy_from_slice(input.as_bytes());
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![f32::from(seed), 1.0],
        },
    );
    calyx_core::Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 7,
        created_at: 1_785_500_000 + u64::from(seed),
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some(format!("synthetic://ph35/atomicity/{label}/{seed}")),
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

fn row_index(rows: &[encode::WriteRow], cf: ColumnFamily) -> usize {
    rows.iter()
        .position(|row| row.cf == cf)
        .expect("row for CF")
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph35-group-commit-atomicity-fsv"))
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyx-aster-vault-{name}-{}-{id}",
        std::process::id()
    ));
    reset_dir(&dir);
    dir
}

fn reset_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).expect("create test dir");
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).expect("cleanup test dir");
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
