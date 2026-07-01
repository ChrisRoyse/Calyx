use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, VaultId};

use super::{ResolvedVault, VaultIndex, VaultIndexEntry, index_path, read_index};
use crate::error::CliResult;

pub(crate) fn resolve_vault(home: &Path, vault: &str) -> CliResult<PathBuf> {
    resolve_vault_info(home, vault).map(|resolved| resolved.path)
}

pub(crate) fn resolve_vault_info(home: &Path, vault: &str) -> CliResult<ResolvedVault> {
    let index = read_index(home)?;
    let checked_index = index_path(home);
    let direct = PathBuf::from(vault);
    // A bare argument (one path component) is a logical vault reference —
    // vault id or CLI-index name — and must never be captured by an
    // incidental same-named entry in the process cwd (#1082). Filesystem
    // paths must be explicit: absolute, or multi-component like ./name.
    let explicit_path = direct.is_absolute() || direct.components().count() > 1;
    if explicit_path {
        if !direct.exists() {
            return Err(CalyxError::vault_access_denied(format!(
                "direct vault path {} does not exist; pass an existing vault directory, a vault id, or a name from CLI index {}",
                direct.display(),
                checked_index.display()
            ))
            .into());
        }
        if let Some(resolved) = resolve_direct_indexed(home, &index, &direct)? {
            return Ok(resolved);
        }
        let vault_id = direct
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.parse::<VaultId>().ok())
            .ok_or_else(|| {
                CalyxError::vault_access_denied(format!(
                    "direct vault path {} must end in a vault id or be present in CLI index {}",
                    direct.display(),
                    checked_index.display()
                ))
            })?;
        return Ok(ResolvedVault {
            path: direct,
            name: vault_id.to_string(),
            vault_id,
        });
    }
    if let Ok(vault_id) = vault.parse::<VaultId>() {
        let path = home.join("vaults").join(vault_id.to_string());
        if path.exists() {
            if let Some(entry) = index.vaults.iter().find(|entry| entry.vault_id == vault_id) {
                return Ok(resolve_entry(home, entry));
            }
            return Ok(ResolvedVault {
                path,
                name: vault_id.to_string(),
                vault_id,
            });
        }
    }
    if let Some(entry) = index
        .vaults
        .iter()
        .find(|entry| entry.name == vault || entry.vault_id.to_string() == vault)
    {
        let resolved = resolve_entry(home, entry);
        if resolved.path.exists() {
            return Ok(resolved);
        }
        return Err(CalyxError::vault_access_denied(format!(
            "vault {vault} resolved through CLI index {} to missing path {}",
            checked_index.display(),
            resolved.path.display()
        ))
        .into());
    }
    Err(CalyxError::vault_access_denied(format!(
        "vault {vault} does not exist; checked CLI index {} for vault ids and names; pass an absolute or ./-prefixed path for a direct vault directory",
        checked_index.display()
    ))
    .into())
}

fn resolve_direct_indexed(
    home: &Path,
    index: &VaultIndex,
    direct: &Path,
) -> CliResult<Option<ResolvedVault>> {
    let direct = direct.canonicalize()?;
    for entry in &index.vaults {
        let path = home.join(&entry.path);
        if path.exists() && path.canonicalize()? == direct {
            return Ok(Some(resolve_entry(home, entry)));
        }
    }
    Ok(None)
}

fn resolve_entry(home: &Path, entry: &VaultIndexEntry) -> ResolvedVault {
    ResolvedVault {
        path: home.join(&entry.path),
        name: entry.name.clone(),
        vault_id: entry.vault_id,
    }
}
