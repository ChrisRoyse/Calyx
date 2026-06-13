//! Security utilities for PH60 tenant isolation: the outermost ZFS crypto-at-rest
//! probe and the lens-store cross-vault guard.

pub mod lens_store;
pub mod zfs;

pub use lens_store::{
    CALYX_LENS_CROSS_VAULT, LensStoreGuard, assert_no_cross_vault_vector,
    content_id_is_vault_agnostic,
};
pub use zfs::{
    ZfsEncryptionStatus, assert_encrypted_or_warn, classify_zfs_output, operator_guidance,
    probe_zfs_encryption,
};
