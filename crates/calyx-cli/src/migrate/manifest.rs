use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{Result, VaultId, content_address};
use serde::{Deserialize, Serialize};

use super::errors;
use super::reader::ChunkRow;

pub const MANIFEST_NAME: &str = "migration-manifest.json";
pub const PANEL_NAME: &str = "migration-panel.json";
pub const SCHEDULER_NAME: &str = "migration-backfill-scheduler.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MigrationManifest {
    pub schema_version: u32,
    pub vault_id: String,
    pub vault_salt_hex: String,
    pub sqlite_path_digest: String,
    pub panel_template: String,
    pub panel_version: u32,
    pub base_slot_id: u16,
    pub base_lens_id: String,
    pub source_rows: usize,
    pub migrated_rows: usize,
    pub created_at_ms: u64,
    pub scheduler_file: String,
}

impl MigrationManifest {
    pub fn load(vault_dir: &Path) -> Result<Self> {
        let path = manifest_path(vault_dir);
        let bytes = fs::read(&path).map_err(|err| errors::io("read manifest", &path, err))?;
        serde_json::from_slice(&bytes)
            .map_err(|err| errors::manifest(format!("{}: {err}", path.display())))
    }

    pub fn load_or_create(
        vault_dir: &Path,
        sqlite_path: &Path,
        rows: &[ChunkRow],
        base_lens_id: String,
        panel_version: u32,
    ) -> Result<Self> {
        let path = manifest_path(vault_dir);
        if path.exists() {
            return Self::load(vault_dir);
        }
        let seed = seed_bytes(sqlite_path, rows);
        let vault_seed = content_address([b"calyx-ph64-vault-id-v1".as_slice(), &seed]);
        let salt = content_address([b"calyx-ph64-vault-salt-v1".as_slice(), &seed]);
        let vault_id = VaultId::from_ulid(ulid::Ulid::from_bytes(vault_seed));
        Ok(Self {
            schema_version: 1,
            vault_id: vault_id.to_string(),
            vault_salt_hex: hex_encode(&salt),
            sqlite_path_digest: hex_encode(&content_address([sqlite_path_text(sqlite_path)])),
            panel_template: "text-default".to_string(),
            panel_version,
            base_slot_id: 0,
            base_lens_id,
            source_rows: rows.len(),
            migrated_rows: 0,
            created_at_ms: now_ms(),
            scheduler_file: SCHEDULER_NAME.to_string(),
        })
    }

    pub fn vault_id(&self) -> Result<VaultId> {
        VaultId::from_str(&self.vault_id)
            .map_err(|err| errors::manifest(format!("invalid vault_id: {err}")))
    }

    pub fn vault_salt(&self) -> Result<Vec<u8>> {
        hex_decode(&self.vault_salt_hex)
    }

    pub fn write(&self, vault_dir: &Path) -> Result<()> {
        fs::create_dir_all(vault_dir)
            .map_err(|err| errors::io("create vault dir", vault_dir, err))?;
        let path = manifest_path(vault_dir);
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|err| errors::manifest(format!("encode manifest: {err}")))?;
        fs::write(&path, bytes).map_err(|err| errors::io("write manifest", &path, err))
    }
}

pub fn manifest_path(vault_dir: &Path) -> PathBuf {
    vault_dir.join(MANIFEST_NAME)
}

pub fn panel_path(vault_dir: &Path) -> PathBuf {
    vault_dir.join(PANEL_NAME)
}

pub fn scheduler_path(vault_dir: &Path) -> PathBuf {
    vault_dir.join(SCHEDULER_NAME)
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

pub fn hex_decode(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(errors::manifest("hex field has odd length"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| {
            let hi = hex_value(chunk[0]).ok_or_else(|| errors::manifest("invalid hex field"))?;
            let lo = hex_value(chunk[1]).ok_or_else(|| errors::manifest("invalid hex field"))?;
            Ok((hi << 4) | lo)
        })
        .collect()
}

fn seed_bytes(sqlite_path: &Path, rows: &[ChunkRow]) -> Vec<u8> {
    let mut out = sqlite_path_text(sqlite_path).to_vec();
    for row in rows {
        out.extend_from_slice(row.database_name.as_bytes());
        out.push(0);
        out.extend_from_slice(row.chunk_id.as_bytes());
        out.push(0);
    }
    out
}

fn sqlite_path_text(path: &Path) -> &[u8] {
    path.as_os_str().as_encoded_bytes()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}
