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
use crate::vault::key::{CALYX_DECRYPTION_FAILED, VaultKey};
use crate::vault::keyspace::KeyspaceGuard;
use crate::vault::quota::{QuotaConfig, QuotaGuard};
use calyx_core::{CalyxError, Result, Ts, VaultId};
use calyx_ledger::ActorId;
use rand::RngCore;
use rand::rngs::OsRng;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// AES-GCM nonce length embedded at the front of every encrypted value.
const VALUE_NONCE_LEN: usize = 12;
/// AES-GCM authentication tag length appended by `VaultKey`.
const VALUE_TAG_LEN: usize = 16;

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
        self.grants_read()
            .check_grant(self.vault_id, dst, actor, now)
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
    ///
    /// Returns `nonce || ciphertext || tag`, with a fresh 96-bit nonce generated
    /// internally for every call. Callers never supply value nonces.
    pub fn encrypt_value(&self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        let mut nonce = [0_u8; VALUE_NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);
        let mut ciphertext = self.key.encrypt(&nonce, plaintext, aad)?;
        let mut sealed = Vec::with_capacity(VALUE_NONCE_LEN + ciphertext.len());
        sealed.extend_from_slice(&nonce);
        sealed.append(&mut ciphertext);
        Ok(sealed)
    }

    /// AES-256-GCM decrypts a `nonce || ciphertext || tag` value (fails closed).
    pub fn decrypt_value(&self, sealed_value: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
        if sealed_value.len() < VALUE_NONCE_LEN + VALUE_TAG_LEN {
            return Err(CalyxError {
                code: CALYX_DECRYPTION_FAILED,
                message: format!(
                    "encrypted value is {} bytes, shorter than nonce plus GCM tag",
                    sealed_value.len()
                ),
                remediation: "read the complete encrypted value envelope before decrypting",
            });
        }
        let mut nonce = [0_u8; VALUE_NONCE_LEN];
        nonce.copy_from_slice(&sealed_value[..VALUE_NONCE_LEN]);
        let ciphertext = &sealed_value[VALUE_NONCE_LEN..];
        self.key.decrypt(&nonce, ciphertext, aad)
    }

    /// Crypto-shreds the live vault key for lawful/user-requested erasure.
    pub fn shred_key_for_erasure(&mut self) {
        self.key.shred_for_erasure();
    }

    /// Returns true once the live key has been overwritten by the erasure sentinel.
    pub fn is_key_shredded_for_erasure(&self) -> bool {
        self.key.is_shredded_for_erasure()
    }

    /// Charges `cx_count` against this vault's ingest quota at `now_ns`.
    pub fn charge_ingest(&self, cx_count: u32, now_ns: u64) -> Result<()> {
        self.quota.charge_ingest(cx_count, now_ns)
    }

    fn grants_read(&self) -> RwLockReadGuard<'_, GrantStore> {
        self.grants
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Write access to the grant store, recovering from lock poisoning.
    pub fn grants_write(&self) -> RwLockWriteGuard<'_, GrantStore> {
        self.grants
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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
        let err =
            VaultContext::new(vault(0xA1), b"", QuotaConfig::default(), "tank/calyx").unwrap_err();
        assert_eq!(err.code, "CALYX_VAULT_KEY_MISSING");
    }

    #[test]
    fn same_master_different_vault_derives_distinct_keys() {
        let master = b"shared-master-key-material-000000";
        let a =
            VaultContext::new(vault(0xA1), master, QuotaConfig::default(), "tank/calyx").unwrap();
        let b =
            VaultContext::new(vault(0xB2), master, QuotaConfig::default(), "tank/calyx").unwrap();
        let sealed = a.encrypt_value(b"x", b"aad").unwrap();
        assert_eq!(a.decrypt_value(&sealed, b"aad").unwrap(), b"x");
        assert_eq!(
            b.decrypt_value(&sealed, b"aad").unwrap_err().code,
            "CALYX_DECRYPTION_FAILED",
            "distinct vaults must derive distinct keys"
        );
    }

    #[test]
    fn encrypt_value_embeds_fresh_nonce_and_round_trips() {
        let ctx = VaultContext::new(
            vault(0xA1),
            b"k0123456789abcdef",
            QuotaConfig::default(),
            "tank/calyx",
        )
        .unwrap();
        let first = ctx.encrypt_value(b"same plaintext", b"aad").unwrap();
        let second = ctx.encrypt_value(b"same plaintext", b"aad").unwrap();
        assert_eq!(first.len(), 12 + "same plaintext".len() + 16);
        assert_eq!(second.len(), first.len());
        assert_ne!(&first[..12], &second[..12], "value nonces must be fresh");
        assert_ne!(first, second, "fresh nonces must change the sealed value");
        assert_eq!(
            ctx.decrypt_value(&first, b"aad").unwrap(),
            b"same plaintext"
        );
        assert_eq!(
            ctx.decrypt_value(&second, b"aad").unwrap(),
            b"same plaintext"
        );
    }

    #[test]
    fn decrypt_value_rejects_truncated_envelope() {
        let ctx = VaultContext::new(
            vault(0xA1),
            b"k0123456789abcdef",
            QuotaConfig::default(),
            "tank/calyx",
        )
        .unwrap();
        let err = ctx.decrypt_value(&[0_u8; 27], b"aad").unwrap_err();
        assert_eq!(err.code, "CALYX_DECRYPTION_FAILED");
    }

    #[test]
    fn decrypt_value_rejects_wrong_aad() {
        let ctx = VaultContext::new(
            vault(0xA1),
            b"k0123456789abcdef",
            QuotaConfig::default(),
            "tank/calyx",
        )
        .unwrap();
        let sealed = ctx.encrypt_value(b"x", b"aad").unwrap();
        let err = ctx.decrypt_value(&sealed, b"other-aad").unwrap_err();
        assert_eq!(err.code, "CALYX_DECRYPTION_FAILED");
    }

    #[test]
    fn context_constructs_when_zfs_unavailable() {
        // ZFS absence is not an error — context still constructs.
        let ctx = VaultContext::new(
            vault(0xA1),
            b"k0123456789abcdef",
            QuotaConfig::default(),
            "tank/none",
        )
        .unwrap();
        println!("zfs_status = {:?}", ctx.zfs_status());
        // On this dev host: not Enabled, but construction succeeded.
        assert_eq!(ctx.vault_id(), vault(0xA1));
    }

    #[test]
    fn quota_respects_configured_limits() {
        let ctx = VaultContext::new(
            vault(0xA1),
            b"k0123456789abcdef",
            QuotaConfig {
                max_ingest_cx_per_sec: 10,
                ..QuotaConfig::default()
            },
            "tank/calyx",
        )
        .unwrap();
        assert!(ctx.charge_ingest(10, 1_000_000_000).is_ok());
        assert_eq!(
            ctx.charge_ingest(1, 1_000_000_000).unwrap_err().code,
            "CALYX_QUOTA_EXCEEDED"
        );
    }

    #[test]
    fn cross_vault_denied_then_granted() {
        let a = VaultContext::new(
            vault(0xA1),
            b"k0123456789abcdef",
            QuotaConfig::default(),
            "tank/calyx",
        )
        .unwrap();
        let b_id = vault(0xB2);
        let actor = ActorId::Agent("agent1".to_string());
        // Default-deny.
        assert_eq!(
            a.check_cross_vault_read(b_id, actor.clone(), 1_000)
                .unwrap_err()
                .code,
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
