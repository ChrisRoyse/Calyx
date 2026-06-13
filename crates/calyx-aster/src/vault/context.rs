//! `VaultContext` — the PH60 tenant-isolation aggregate (T07).
//!
//! Every vault-scoped storage operation receives a `VaultContext`, which binds
//! together all four defense-in-depth layers for one vault:
//!
//! - [`VaultKey`] — per-vault AES-256-GCM key (HKDF-derived) for value crypto.
//! - [`KeyspaceGuard`] — per-vault CF-key prefix isolation.
//! - [`GrantStore`] — default-deny cross-vault grants + immutable audit.
//! - [`QuotaGuard`] — per-vault rate limits / backpressure.
//!
//! plus the probed [`ZfsEncryptionStatus`] (outermost crypto-at-rest layer),
//! recorded so the vault manifest can report it.

use crate::cf::ColumnFamily;
use crate::security::zfs::{ZfsEncryptionStatus, probe_zfs_encryption};
use crate::vault::grant::GrantStore;
use crate::vault::key::VaultKey;
use crate::vault::keyspace::KeyspaceGuard;
use crate::vault::quota::{QuotaConfig, QuotaGuard};
use calyx_core::{Result, Ts, VaultId};
use calyx_ledger::ActorId;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// The single per-vault security aggregate threaded through every storage op.
#[derive(Debug)]
pub struct VaultContext {
    vault_id: VaultId,
    key: VaultKey,
    keyspace: KeyspaceGuard,
    grants: Arc<RwLock<GrantStore>>,
    quota: QuotaGuard,
    zfs_status: ZfsEncryptionStatus,
}

impl VaultContext {
    /// Builds the full PH60 stack for `vault_id`.
    ///
    /// Derives the vault key from `master_key` via HKDF, builds the keyspace
    /// guard, an empty grant store, the quota guard, and probes the ZFS dataset.
    ///
    /// # Errors
    /// [`CALYX_VAULT_KEY_MISSING`](crate::vault::key::CALYX_VAULT_KEY_MISSING)
    /// if `master_key` is empty (propagated from [`VaultKey::derive`]).
    pub fn new(
        vault_id: VaultId,
        master_key: &[u8],
        config: QuotaConfig,
        zfs_dataset: &str,
    ) -> Result<Self> {
        let key = VaultKey::derive(master_key, &vault_id)?;
        Ok(Self {
            vault_id,
            key,
            keyspace: KeyspaceGuard::new(vault_id),
            grants: Arc::new(RwLock::new(GrantStore::new())),
            quota: QuotaGuard::new(vault_id, config),
            zfs_status: probe_zfs_encryption(zfs_dataset),
        })
    }

    /// The vault this context scopes to.
    pub fn vault_id(&self) -> VaultId {
        self.vault_id
    }

    /// The probed ZFS encryption status (recorded in the vault manifest).
    pub fn zfs_status(&self) -> &ZfsEncryptionStatus {
        &self.zfs_status
    }

    /// The keyspace guard (a `Copy` codec) for direct prefix checks.
    pub fn keyspace(&self) -> KeyspaceGuard {
        self.keyspace
    }

    /// The quota guard.
    pub fn quota(&self) -> &QuotaGuard {
        &self.quota
    }

    /// Shared handle to the grant store (read for checks, write to add/revoke).
    pub fn grants(&self) -> &Arc<RwLock<GrantStore>> {
        &self.grants
    }

    /// Authorizes a cross-vault read from this vault into `dst` for `actor`.
    ///
    /// # Errors
    /// [`CALYX_VAULT_ACCESS_DENIED`](calyx_core::CalyxError::vault_access_denied)
    /// if no active grant exists; the denial is audited in the grant store.
    pub fn check_cross_vault_read(&self, dst: VaultId, actor: ActorId, now: Ts) -> Result<()> {
        self.grants_read().check_grant(self.vault_id, dst, actor, now)
    }

    /// Encodes a storable, vault-prefixed CF key (`prefix ‖ cf_tag ‖ user_key`).
    pub fn encode_key(&self, cf: ColumnFamily, user_key: &[u8]) -> Vec<u8> {
        self.keyspace.encode_key(cf, user_key)
    }

    /// Decodes a raw CF key, verifying it belongs to this vault.
    ///
    /// # Errors
    /// [`CALYX_VAULT_KEYSPACE_MISMATCH`](crate::vault::keyspace::CALYX_VAULT_KEYSPACE_MISMATCH)
    /// for a foreign / short / malformed key.
    pub fn decode_key<'a>(&self, raw: &'a [u8]) -> Result<(ColumnFamily, &'a [u8])> {
        self.keyspace.decode_key(raw)
    }

    /// AES-256-GCM encrypts a value under this vault's key.
    pub fn encrypt_value(&self, nonce: &[u8; 12], plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        self.key.encrypt(nonce, plaintext, aad)
    }

    /// AES-256-GCM decrypts a value under this vault's key (fails closed).
    pub fn decrypt_value(&self, nonce: &[u8; 12], ciphertext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        self.key.decrypt(nonce, ciphertext, aad)
    }

    /// Charges `cx_count` against this vault's ingest quota at `now_ns`.
    pub fn charge_ingest(&self, cx_count: u32, now_ns: u64) -> Result<()> {
        self.quota.charge_ingest(cx_count, now_ns)
    }

    fn grants_read(&self) -> RwLockReadGuard<'_, GrantStore> {
        self.grants.read().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Write access to the grant store, recovering from lock poisoning.
    pub fn grants_write(&self) -> RwLockWriteGuard<'_, GrantStore> {
        self.grants.write().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::grant::GrantEntry;
    use ulid::Ulid;

    fn vault(byte: u8) -> VaultId {
        VaultId::from_ulid(Ulid::from_bytes([byte; 16]))
    }

    #[test]
    fn new_with_empty_master_fails_closed() {
        let err = VaultContext::new(vault(0xA1), b"", QuotaConfig::default(), "tank/calyx").unwrap_err();
        assert_eq!(err.code, "CALYX_VAULT_KEY_MISSING");
    }

    #[test]
    fn same_master_different_vault_derives_distinct_keys() {
        let master = b"shared-master-key-material-000000";
        let a = VaultContext::new(vault(0xA1), master, QuotaConfig::default(), "tank/calyx").unwrap();
        let b = VaultContext::new(vault(0xB2), master, QuotaConfig::default(), "tank/calyx").unwrap();
        // Same plaintext+nonce+aad encrypts differently because HKDF info (the
        // vault id) differs -> different derived keys.
        let nonce = [7u8; 12];
        let ca = a.encrypt_value(&nonce, b"x", b"aad").unwrap();
        let cb = b.encrypt_value(&nonce, b"x", b"aad").unwrap();
        assert_ne!(ca, cb, "distinct vaults must derive distinct keys");
    }

    #[test]
    fn context_constructs_when_zfs_unavailable() {
        // ZFS absence is not an error — context still constructs.
        let ctx = VaultContext::new(vault(0xA1), b"k0123456789abcdef", QuotaConfig::default(), "tank/none").unwrap();
        println!("zfs_status = {:?}", ctx.zfs_status());
        // On this dev host: not Enabled, but construction succeeded.
        assert_eq!(ctx.vault_id(), vault(0xA1));
    }

    #[test]
    fn quota_respects_configured_limits() {
        let ctx = VaultContext::new(
            vault(0xA1),
            b"k0123456789abcdef",
            QuotaConfig { max_ingest_cx_per_sec: 10, ..QuotaConfig::default() },
            "tank/calyx",
        )
        .unwrap();
        assert!(ctx.charge_ingest(10, 1_000_000_000).is_ok());
        assert_eq!(ctx.charge_ingest(1, 1_000_000_000).unwrap_err().code, "CALYX_QUOTA_EXCEEDED");
    }

    #[test]
    fn cross_vault_denied_then_granted() {
        let a = VaultContext::new(vault(0xA1), b"k0123456789abcdef", QuotaConfig::default(), "tank/calyx").unwrap();
        let b_id = vault(0xB2);
        let actor = ActorId::Agent("agent1".to_string());
        // Default-deny.
        assert_eq!(
            a.check_cross_vault_read(b_id, actor.clone(), 1_000).unwrap_err().code,
            "CALYX_VAULT_ACCESS_DENIED"
        );
        // Grant, then allowed.
        a.grants_write().add_grant(GrantEntry {
            src_vault: a.vault_id(),
            dst_vault: b_id,
            actor: actor.clone(),
            granted_at: 1_000,
            expires_at: None,
            read_only: true,
        });
        assert!(a.check_cross_vault_read(b_id, actor, 1_000).is_ok());
    }
}
