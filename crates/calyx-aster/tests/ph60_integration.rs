//! PH60 · T07 — end-to-end tenant-isolation integration FSV.
//!
//! Exercises the complete defense-in-depth stack (key + keyspace + grant +
//! quota) through [`VaultContext`] against a **real on-disk CF store**, proving
//! the two PH60 phase-exit gates:
//!   1. a cross-vault read without a grant returns `CALYX_VAULT_ACCESS_DENIED`
//!      and leaves an audit record;
//!   2. vault data is encrypted at rest with a per-vault key — vault B cannot
//!      decrypt vault A's bytes.

use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_aster::vault::quota::QuotaConfig;
use calyx_aster::vault::{GrantEntry, VaultContext};
use calyx_core::VaultId;
use calyx_ledger::ActorId;
use std::path::PathBuf;
use ulid::Ulid;

mod fsv_support;
use fsv_support::prepared_temp_root;

fn test_dir(name: &str) -> PathBuf {
    prepared_temp_root("calyx-aster-ph60", name)
}

fn vault(byte: u8) -> VaultId {
    VaultId::from_ulid(Ulid::from_bytes([byte; 16]))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn ph60_full_stack_tenant_isolation_fsv() {
    let dir = test_dir("fullstack");
    let master = b"ph60-shared-master-key-0123456789";
    let actor = ActorId::Agent("agent1".to_string());
    let now: u64 = 1_000_000;

    // Two vaults sharing the same master but with distinct ids -> distinct keys.
    let a_id = vault(0xA1);
    let b_id = vault(0xB2);
    let ctx_a = VaultContext::new(a_id, master, QuotaConfig::default(), "tank/calyx").unwrap();
    let ctx_b = VaultContext::new(b_id, master, QuotaConfig::default(), "tank/calyx").unwrap();

    // ── Gate 2a: encrypt-at-rest round trip through a real CF store ──────────
    let plaintext = b"synthetic constellation payload v1";
    let aad = b"cx:base";
    let key = ctx_a.encode_key(ColumnFamily::Base, b"cx-0001");
    let ciphertext = ctx_a.encrypt_value(plaintext, aad).unwrap();

    let mut router = CfRouter::open(&dir, 1 << 20).unwrap();
    router.put(ColumnFamily::Base, &key, &ciphertext).unwrap();

    // Separate read of the SoT: pull the row back off disk.
    let stored = router
        .get(ColumnFamily::Base, &key)
        .unwrap()
        .expect("row must physically exist");
    println!("[gate2] stored key        = {}", hex(&key));
    println!("[gate2] stored ciphertext = {}", hex(&stored));
    assert_eq!(stored, ciphertext, "ciphertext must persist byte-exact");
    assert_eq!(stored.len(), 12 + plaintext.len() + 16);
    assert_ne!(stored, plaintext, "value at rest must NOT be plaintext");

    // Vault A decodes its own key + decrypts the bytes back to plaintext.
    let (cf, user_key) = ctx_a.decode_key(&key).unwrap();
    assert_eq!(cf, ColumnFamily::Base);
    assert_eq!(user_key, b"cx-0001");
    let recovered = ctx_a.decrypt_value(&stored, aad).unwrap();
    println!(
        "[gate2] decrypted (vault A) = {:?}",
        String::from_utf8_lossy(&recovered)
    );
    assert_eq!(recovered, plaintext, "vault A recovers its own plaintext");

    // ── Gate keyspace: vault A cannot decode a vault-B-prefixed key ──────────
    let b_key = ctx_b.encode_key(ColumnFamily::Base, b"cx-0001");
    let mismatch = ctx_a.decode_key(&b_key).unwrap_err();
    println!(
        "[keyspace] ctx_a.decode_key(vault_b_key) = Err({})",
        mismatch.code
    );
    assert_eq!(mismatch.code, "CALYX_VAULT_KEYSPACE_MISMATCH");

    // ── Gate 1: cross-vault read denied + audited, then granted ─────────────
    let denied = ctx_a
        .check_cross_vault_read(b_id, actor.clone(), now)
        .unwrap_err();
    println!(
        "[gate1] check_cross_vault_read (no grant) = Err({})",
        denied.code
    );
    assert_eq!(denied.code, "CALYX_VAULT_ACCESS_DENIED");

    // The denial physically resides in the audit ring.
    let events = ctx_a.grants().read().unwrap().audit_events(1);
    println!("[gate1] audit_events(1) = {events:?}");
    assert_eq!(events.len(), 1);
    assert!(
        matches!(&events[0], calyx_aster::vault::AuditEvent::Denied { src_vault, dst_vault, .. }
            if *src_vault == a_id && *dst_vault == b_id),
        "audit ring must hold a Denied(vault_a, vault_b) record"
    );

    // Add the grant -> the same read is now authorized.
    ctx_a.grants_write().add_grant(GrantEntry {
        src_vault: a_id,
        dst_vault: b_id,
        actor: actor.clone(),
        granted_at: now,
        expires_at: None,
        read_only: true,
    });
    assert!(
        ctx_a.check_cross_vault_read(b_id, actor, now).is_ok(),
        "granted cross-vault read must be allowed"
    );

    // ── Gate 2b: vault B cannot decrypt vault A's ciphertext ────────────────
    let cross = ctx_b.decrypt_value(&stored, aad).unwrap_err();
    println!(
        "[gate2] ctx_b.decrypt_value(vault_a_ciphertext) = Err({})",
        cross.code
    );
    assert_eq!(
        cross.code, "CALYX_DECRYPTION_FAILED",
        "different vault -> different derived key -> tag mismatch"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}
