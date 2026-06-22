//! Full State Verification for vault data residency (PRD `30 §4`, A33).
//!
//! Sources of truth: (1) the residency pin physically on disk at
//! `<vault>/residency.json`; (2) the governance audit entry physically in the
//! `Ledger` column family. Every check re-reads the SoT independently of the
//! call's return value, with hand-computed expectations. Run with `--nocapture`
//! to emit the evidence log.

use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::residency::{RESIDENCY_SIDECAR, Residency};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::VaultId;
use calyx_ledger::{EntryKind, SubjectId, decode as decode_ledger};
use std::fs;
use std::path::Path;

mod fsv_support;
use fsv_support::{named_fsv_root, reset_dir};

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
}

fn options_with_residency(pin: Residency) -> VaultOptions {
    VaultOptions {
        residency: Some(pin),
        ..VaultOptions::default()
    }
}

/// Scans the Ledger CF for a governance (`Admin`) audit entry whose payload
/// records a residency violation against `target`.
fn find_residency_audit(
    vault: &AsterVault,
    snapshot: u64,
    target: &Path,
) -> Option<serde_json::Value> {
    for seq in 0..64u64 {
        let Some(row) = vault
            .read_cf_at(snapshot, ColumnFamily::Ledger, &ledger_key(seq))
            .expect("read ledger row")
        else {
            continue;
        };
        let entry = decode_ledger(&row).expect("decode ledger entry");
        if entry.kind != EntryKind::Admin {
            continue;
        }
        let payload: serde_json::Value =
            serde_json::from_slice(&entry.payload).expect("audit payload json");
        if payload.get("event").and_then(|v| v.as_str()) == Some("residency_violation")
            && payload
                .get("attempted_target_hash")
                .and_then(|v| v.as_str())
                == Some(Residency::path_digest(target).as_str())
        {
            assert!(
                matches!(entry.subject, SubjectId::Guard(_)),
                "residency audit must use a Guard subject"
            );
            return Some(payload);
        }
    }
    None
}

#[test]
fn vault_residency_fsv() {
    let (root, keep) = named_fsv_root("CALYX_ASTER_RESIDENCY_FSV_ROOT", "residency-fsv");
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let dataset_root = vault_dir.clone();

    println!("\n==================== RESIDENCY FSV ====================");
    println!("SoT 1: {}/{}", vault_dir.display(), RESIDENCY_SIDECAR);
    println!("SoT 2: Ledger CF (governance Admin audit entry)");

    // ---- Trigger X: open a vault pinned to its own dataset root. -----------
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"residency-fsv".to_vec(),
        options_with_residency(Residency::pin(&dataset_root)),
    )
    .expect("open pinned vault");

    // ---- SoT 1: read the pin back from disk, independent of the API. -------
    let sidecar = vault_dir.join(RESIDENCY_SIDECAR);
    let on_disk: Residency =
        serde_json::from_slice(&fs::read(&sidecar).expect("read residency.json")).expect("decode");
    println!("\n[SoT 1: on-disk pin] {:?}", on_disk);
    assert_eq!(on_disk, Residency::pin(&dataset_root));
    assert_eq!(vault.residency(), Some(&Residency::pin(&dataset_root)));

    // ---- Happy path: an in-dataset target is authorized (no audit). --------
    let inside = dataset_root.join("cf").join("base").join("0001.sst");
    println!("\n[authorize in-dataset] {}", inside.display());
    vault
        .authorize_external_copy(&inside)
        .expect("in-dataset copy authorized");

    // ---- EDGE 1: off-dataset copy fails closed + writes an audit entry. ----
    let outside = root.join("exfil").join("leak.sst");
    println!("\n[EDGE 1: off-dataset copy] {}", outside.display());
    let ledger_before = ledger_entry_count(&vault);
    let err = vault
        .authorize_external_copy(&outside)
        .expect_err("off-dataset copy must fail closed");
    println!("  return code = {} / msg = {}", err.code, err.message);
    assert_eq!(err.code, "CALYX_RESIDENCY_VIOLATION");
    vault.flush().expect("flush audit entry");
    let snapshot = vault.latest_seq();
    let ledger_after = ledger_entry_count(&vault);
    println!("  ledger entries BEFORE = {ledger_before}, AFTER = {ledger_after}");
    assert_eq!(ledger_after, ledger_before + 1, "exactly one audit entry");
    let audit = find_residency_audit(&vault, snapshot, &outside)
        .expect("residency violation must be audited in the Ledger");
    println!("  [SoT 2: ledger audit] {audit}");
    assert_eq!(audit["event"], "residency_violation");
    assert_eq!(audit["allow_off_dataset"], serde_json::json!(false));
    // The audit digest must be the verifiable blake3 of the attempted target.
    assert_eq!(
        audit["attempted_target_hash"],
        serde_json::json!(Residency::path_digest(&outside))
    );

    // ---- EDGE 2: pin is immutable — re-pinning elsewhere fails closed. -----
    println!("\n[EDGE 2: pin conflict]");
    let conflict = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"residency-fsv".to_vec(),
        options_with_residency(Residency::pin(root.join("somewhere-else"))),
    )
    .expect_err("re-pinning to a different root must fail closed");
    println!("  reopen with different pin -> {}", conflict.code);
    assert_eq!(conflict.code, "CALYX_RESIDENCY_PIN_CONFLICT");

    // ---- EDGE 3: reopen WITHOUT options re-reads the persisted pin. --------
    println!("\n[EDGE 3: pin survives reopen]");
    let reopened = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"residency-fsv".to_vec(),
        VaultOptions::default(),
    )
    .expect("reopen without residency option");
    println!("  reopened pin = {:?}", reopened.residency());
    assert_eq!(reopened.residency(), Some(&Residency::pin(&dataset_root)));
    let err2 = reopened
        .authorize_external_copy(&outside)
        .expect_err("enforcement still active after reopen");
    assert_eq!(err2.code, "CALYX_RESIDENCY_VIOLATION");

    // ---- EDGE 4: permissive policy authorizes off-dataset (no audit). ------
    println!("\n[EDGE 4: permissive policy]");
    let open_dir = root.join("open-vault");
    let open_vault = AsterVault::new_durable(
        &open_dir,
        vault_id(),
        b"residency-open".to_vec(),
        options_with_residency(Residency::pin_allowing_off_dataset(&open_dir)),
    )
    .expect("open permissive vault");
    open_vault
        .authorize_external_copy(Path::new("/tmp/anywhere"))
        .expect("permissive policy authorizes off-dataset");
    println!("  off-dataset copy under permissive policy = OK");

    println!("\n==================== FSV PASS ====================\n");
    if keep {
        println!("residency_fsv_root={}", root.display());
    } else {
        let _ = fs::remove_dir_all(&root);
    }
}

fn ledger_entry_count(vault: &AsterVault) -> usize {
    let snapshot = vault.latest_seq();
    (0..64u64)
        .filter(|seq| {
            vault
                .read_cf_at(snapshot, ColumnFamily::Ledger, &ledger_key(*seq))
                .expect("read ledger row")
                .is_some()
        })
        .count()
}
