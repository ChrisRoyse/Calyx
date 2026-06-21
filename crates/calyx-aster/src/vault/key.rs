//! Per-vault key derivation and authenticated encryption (PH60 · T01).
//!
//! [`VaultKey`] is the innermost cryptographic layer of the defense-in-depth
//! tenant-isolation stack (key + keyspace + grant — PRD `30 §2`). It derives a
//! unique AES-256-GCM key per vault from host-provided key material using
//! HKDF-SHA-256, then provides authenticated encryption that **fails closed**
//! (a tampered tag or truncated ciphertext returns an error — never a silent
//! zero-fill or garbage plaintext, per axiom A16).
//!
//! Key material lives in [`zeroize::Zeroizing`], so the raw 32 bytes are wiped
//! from memory on drop and are never cloned into a static. `Clone` is
//! intentionally **not** derived: a secret should have exactly one owner.

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce};
use calyx_core::{CalyxError, Result, VaultId};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::{Zeroize, Zeroizing};

/// Host-provided key material was empty, so no vault key could be derived.
pub const CALYX_VAULT_KEY_MISSING: &str = "CALYX_VAULT_KEY_MISSING";
/// AES-256-GCM encryption failed inside the cipher.
pub const CALYX_ENCRYPTION_FAILED: &str = "CALYX_ENCRYPTION_FAILED";
/// AES-256-GCM decryption failed: tag mismatch, wrong AAD, or truncated input.
pub const CALYX_DECRYPTION_FAILED: &str = "CALYX_DECRYPTION_FAILED";

/// HKDF salt — domain-separates Calyx vault keys from any other HKDF use of the
/// same master material. Versioned so a future scheme can rotate without
/// colliding with `-v1` keys.
const VAULT_KEY_SALT: &[u8] = b"calyx-vault-key-v1";

/// AES-256 key length in bytes.
const KEY_LEN: usize = 32;
/// AES-GCM nonce length in bytes (96-bit, the GCM-standard size).
const NONCE_LEN: usize = 12;
/// AES-GCM authentication tag length in bytes (128-bit).
const TAG_LEN: usize = 16;

/// A per-vault AES-256-GCM key derived from host master material.
///
/// Holds 32 secret bytes in [`Zeroizing`], wiped on drop. Not `Clone`.
pub struct VaultKey {
    inner: Zeroizing<[u8; KEY_LEN]>,
}

impl VaultKey {
    /// Derives a per-vault key from host `master` material via HKDF-SHA-256.
    ///
    /// `ikm = master`, `salt = VAULT_KEY_SALT`, `info = vault_id` ULID bytes —
    /// so two vaults with the same master get distinct keys, and the same
    /// `(master, vault_id)` always derives the identical key (determinism that
    /// the golden-vector test pins).
    ///
    /// # Errors
    /// [`CALYX_VAULT_KEY_MISSING`] if `master` is empty.
    pub fn derive(master: &[u8], vault_id: &VaultId) -> Result<Self> {
        if master.is_empty() {
            return Err(vault_key_missing(
                "host-provided master key material is empty",
            ));
        }
        let info = vault_id.as_ulid().to_bytes();
        let hk = Hkdf::<Sha256>::new(Some(VAULT_KEY_SALT), master);
        let mut okm = Zeroizing::new([0_u8; KEY_LEN]);
        // HKDF-expand only fails when the requested length exceeds 255*HashLen
        // (255*32 = 8160 bytes). KEY_LEN is 32, so this branch is unreachable;
        // we still surface it as an error rather than unwrap, to stay fail-loud.
        hk.expand(&info, okm.as_mut_slice())
            .map_err(|err| CalyxError {
                code: CALYX_VAULT_KEY_MISSING,
                message: format!("HKDF-SHA-256 expand failed: {err}"),
                remediation: "report a bug: HKDF output length is fixed at 32 bytes",
            })?;
        Ok(Self { inner: okm })
    }

    /// Wraps a pre-derived 32-byte key directly for cryptographic KATs.
    #[cfg(test)]
    pub(crate) fn from_raw(bytes: [u8; KEY_LEN]) -> Self {
        Self {
            inner: Zeroizing::new(bytes),
        }
    }

    /// Overwrites this live key with the all-zero erasure sentinel.
    pub(crate) fn shred_for_erasure(&mut self) {
        self.inner.as_mut_slice().zeroize();
    }

    /// Reports whether this live key has been overwritten by erasure.
    pub(crate) fn is_shredded_for_erasure(&self) -> bool {
        self.inner.iter().all(|byte| *byte == 0)
    }

    /// Zero-copy borrow of the inner bytes as an AES-256-GCM key reference.
    pub fn aes_gcm_key(&self) -> &Key<Aes256Gcm> {
        Key::<Aes256Gcm>::from_slice(self.inner.as_slice())
    }

    /// AES-256-GCM encrypts `plaintext` under `nonce`, binding `aad`.
    ///
    /// Returns `ciphertext || tag` (the 16-byte GCM tag is appended). This is a
    /// crate-internal primitive; public value encryption goes through
    /// `VaultContext::encrypt_value`, which generates and stores a fresh nonce.
    ///
    /// # Errors
    /// [`CALYX_ENCRYPTION_FAILED`] on any cipher error.
    pub(crate) fn encrypt(
        &self,
        nonce: &[u8; NONCE_LEN],
        plaintext: &[u8],
        aad: &[u8],
    ) -> Result<Vec<u8>> {
        let cipher = Aes256Gcm::new(self.aes_gcm_key());
        cipher
            .encrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| encryption_failed("AES-256-GCM encryption failed"))
    }

    /// AES-256-GCM decrypts `ciphertext` (`= ct || tag`) under `nonce`, verifying
    /// `aad` and the authentication tag.
    ///
    /// Fails closed (A16): a flipped tag byte, wrong AAD, wrong nonce, or input
    /// shorter than the 16-byte tag all return an error — never garbage bytes.
    ///
    /// # Errors
    /// [`CALYX_DECRYPTION_FAILED`] on tag/AAD mismatch or truncated input.
    pub(crate) fn decrypt(
        &self,
        nonce: &[u8; NONCE_LEN],
        ciphertext: &[u8],
        aad: &[u8],
    ) -> Result<Vec<u8>> {
        if ciphertext.len() < TAG_LEN {
            return Err(decryption_failed(format!(
                "ciphertext is {} bytes, shorter than the {TAG_LEN}-byte GCM tag",
                ciphertext.len()
            )));
        }
        let cipher = Aes256Gcm::new(self.aes_gcm_key());
        cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad,
                },
            )
            .map_err(|_| decryption_failed("AES-256-GCM tag verification failed"))
    }
}

impl std::fmt::Debug for VaultKey {
    /// Never prints the secret bytes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultKey")
            .field("inner", &"<redacted>")
            .finish()
    }
}

fn vault_key_missing(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_VAULT_KEY_MISSING,
        message: message.into(),
        remediation: "provision non-empty master key material for the vault",
    }
}

fn encryption_failed(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ENCRYPTION_FAILED,
        message: message.into(),
        remediation: "verify the nonce is 12 bytes and plaintext is within size limits",
    }
}

fn decryption_failed(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_DECRYPTION_FAILED,
        message: message.into(),
        remediation: "verify the key, nonce, AAD, and that the ciphertext is intact",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ulid::Ulid;

    /// Fixed synthetic master material (32 bytes) — the `2+2` known input.
    const MASTER: &[u8] = b"calyx-test-master-key-0123456789";
    /// Fixed synthetic ULID bytes `01 02 .. 10` → HKDF `info`.
    const VAULT_ULID_BYTES: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
    /// Golden HKDF-SHA-256 output, computed independently by Python
    /// `cryptography.hazmat.primitives.kdf.hkdf.HKDF` (the external oracle).
    const GOLDEN_KEY: [u8; 32] = [
        0x60, 0x7c, 0x8e, 0xbc, 0x1e, 0xc3, 0x2a, 0xb6, 0x18, 0x63, 0x95, 0x7e, 0xe4, 0xc4, 0x83,
        0x67, 0x7f, 0x37, 0xcb, 0x4e, 0x43, 0x6a, 0x46, 0x8c, 0xfa, 0x02, 0x9f, 0x45, 0xa5, 0xf9,
        0xfb, 0x28,
    ];
    const NONCE: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
    const PLAINTEXT: &[u8] = b"the quick brown fox";
    const AAD: &[u8] = b"calyx-aad-v1";
    /// Golden AES-256-GCM `ciphertext||tag` for (GOLDEN_KEY, NONCE, PLAINTEXT,
    /// AAD), computed independently by Python `AESGCM`. Fixed nonce here is a
    /// determinism known-answer test ONLY — production never reuses a nonce.
    const GOLDEN_CT: [u8; 35] = [
        0x45, 0x26, 0xaf, 0x52, 0x4a, 0xe4, 0x07, 0x59, 0x42, 0x44, 0xfb, 0x99, 0x17, 0xad, 0xbc,
        0x26, 0xd4, 0xb9, 0x8f, 0x7f, 0xb7, 0x7e, 0x14, 0x5d, 0xd0, 0x0f, 0x8f, 0x7a, 0x6d, 0xf8,
        0x4f, 0xdf, 0x3e, 0x43, 0xd8,
    ];

    fn test_vault_id() -> VaultId {
        VaultId::from_ulid(Ulid::from_bytes(VAULT_ULID_BYTES))
    }

    #[test]
    fn derive_matches_golden_hkdf_vector() {
        let key = VaultKey::derive(MASTER, &test_vault_id()).expect("derive");
        let derived = *key.inner;
        println!("derived key = {}", hex(&derived));
        println!("golden  key = {}", hex(&GOLDEN_KEY));
        assert_eq!(
            derived, GOLDEN_KEY,
            "HKDF-SHA-256 derivation drifted from golden vector"
        );
    }

    #[test]
    fn derive_is_deterministic_and_vault_scoped() {
        let a = VaultKey::derive(MASTER, &test_vault_id()).unwrap();
        let b = VaultKey::derive(MASTER, &test_vault_id()).unwrap();
        assert_eq!(
            *a.inner, *b.inner,
            "same (master, vault) must derive same key"
        );

        let other = VaultId::from_ulid(Ulid::from_bytes([99; 16]));
        let c = VaultKey::derive(MASTER, &other).unwrap();
        assert_ne!(
            *a.inner, *c.inner,
            "different vault_id must derive a different key"
        );
    }

    #[test]
    fn encrypt_matches_golden_ciphertext() {
        let key = VaultKey::from_raw(GOLDEN_KEY);
        let ct = key.encrypt(&NONCE, PLAINTEXT, AAD).expect("encrypt");
        println!("ciphertext = {}", hex(&ct));
        println!("golden  ct = {}", hex(&GOLDEN_CT));
        assert_eq!(ct.as_slice(), GOLDEN_CT.as_slice());
        assert_eq!(ct.len(), PLAINTEXT.len() + TAG_LEN);
    }

    #[test]
    fn encrypt_decrypt_round_trips() {
        let key = VaultKey::from_raw(GOLDEN_KEY);
        let ct = key.encrypt(&NONCE, PLAINTEXT, AAD).unwrap();
        let pt = key.decrypt(&NONCE, &ct, AAD).unwrap();
        assert_eq!(pt, PLAINTEXT);
    }

    #[test]
    fn empty_master_fails_closed() {
        let err = VaultKey::derive(b"", &test_vault_id()).unwrap_err();
        assert_eq!(err.code, CALYX_VAULT_KEY_MISSING);
    }

    #[test]
    fn empty_plaintext_encrypts_to_tag_only() {
        let key = VaultKey::from_raw(GOLDEN_KEY);
        let ct = key.encrypt(&NONCE, b"", AAD).unwrap();
        assert_eq!(
            ct.len(),
            TAG_LEN,
            "empty plaintext → 16-byte tag-only ciphertext"
        );
        let pt = key.decrypt(&NONCE, &ct, AAD).unwrap();
        assert!(pt.is_empty());
    }

    #[test]
    fn wrong_aad_fails_closed() {
        let key = VaultKey::from_raw(GOLDEN_KEY);
        let ct = key.encrypt(&NONCE, PLAINTEXT, AAD).unwrap();
        let err = key.decrypt(&NONCE, &ct, b"different-aad").unwrap_err();
        assert_eq!(err.code, CALYX_DECRYPTION_FAILED);
    }

    #[test]
    fn flipped_tag_byte_fails_closed() {
        let key = VaultKey::from_raw(GOLDEN_KEY);
        let mut ct = key.encrypt(&NONCE, PLAINTEXT, AAD).unwrap();
        let last = ct.len() - 1;
        ct[last] ^= 0x01;
        let err = key.decrypt(&NONCE, &ct, AAD).unwrap_err();
        assert_eq!(
            err.code, CALYX_DECRYPTION_FAILED,
            "tampered tag must not yield plaintext"
        );
    }

    #[test]
    fn truncated_ciphertext_fails_closed() {
        let key = VaultKey::from_raw(GOLDEN_KEY);
        let err = key.decrypt(&NONCE, &[0_u8; TAG_LEN - 1], AAD).unwrap_err();
        assert_eq!(err.code, CALYX_DECRYPTION_FAILED);
    }

    #[test]
    fn drop_zeroizes_secret_bytes() {
        use std::mem::ManuallyDrop;
        // Sound white-box probe of the zeroize-on-drop invariant. Reading freed
        // stack memory through a dangling pointer would be undefined behavior
        // (the optimizer may keep the value in a register and never reflect the
        // in-place wipe), so instead we keep the allocation *live* on the heap:
        //   1. `Box::into_raw` hands us ownership of a live allocation.
        //   2. `drop_in_place` runs ONLY `Zeroizing`'s destructor — it wipes the
        //      32 bytes in place but does NOT deallocate.
        //   3. We read those still-allocated bytes as `u8` (every bit pattern is
        //      a valid `u8`, so this read is well-defined) and confirm zeros.
        //   4. `Box::from_raw` into `ManuallyDrop` reclaims the allocation
        //      without re-running the (already-run) destructor.
        let key = VaultKey::from_raw([0xAA_u8; KEY_LEN]);
        let raw: *mut VaultKey = Box::into_raw(Box::new(key));
        // SAFETY: `raw` is a live, aligned, initialized allocation from `Box`.
        let byte_ptr = unsafe { (*raw).inner.as_ptr() };
        // SAFETY: same live allocation; bytes are initialized to 0xAA.
        assert_eq!(
            unsafe { *byte_ptr },
            0xAA,
            "precondition: bytes set before drop"
        );
        // SAFETY: `raw` points to a live, initialized `VaultKey`; this runs its
        // destructor (zeroizing `inner` in place) exactly once and leaves the
        // backing allocation intact.
        unsafe { std::ptr::drop_in_place(raw) };
        // SAFETY: the allocation is still live (only the destructor ran, not the
        // free); we read 32 initialized `u8`s from it.
        let after = unsafe { std::slice::from_raw_parts(byte_ptr, KEY_LEN) };
        assert_eq!(
            after, &[0_u8; KEY_LEN],
            "secret bytes must be zeroized on drop"
        );
        // SAFETY: reclaim the allocation without re-running the destructor.
        unsafe { drop(Box::from_raw(raw as *mut ManuallyDrop<VaultKey>)) };
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    proptest::proptest! {
        #[test]
        fn decrypt_encrypt_round_trip_any_plaintext(plaintext in proptest::collection::vec(proptest::num::u8::ANY, 0..4096)) {
            let key = VaultKey::from_raw(GOLDEN_KEY);
            let ct = key.encrypt(&NONCE, &plaintext, AAD).unwrap();
            let pt = key.decrypt(&NONCE, &ct, AAD).unwrap();
            proptest::prop_assert_eq!(pt, plaintext);
        }
    }
}
