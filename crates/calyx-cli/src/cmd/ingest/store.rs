use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CalyxError, CxId, VaultStore};

use super::super::vault::{ResolvedVault, home_dir, resolve_vault_info, vault_salt};
use crate::error::CliResult;

pub(super) fn resolve_cli_vault(vault: &str) -> CliResult<ResolvedVault> {
    let home = home_dir()?;
    resolve_vault_info(&home, vault)
}

pub(super) fn open_vault(resolved: &ResolvedVault) -> CliResult<AsterVault> {
    Ok(AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::default(),
    )?)
}

pub(super) fn ensure_base_exists(vault: &AsterVault, cx_id: CxId) -> CliResult {
    if base_exists(vault, cx_id)? {
        return Ok(());
    }
    Err(CalyxError::vault_access_denied(format!("cx_id {cx_id} does not exist in vault")).into())
}

pub(super) fn base_exists(vault: &AsterVault, cx_id: CxId) -> CliResult<bool> {
    Ok(vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Base, &base_key(cx_id))?
        .is_some())
}
