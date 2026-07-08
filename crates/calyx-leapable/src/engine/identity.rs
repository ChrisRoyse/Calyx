use calyx_core::{VaultId, content_address};
use ulid::Ulid;

pub(super) fn vault_id_for(vault_ref: &str) -> VaultId {
    VaultId::from_ulid(Ulid::from_bytes(content_address([vault_ref.as_bytes()])))
}

pub(super) fn salt_for(vault_ref: &str) -> Vec<u8> {
    content_address([
        b"calyx-leapable-vault-salt".as_slice(),
        vault_ref.as_bytes(),
    ])
    .to_vec()
}
