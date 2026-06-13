//! Lens-store cross-vault guard (PH60 · T06).
//!
//! Lens *weights* are content-addressed by [`LensId`] and shared across all
//! vaults — that is intentional and safe (the weights carry no tenant data).
//! What must **never** cross a tenant boundary is a *materialised vector*: the
//! embedding produced for a specific vault's constellation. [`LensStoreGuard`]
//! is checked at every point where a stored vector is about to be copied or
//! returned to a caller, blocking with [`CALYX_LENS_CROSS_VAULT`] if the
//! vector's owning vault differs from the requesting vault (fail closed, A16).

use calyx_core::{CalyxError, LensId, Result, VaultId};

/// A lens-store guard scoped to the vault making the request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LensStoreGuard {
    requesting_vault: VaultId,
}

/// A materialised constellation vector from one vault was about to be returned
/// to a different vault. Blocked unconditionally — no config flag relaxes it.
pub const CALYX_LENS_CROSS_VAULT: &str = "CALYX_LENS_CROSS_VAULT";

impl LensStoreGuard {
    /// Builds a guard for the vault that is requesting lens-store access.
    pub fn new(requesting_vault: VaultId) -> Self {
        Self { requesting_vault }
    }

    /// The vault this guard is scoped to.
    pub fn requesting_vault(&self) -> VaultId {
        self.requesting_vault
    }
}

/// Asserts that a vector owned by `embedding_vault` may be returned to the
/// guard's requesting vault.
///
/// # Errors
/// [`CALYX_LENS_CROSS_VAULT`] iff `embedding_vault != guard.requesting_vault`.
/// This is unconditional — there is no flag that permits a cross-vault vector
/// to be materialised (A16).
pub fn assert_no_cross_vault_vector(
    guard: &LensStoreGuard,
    embedding_vault: VaultId,
) -> Result<()> {
    if embedding_vault != guard.requesting_vault {
        return Err(lens_cross_vault(format!(
            "vector owned by vault {embedding_vault} cannot be returned to vault {}",
            guard.requesting_vault,
        )));
    }
    Ok(())
}

/// Lens *weights* are content-addressed and vault-agnostic by construction: the
/// same [`LensId`] denotes identical weights for every vault. This documents the
/// invariant; it is always `true` (the vault scoping is on vectors, not weights).
pub fn content_id_is_vault_agnostic(_lens_id: &LensId) -> bool {
    true
}

fn lens_cross_vault(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_LENS_CROSS_VAULT,
        message: message.into(),
        remediation: "materialise vectors only for the vault that owns them; \
                      use an explicit cross-vault grant for shared reads",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ulid::Ulid;

    fn vault(byte: u8) -> VaultId {
        VaultId::from_ulid(Ulid::from_bytes([byte; 16]))
    }

    #[test]
    fn same_vault_vector_is_allowed() {
        let a = vault(0xA1);
        let guard = LensStoreGuard::new(a);
        assert!(assert_no_cross_vault_vector(&guard, a).is_ok());
    }

    #[test]
    fn cross_vault_vector_is_blocked() {
        let (a, b) = (vault(0xA1), vault(0xB2));
        let guard = LensStoreGuard::new(a);
        let err = assert_no_cross_vault_vector(&guard, b).unwrap_err();
        println!(
            "assert_no_cross_vault_vector(guard_A, vault_B) = Err({})",
            err.code
        );
        assert_eq!(err.code, CALYX_LENS_CROSS_VAULT);
    }

    #[test]
    fn cross_vault_is_blocked_for_every_foreign_vault() {
        // Fail-closed: no foreign vault is ever silently allowed.
        let guard = LensStoreGuard::new(vault(0x00));
        for byte in 1u8..=255 {
            assert_eq!(
                assert_no_cross_vault_vector(&guard, vault(byte))
                    .unwrap_err()
                    .code,
                CALYX_LENS_CROSS_VAULT,
                "foreign vault {byte:#x} must be blocked"
            );
        }
        // The one matching vault is allowed.
        assert!(assert_no_cross_vault_vector(&guard, vault(0x00)).is_ok());
    }

    #[test]
    fn lens_weights_are_vault_agnostic() {
        let lens = LensId::from_parts("sem-self", b"weights", b"corpus", b"768xf32");
        assert!(
            content_id_is_vault_agnostic(&lens),
            "lens weights are shared by design"
        );
    }
}
