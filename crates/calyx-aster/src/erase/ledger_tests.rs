use super::*;
use crate::cf::ledger_key;
use crate::sst::write_sst;
use calyx_core::{Clock, CxFlags, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultStore};
use calyx_ledger::{
    ErasureScope as LedgerErasureScope, LedgerEntry, decode as decode_ledger, tombstone_from_entry,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use ulid::Ulid;

fn vault_id() -> VaultId {
    VaultId::from_ulid(Ulid::from_bytes([0xCD; 16]))
}

fn context() -> VaultContext {
    VaultContext::new(
        vault_id(),
        b"erase-ledger-test-master-key-material",
        crate::vault::QuotaConfig::default(),
        "tank/calyx",
    )
    .unwrap()
}

fn durable_vault(name: &str) -> (PathBuf, AsterVault) {
    let dir =
        std::env::temp_dir().join(format!("calyx-erase-ledger-{name}-{}", std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    let vault = AsterVault::new_durable(
        &dir,
        vault_id(),
        b"salt",
        crate::vault::VaultOptions::default(),
    )
    .unwrap();
    (dir, vault)
}

fn cx<C>(vault: &AsterVault<C>, seed: &'static [u8]) -> Constellation
where
    C: Clock,
{
    let hash = *blake3::hash(seed).as_bytes();
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![seed[0] as f32, 1.0],
        },
    );
    Constellation {
        cx_id: vault.cx_id_for_input(seed, 1),
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 123,
        input_ref: InputRef {
            hash,
            pointer: Some(format!(
                "synthetic://issue503-{0:02x}{1:02x}{2:02x}{3:02x}",
                hash[0], hash[1], hash[2], hash[3]
            )),
            redacted: true,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [seed[0]; 32],
        },
        flags: CxFlags::default(),
    }
}

fn ledger_entries<C>(vault: &AsterVault<C>) -> Vec<LedgerEntry>
where
    C: Clock,
{
    let mut rows = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Ledger)
        .unwrap();
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    rows.into_iter()
        .map(|(_, bytes)| decode_ledger(&bytes).unwrap())
        .collect()
}

#[test]
fn erase_cx_writes_one_ledger_tombstone_and_reerase_fails() {
    let (_dir, vault) = durable_vault("cx");
    let mut ctx = context();
    let registry = EraseRegistry::new();
    let original = b"fsv-original-content";
    let first = cx(&vault, original);
    let first_id = first.cx_id;
    vault.put(first).unwrap();

    let result = vault
        .erase(EraseScope::Cx(first_id), &mut ctx, &registry)
        .unwrap();
    let entries = ledger_entries(&vault);
    let tombstones = erase_tombstones(&entries);
    let tombstone = &tombstones[0];

    assert_eq!(result.records_deleted, 1);
    assert_eq!(tombstones.len(), 1);
    assert_eq!(tombstone.records_deleted, 1);
    assert!(tombstone.erased_at > 0);
    assert_eq!(tombstone.scope, LedgerErasureScope::Cx(first_id));
    assert!(tombstone.as_ledger_payload().len() < 128);
    assert!(
        !tombstone
            .as_ledger_payload()
            .windows(4)
            .any(|window| original.windows(4).any(|needle| needle == window))
    );

    let error = vault
        .erase(EraseScope::Cx(first_id), &mut ctx, &registry)
        .unwrap_err();
    assert_eq!(error.code, "CALYX_ERASE_ALREADY_TOMBSTONED");
    assert_eq!(erase_tombstones(&ledger_entries(&vault)).len(), 1);
}

#[test]
fn empty_vault_erase_writes_vault_scope_tombstone() {
    let (_dir, vault) = durable_vault("empty-vault");
    let mut ctx = context();
    let registry = EraseRegistry::new();

    let result = vault.erase(EraseScope::Vault, &mut ctx, &registry).unwrap();
    let entries = ledger_entries(&vault);
    let tombstone = tombstone_from_entry(&entries[0]).unwrap().unwrap();

    assert_eq!(result.records_deleted, 0);
    assert_eq!(entries.len(), 1);
    assert_eq!(tombstone.scope, LedgerErasureScope::Vault);
    assert_eq!(tombstone.records_deleted, 0);
    assert!(ctx.is_key_shredded_for_erasure());
}

#[test]
fn corrupt_ledger_aborts_before_delete_or_shred() {
    let (dir, vault) = durable_vault("corrupt-ledger");
    let ctx = context();
    let first = cx(&vault, b"kept-after-ledger-error");
    let ciphertext = ctx.encrypt_value(b"still-readable", b"aad").unwrap();
    vault.put(first).unwrap();
    vault.flush().unwrap();
    drop(vault);
    let _ = fs::remove_dir_all(dir.join("wal"));
    replace_ledger_ssts_with_corrupt_row(&dir);

    // The ledger head is anchored outside the row chain and verified at open,
    // so a structurally corrupt ledger row is now caught at open (fail
    // closed) — strictly earlier and safer than the prior erase-time guard. The
    // vault refuses to load, so no destructive action can run. Assert the
    // corruption is reported loudly and nothing was deleted or key-shredded.
    let error = AsterVault::open(
        &dir,
        vault_id(),
        b"salt",
        crate::vault::VaultOptions::default(),
    )
    .expect_err("open must fail closed on a corrupt ledger");

    assert!(error.code.starts_with("CALYX_LEDGER_"));
    assert!(!ctx.is_key_shredded_for_erasure());
    assert_eq!(
        ctx.decrypt_value(&ciphertext, b"aad").unwrap(),
        b"still-readable"
    );
}

fn replace_ledger_ssts_with_corrupt_row(vault_dir: &Path) {
    let ledger_dir = vault_dir.join("cf").join(ColumnFamily::Ledger.name());
    let mut paths = fs::read_dir(&ledger_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("sst"))
        .collect::<Vec<_>>();
    paths.sort();
    assert!(!paths.is_empty());
    for path in paths {
        fs::remove_file(path).unwrap();
    }
    write_sst(
        ledger_dir.join("00000000000000000000.sst"),
        [(ledger_key(0).as_slice(), b"corrupt-ledger".as_slice())],
    )
    .unwrap();
}

fn erase_tombstones(entries: &[LedgerEntry]) -> Vec<calyx_ledger::ErasureTombstone> {
    entries
        .iter()
        .filter_map(|entry| tombstone_from_entry(entry).unwrap())
        .collect()
}

#[test]
#[ignore = "manual FSV fixture for issue #503"]
fn issue503_erasure_ledger_fsv_fixture() {
    let root = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-issue503-fsv"));
    let vault_dir = root.join("issue503-ledger-vault");
    if vault_dir.exists() {
        fs::remove_dir_all(&vault_dir).unwrap();
    }
    fs::create_dir_all(&root).unwrap();
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"salt",
        crate::vault::VaultOptions::default(),
    )
    .unwrap();
    let mut ctx = context();
    let registry = EraseRegistry::new();
    let original = b"FSV_ISSUE_503_ORIGINAL_CONTENT";
    let record = cx(&vault, original);
    let cx_id = record.cx_id;
    vault.put(record).unwrap();
    let before_tombstones = erase_tombstones(&ledger_entries(&vault));
    println!("FSV_ISSUE503_BEFORE_TOMBSTONES={}", before_tombstones.len());

    let result = vault
        .erase(EraseScope::Cx(cx_id), &mut ctx, &registry)
        .unwrap();
    let entries = ledger_entries(&vault);
    let tombstone = entries
        .iter()
        .find_map(|entry| tombstone_from_entry(entry).unwrap())
        .unwrap();

    println!("FSV_ISSUE503_VAULT={}", vault_dir.display());
    println!("FSV_ISSUE503_CX={cx_id}");
    println!("FSV_ISSUE503_RECORDS_DELETED={}", result.records_deleted);
    println!("FSV_ISSUE503_TOMBSTONE_SEQ={}", tombstone.seq);
    println!(
        "FSV_ISSUE503_TOMBSTONE_PAYLOAD_JSON={}",
        tombstone.as_json_value()
    );
    println!(
        "FSV_ISSUE503_PAYLOAD_BYTES={}",
        tombstone.as_ledger_payload().len()
    );
    println!(
        "FSV_ISSUE503_KEY_SHREDDED={}",
        ctx.is_key_shredded_for_erasure()
    );

    let reerase_error = vault
        .erase(EraseScope::Cx(cx_id), &mut ctx, &registry)
        .unwrap_err();
    println!("FSV_ISSUE503_REERASE_ERROR={}", reerase_error.code);
    println!(
        "FSV_ISSUE503_AFTER_REERASE_TOMBSTONES={}",
        erase_tombstones(&ledger_entries(&vault)).len()
    );

    let empty_dir = root.join("issue503-empty-vault");
    if empty_dir.exists() {
        fs::remove_dir_all(&empty_dir).unwrap();
    }
    let empty_vault = AsterVault::new_durable(
        &empty_dir,
        vault_id(),
        b"salt",
        crate::vault::VaultOptions::default(),
    )
    .unwrap();
    let mut empty_ctx = context();
    println!(
        "FSV_ISSUE503_EMPTY_BEFORE_TOMBSTONES={}",
        erase_tombstones(&ledger_entries(&empty_vault)).len()
    );
    let empty_result = empty_vault
        .erase(EraseScope::Vault, &mut empty_ctx, &registry)
        .unwrap();
    let empty_tombstone = erase_tombstones(&ledger_entries(&empty_vault))
        .into_iter()
        .next()
        .unwrap();
    println!("FSV_ISSUE503_EMPTY_VAULT={}", empty_dir.display());
    println!(
        "FSV_ISSUE503_EMPTY_RECORDS={}",
        empty_result.records_deleted
    );
    println!("FSV_ISSUE503_EMPTY_SCOPE={:?}", empty_tombstone.scope);
    println!(
        "FSV_ISSUE503_EMPTY_KEY_SHREDDED={}",
        empty_ctx.is_key_shredded_for_erasure()
    );

    let corrupt_dir = root.join("issue503-corrupt-ledger");
    if corrupt_dir.exists() {
        fs::remove_dir_all(&corrupt_dir).unwrap();
    }
    let corrupt_vault = AsterVault::new_durable(
        &corrupt_dir,
        vault_id(),
        b"salt",
        crate::vault::VaultOptions::default(),
    )
    .unwrap();
    let mut corrupt_ctx = context();
    let corrupt_record = cx(&corrupt_vault, b"FSV_ISSUE_503_CORRUPT_EDGE");
    let corrupt_id = corrupt_record.cx_id;
    let ciphertext = corrupt_ctx
        .encrypt_value(b"corrupt-edge-readable", b"aad")
        .unwrap();
    corrupt_vault.put(corrupt_record).unwrap();
    let corrupt_before_present = corrupt_vault
        .get(corrupt_id, corrupt_vault.snapshot())
        .is_ok();
    corrupt_vault
        .write_cf(
            ColumnFamily::Ledger,
            ledger_key(0),
            b"corrupt-ledger".to_vec(),
        )
        .unwrap();
    let corrupt_error = corrupt_vault
        .erase(EraseScope::Cx(corrupt_id), &mut corrupt_ctx, &registry)
        .unwrap_err();
    let corrupt_after_present = corrupt_vault
        .get(corrupt_id, corrupt_vault.snapshot())
        .is_ok();
    let corrupt_after_decrypt = corrupt_ctx.decrypt_value(&ciphertext, b"aad").unwrap();
    println!("FSV_ISSUE503_CORRUPT_VAULT={}", corrupt_dir.display());
    println!("FSV_ISSUE503_CORRUPT_ERROR={}", corrupt_error.code);
    println!("FSV_ISSUE503_CORRUPT_BEFORE_PRESENT={corrupt_before_present}");
    println!("FSV_ISSUE503_CORRUPT_AFTER_PRESENT={corrupt_after_present}");
    println!(
        "FSV_ISSUE503_CORRUPT_KEY_SHREDDED={}",
        corrupt_ctx.is_key_shredded_for_erasure()
    );
    println!(
        "FSV_ISSUE503_CORRUPT_DECRYPT={}",
        String::from_utf8(corrupt_after_decrypt).unwrap()
    );
}
