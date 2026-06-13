//! Per-vault keyspace isolation (PH60 · T02).
//!
//! [`KeyspaceGuard`] enforces that every CF key a vault writes or reads is
//! prefixed with that vault's unique 16-byte `VaultId`, so one vault's read
//! path **cannot structurally reach** another vault's key range. This is the
//! second layer of the defense-in-depth tenant-isolation stack (key +
//! **keyspace** + grant — PRD `30 §2`), sitting above [`super::key::VaultKey`].
//!
//! ## Why a 16-byte prefix (not 8)
//!
//! A `VaultId` wraps a 128-bit ULID. The task card sketched an 8-byte
//! (`as_u64`) prefix, but truncating a 128-bit identifier to 64 bits is **not
//! collision-free** — two distinct vaults could share a prefix and alias into
//! each other's keyspace, the exact failure tenant isolation must rule out
//! (prefix aliasing — see PR notes). We therefore use the **full ULID** as the
//! prefix: distinct `VaultId` ⟺ distinct prefix, with zero collision risk.
//! Every decode re-verifies the full prefix and fails closed on mismatch (A16).
//!
//! ## Why the write lock is a separate type
//!
//! [`KeyspaceGuard`] is a `Copy`, stateless key codec. A per-vault write lock
//! must be a **single shared** `Mutex` — if every `KeyspaceGuard::new` minted
//! its own mutex, two guards for the same vault would not actually exclude each
//! other. So the lock lives in [`VaultWriteLock`], constructed once per vault
//! and shared, rather than inside the copyable codec.

use crate::cf::ColumnFamily;
use calyx_core::{CalyxError, Result, VaultId};
use std::sync::{Mutex, MutexGuard};

/// A CF key presented to a vault guard did not carry that vault's keyspace
/// prefix (or was too short / malformed to carry one).
pub const CALYX_VAULT_KEYSPACE_MISMATCH: &str = "CALYX_VAULT_KEYSPACE_MISMATCH";

/// Width of the vault keyspace prefix: the full 16-byte ULID.
pub const PREFIX_LEN: usize = 16;

/// Returns the deterministic, collision-free keyspace prefix for `vault_id`:
/// the full 16 bytes of its ULID, in ULID byte order.
pub fn vault_prefix(vault_id: &VaultId) -> [u8; PREFIX_LEN] {
    vault_id.as_ulid().to_bytes()
}

/// Stateless per-vault CF-key codec enforcing keyspace isolation.
///
/// Holds no secret material, so it is `Copy`. The only path that produces a
/// storable, vault-scoped CF key is [`KeyspaceGuard::encode_key`]; the only
/// path that reads one back is [`KeyspaceGuard::decode_key`], which fails
/// closed if the key belongs to another vault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyspaceGuard {
    vault_id: VaultId,
    prefix: [u8; PREFIX_LEN],
}

impl KeyspaceGuard {
    /// Builds a guard for `vault_id`, deriving its keyspace prefix.
    pub fn new(vault_id: VaultId) -> Self {
        Self {
            vault_id,
            prefix: vault_prefix(&vault_id),
        }
    }

    /// The vault this guard scopes keys to.
    pub fn vault_id(&self) -> VaultId {
        self.vault_id
    }

    /// The 16-byte keyspace prefix every key for this vault carries.
    pub fn prefix(&self) -> &[u8; PREFIX_LEN] {
        &self.prefix
    }

    /// Encodes a storable CF key: `prefix ‖ cf_tag ‖ user_key`.
    ///
    /// This is the only path that produces a CF key for a vault-scoped
    /// operation. The `cf` tag round-trips exactly through [`decode_key`].
    pub fn encode_key(&self, cf: ColumnFamily, user_key: &[u8]) -> Vec<u8> {
        let tag = cf.keyspace_tag();
        let mut out = Vec::with_capacity(PREFIX_LEN + tag.len() + user_key.len());
        out.extend_from_slice(&self.prefix);
        out.extend_from_slice(&tag);
        out.extend_from_slice(user_key);
        out
    }

    /// Fast prefix check without allocating — used in range-scan filters.
    /// Returns `true` iff `raw` begins with this vault's full prefix.
    pub fn owns_key(&self, raw: &[u8]) -> bool {
        raw.get(..PREFIX_LEN) == Some(&self.prefix[..])
    }

    /// Decodes a raw CF key back into `(ColumnFamily, user_key)`, verifying it
    /// belongs to this vault.
    ///
    /// # Errors
    /// [`CALYX_VAULT_KEYSPACE_MISMATCH`] if the leading 16 bytes are not this
    /// vault's prefix, the key is shorter than the prefix, or the CF tag is
    /// malformed. Never returns another vault's key bytes (fail closed, A16).
    pub fn decode_key<'a>(&self, raw: &'a [u8]) -> Result<(ColumnFamily, &'a [u8])> {
        if !self.owns_key(raw) {
            return Err(keyspace_mismatch(format!(
                "key prefix does not match vault {} (got {} leading bytes)",
                self.vault_id,
                raw.len().min(PREFIX_LEN),
            )));
        }
        let rest = &raw[PREFIX_LEN..];
        ColumnFamily::parse_keyspace_tag(rest)
            .ok_or_else(|| keyspace_mismatch("malformed column-family tag after vault prefix"))
    }
}

/// Single-instance, per-vault write lock guarding WAL group-commit ordering so
/// concurrent cross-vault mutations cannot interleave a vault's keys.
///
/// Construct exactly one per vault and share it; cloning the lock would defeat
/// the mutual exclusion it exists to provide.
#[derive(Debug, Default)]
pub struct VaultWriteLock {
    inner: Mutex<()>,
}

impl VaultWriteLock {
    /// Creates an unlocked per-vault write lock.
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquires the lock, blocking until available. Released on guard drop.
    ///
    /// Recovers from poisoning (a panic while held does not wedge the vault):
    /// the lock protects ordering, not invariant-bearing data.
    pub fn lock(&self) -> VaultWriteLockGuard<'_> {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        VaultWriteLockGuard { _inner: guard }
    }

    /// Attempts to acquire without blocking; `None` if already held.
    pub fn try_lock(&self) -> Option<VaultWriteLockGuard<'_>> {
        match self.inner.try_lock() {
            Ok(guard) => Some(VaultWriteLockGuard { _inner: guard }),
            Err(std::sync::TryLockError::WouldBlock) => None,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => Some(VaultWriteLockGuard {
                _inner: poisoned.into_inner(),
            }),
        }
    }
}

/// RAII guard releasing the [`VaultWriteLock`] on drop.
#[derive(Debug)]
pub struct VaultWriteLockGuard<'a> {
    _inner: MutexGuard<'a, ()>,
}

fn keyspace_mismatch(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_VAULT_KEYSPACE_MISMATCH,
        message: message.into(),
        remediation: "read/write keys only through the owning vault's KeyspaceGuard",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cf::{CfRouter, SlotFamilyKind};
    use calyx_core::SlotId;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use ulid::Ulid;

    // Two synthetic vaults with hand-known ULID bytes → hand-known prefixes.
    const VAULT_A_BYTES: [u8; 16] = [0x0A; 16];
    const VAULT_B_BYTES: [u8; 16] = [0x0B; 16];

    fn vault(bytes: [u8; 16]) -> VaultId {
        VaultId::from_ulid(Ulid::from_bytes(bytes))
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn prefix_is_the_full_16_byte_ulid() {
        let guard = KeyspaceGuard::new(vault(VAULT_A_BYTES));
        assert_eq!(
            guard.prefix(),
            &VAULT_A_BYTES,
            "prefix must be the full ULID"
        );
        assert_eq!(PREFIX_LEN, 16);
    }

    #[test]
    fn encode_key_layout_matches_hand_computed_bytes() {
        let guard = KeyspaceGuard::new(vault(VAULT_A_BYTES));
        let encoded = guard.encode_key(ColumnFamily::Base, b"row1");
        // Expected: prefix(16×0x0A) ‖ cf_tag(Base = STATIC[0] = 0x00) ‖ "row1".
        let mut expected = Vec::new();
        expected.extend_from_slice(&[0x0A; 16]);
        expected.push(0x00);
        expected.extend_from_slice(b"row1");
        println!("encoded  = {}", hex(&encoded));
        println!("expected = {}", hex(&expected));
        assert_eq!(encoded, expected);
    }

    #[test]
    fn distinct_vaults_encode_same_user_key_to_different_bytes() {
        let a = KeyspaceGuard::new(vault(VAULT_A_BYTES));
        let b = KeyspaceGuard::new(vault(VAULT_B_BYTES));
        let ka = a.encode_key(ColumnFamily::Base, b"same-key");
        let kb = b.encode_key(ColumnFamily::Base, b"same-key");
        println!("vault A key = {}", hex(&ka));
        println!("vault B key = {}", hex(&kb));
        assert_ne!(
            ka, kb,
            "different vaults must never collide on an identical user key"
        );
        assert_eq!(&ka[..16], &VAULT_A_BYTES);
        assert_eq!(&kb[..16], &VAULT_B_BYTES);
    }

    #[test]
    fn cross_vault_decode_fails_closed() {
        let a = KeyspaceGuard::new(vault(VAULT_A_BYTES));
        let b = KeyspaceGuard::new(vault(VAULT_B_BYTES));
        let raw = a.encode_key(ColumnFamily::Base, b"secret");
        // Vault A decodes its own key.
        let (cf, user) = a.decode_key(&raw).expect("own decode");
        assert_eq!(cf, ColumnFamily::Base);
        assert_eq!(user, b"secret");
        // Vault B must NOT get vault A's user key back.
        let err = b.decode_key(&raw).unwrap_err();
        println!("cross-vault decode -> {}", err.code);
        assert_eq!(err.code, CALYX_VAULT_KEYSPACE_MISMATCH);
    }

    #[test]
    fn owns_key_boundary_at_offset_15() {
        let a = KeyspaceGuard::new(vault(VAULT_A_BYTES));
        let raw = a.encode_key(ColumnFamily::Base, b"k");
        assert!(a.owns_key(&raw));
        // Flip the last prefix byte (offset 15) → no longer owned.
        let mut tampered = raw.clone();
        tampered[15] ^= 0x01;
        assert!(
            !a.owns_key(&tampered),
            "single-byte prefix change must break ownership"
        );
    }

    #[test]
    fn round_trips_slot_cf_with_kind_and_index() {
        let a = KeyspaceGuard::new(vault(VAULT_A_BYTES));
        for cf in [
            ColumnFamily::slot(SlotId::new(0)),
            ColumnFamily::slot(SlotId::new(513)),
            ColumnFamily::slot_raw(SlotId::new(513)),
            ColumnFamily::Ledger,
            ColumnFamily::TimeIndex,
        ] {
            let raw = a.encode_key(cf, b"u");
            let (decoded, user) = a.decode_key(&raw).unwrap();
            assert_eq!(decoded, cf, "CF tag must round-trip exactly");
            assert_eq!(user, b"u");
        }
        // Quantized vs Raw for the same slot must not be confused.
        let q = a.encode_key(ColumnFamily::slot(SlotId::new(7)), b"u");
        let r = a.encode_key(ColumnFamily::slot_raw(SlotId::new(7)), b"u");
        assert_ne!(q, r);
        assert!(matches!(
            a.decode_key(&r).unwrap().0,
            ColumnFamily::Slot {
                kind: SlotFamilyKind::Raw,
                ..
            }
        ));
    }

    #[test]
    fn empty_user_key_and_prefix_only_edges() {
        let a = KeyspaceGuard::new(vault(VAULT_A_BYTES));
        // Empty user key: encodes to prefix ‖ cf_tag, decodes to empty slice.
        let raw = a.encode_key(ColumnFamily::Base, b"");
        assert_eq!(raw.len(), PREFIX_LEN + 1);
        let (cf, user) = a.decode_key(&raw).unwrap();
        assert_eq!(cf, ColumnFamily::Base);
        assert!(user.is_empty());
        // Exactly the prefix, no CF tag → malformed → fail closed.
        let err = a.decode_key(&VAULT_A_BYTES).unwrap_err();
        assert_eq!(err.code, CALYX_VAULT_KEYSPACE_MISMATCH);
    }

    #[test]
    fn all_zero_user_key_does_not_alias_another_vault() {
        // Vault whose ULID is all zeros; an all-zero user key must still stay
        // inside this vault and never be mistaken for a different vault.
        let zero = KeyspaceGuard::new(vault([0x00; 16]));
        let mut other_bytes = [0x00; 16];
        other_bytes[15] = 0x01;
        let other = KeyspaceGuard::new(vault(other_bytes));
        let raw = zero.encode_key(ColumnFamily::Base, &[0u8; 8]);
        assert!(zero.owns_key(&raw));
        assert!(
            !other.owns_key(&raw),
            "all-zero key must not alias the neighbouring vault"
        );
        assert_eq!(
            other.decode_key(&raw).unwrap_err().code,
            CALYX_VAULT_KEYSPACE_MISMATCH
        );
    }

    #[test]
    fn vaults_differing_only_in_last_ulid_byte_are_isolated() {
        let mut b_bytes = [0x00; 16];
        b_bytes[15] = 0x01;
        let a = KeyspaceGuard::new(vault([0x00; 16]));
        let b = KeyspaceGuard::new(vault(b_bytes));
        let ka = a.encode_key(ColumnFamily::Base, b"x");
        assert!(!b.owns_key(&ka));
        assert_eq!(
            b.decode_key(&ka).unwrap_err().code,
            CALYX_VAULT_KEYSPACE_MISMATCH
        );
    }

    #[test]
    fn short_key_fails_closed() {
        let a = KeyspaceGuard::new(vault(VAULT_A_BYTES));
        // 9 bytes: shorter than the 16-byte prefix.
        let err = a.decode_key(&[0x0A; 9]).unwrap_err();
        assert_eq!(err.code, CALYX_VAULT_KEYSPACE_MISMATCH);
        assert!(!a.owns_key(&[0x0A; 9]));
    }

    #[test]
    fn write_lock_is_mutually_exclusive() {
        let lock = VaultWriteLock::new();
        let held = lock.lock();
        // While held, a non-blocking acquire must fail.
        assert!(
            lock.try_lock().is_none(),
            "write lock must exclude a second holder"
        );
        drop(held);
        // After release, acquire succeeds.
        assert!(lock.try_lock().is_some(), "write lock must release on drop");
    }

    proptest::proptest! {
        #[test]
        fn decode_encode_round_trip(
            vault_bytes in proptest::array::uniform16(proptest::num::u8::ANY),
            cf_idx in 0usize..23,
            user_key in proptest::collection::vec(proptest::num::u8::ANY, 0..256),
        ) {
            let guard = KeyspaceGuard::new(vault(vault_bytes));
            let cf = ColumnFamily::STATIC[cf_idx];
            let raw = guard.encode_key(cf, &user_key);
            let (decoded_cf, decoded_user) = guard.decode_key(&raw).unwrap();
            proptest::prop_assert_eq!(decoded_cf, cf);
            proptest::prop_assert_eq!(decoded_user, user_key.as_slice());
        }
    }

    // ── Physical, on-disk FSV ────────────────────────────────────────────────
    // The truth gate: the encoded key must physically land in a real CF store
    // carrying the vault prefix, and a foreign vault's guard must reject it.

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn test_dir(name: &str) -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "calyx-aster-keyspace-{name}-{}-{id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn encoded_key_physically_persists_with_vault_prefix() {
        let dir = test_dir("persist");
        let a = KeyspaceGuard::new(vault(VAULT_A_BYTES));
        let b = KeyspaceGuard::new(vault(VAULT_B_BYTES));

        let key = a.encode_key(ColumnFamily::Base, b"physical-row");
        let value = b"payload-v1";

        let mut router = CfRouter::open(&dir, 1 << 20).unwrap();
        router.put(ColumnFamily::Base, &key, value).unwrap();

        // Separate READ of the source of truth: scan the CF store back.
        let rows = router.range(ColumnFamily::Base, b"", &[0xFF; 64]).unwrap();
        let stored = rows
            .iter()
            .find(|e| e.key == key)
            .expect("row must be on disk");
        println!("stored key   = {}", hex(&stored.key));
        println!("stored value = {}", hex(&stored.value));
        assert_eq!(stored.value, value);
        // The bytes physically on disk carry vault A's full prefix...
        assert_eq!(&stored.key[..16], &VAULT_A_BYTES);
        assert!(a.owns_key(&stored.key));
        // ...and vault B's guard structurally rejects the same physical bytes.
        assert!(!b.owns_key(&stored.key));
        assert_eq!(
            b.decode_key(&stored.key).unwrap_err().code,
            CALYX_VAULT_KEYSPACE_MISMATCH
        );
        // Vault A round-trips the physical bytes back to (CF, user_key).
        let (cf, user) = a.decode_key(&stored.key).unwrap();
        assert_eq!(cf, ColumnFamily::Base);
        assert_eq!(user, b"physical-row");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
