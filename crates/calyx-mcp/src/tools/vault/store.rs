use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, VaultId};
use serde::{Deserialize, Serialize};

use crate::server::ToolResult;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct VaultIndex {
    pub(super) vaults: Vec<VaultIndexEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct VaultIndexEntry {
    pub(super) name: String,
    pub(super) vault_id: VaultId,
    pub(super) path: String,
    pub(super) panel_template: String,
}

#[derive(Clone, Debug)]
pub(in crate::tools) struct ResolvedVault {
    pub(in crate::tools) path: PathBuf,
    pub(in crate::tools) name: String,
    pub(in crate::tools) vault_id: VaultId,
}

pub(in crate::tools) fn home_dir() -> ToolResult<PathBuf> {
    env::var_os("CALYX_HOME")
        .map(PathBuf::from)
        .ok_or_else(|| CalyxError::vault_access_denied("CALYX_HOME is required").into())
}

pub(super) fn read_index(home: &Path) -> ToolResult<VaultIndex> {
    let path = index_path(home);
    if !path.exists() {
        return Ok(VaultIndex::default());
    }
    let bytes = fs::read(&path).map_err(|err| disk_error("read vault index", err))?;
    serde_json::from_slice(&bytes)
        .map_err(|err| CalyxError::aster_corrupt_shard(format!("decode vault index: {err}")).into())
}

pub(super) fn write_index(home: &Path, index: &VaultIndex) -> ToolResult<()> {
    let path = index_path(home);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| disk_error("create vault index dir", err))?;
    }
    let bytes = serde_json::to_vec_pretty(index)
        .map_err(|err| CalyxError::aster_corrupt_shard(format!("encode vault index: {err}")))?;
    fs::write(&path, bytes).map_err(|err| disk_error("write vault index", err))?;
    Ok(())
}

pub(super) fn resolve_vault(home: &Path, vault: &str) -> ToolResult<PathBuf> {
    resolve_vault_info(home, vault).map(|resolved| resolved.path)
}

pub(in crate::tools) fn resolve_vault_info(home: &Path, vault: &str) -> ToolResult<ResolvedVault> {
    let index = read_index(home)?;
    let direct = PathBuf::from(vault);
    if direct.exists() {
        if let Some(resolved) = resolve_direct_indexed(home, &index, &direct)? {
            return Ok(resolved);
        }
        let vault_id = direct
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.parse::<VaultId>().ok())
            .ok_or_else(|| {
                CalyxError::vault_access_denied(
                    "direct vault path must end in a vault id or be present in the index",
                )
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
    }
    Err(CalyxError::vault_access_denied(format!("vault {vault} does not exist")).into())
}

pub(in crate::tools) fn vault_salt(vault_id: VaultId, name: &str) -> Vec<u8> {
    format!("calyx-cli-vault:{vault_id}:{name}").into_bytes()
}

fn resolve_direct_indexed(
    home: &Path,
    index: &VaultIndex,
    direct: &Path,
) -> ToolResult<Option<ResolvedVault>> {
    let direct = direct
        .canonicalize()
        .map_err(|err| disk_error("canonicalize vault path", err))?;
    for entry in &index.vaults {
        let path = home.join(&entry.path);
        if path.exists()
            && path
                .canonicalize()
                .map_err(|err| disk_error("canonicalize indexed vault", err))?
                == direct
        {
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

fn index_path(home: &Path) -> PathBuf {
    home.join("vaults").join("index.json")
}

fn disk_error(context: &str, error: std::io::Error) -> CalyxError {
    CalyxError::disk_pressure(format!("{context}: {error}"))
}
