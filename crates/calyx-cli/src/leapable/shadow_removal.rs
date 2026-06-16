use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use calyx_core::{CalyxError, LedgerRef};
use calyx_ledger::{ActorId, EntryKind, SubjectId};
use calyx_registry::{PanelTemplate, civic_default, code_default, media_default, text_default};
use serde::{Deserialize, Serialize};

use super::dual_write::aster_dir;
use super::shadow_harness::read_shadow_manifest;
use super::{ShadowVault, VaultMode};
use crate::migrate;
use crate::migrate::manifest::{MigrationManifest, hex_encode};

mod cli;
mod panels;

pub(crate) use cli::run_remove_shadow;
#[cfg(test)]
pub(crate) use panels::DefaultPanelOptions;
pub(crate) use panels::DefaultPanels;

pub(crate) const CALYX_VAULT_FLIP_REQUIRED: &str = "CALYX_VAULT_FLIP_REQUIRED";
pub(crate) const CALYX_SHADOW_REMOVAL_FAILED: &str = "CALYX_SHADOW_REMOVAL_FAILED";
#[allow(dead_code)]
pub(crate) const CALYX_ROLLBACK_GATE_ALREADY_PASSED: &str = "CALYX_ROLLBACK_GATE_ALREADY_PASSED";
pub(crate) const CALYX_VAULT_TYPE_INVALID: &str = "CALYX_VAULT_TYPE_INVALID";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum VaultType {
    Text,
    Code,
    Civic,
    Media,
}

impl VaultType {
    fn template(self) -> PanelTemplate {
        match self {
            Self::Text => text_default(),
            Self::Code => code_default(),
            Self::Civic => civic_default(),
            Self::Media => media_default(),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Code => "code",
            Self::Civic => "civic",
            Self::Media => "media",
        }
    }
}

impl FromStr for VaultType {
    type Err = CalyxError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "text" => Ok(Self::Text),
            "code" => Ok(Self::Code),
            "civic" => Ok(Self::Civic),
            "media" => Ok(Self::Media),
            other => Err(error(
                CALYX_VAULT_TYPE_INVALID,
                format!("unknown vault type {other}"),
                "use one of: text, code, civic, media",
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PanelReceipt {
    pub(crate) vault_type: VaultType,
    pub(crate) template: String,
    pub(crate) lens_count: usize,
    pub(crate) backfill_pending: usize,
    pub(crate) panel_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RemovalReceipt {
    pub(crate) database_name: String,
    pub(crate) sqlite_path: PathBuf,
    pub(crate) archived_path: PathBuf,
    pub(crate) calyx_dir: PathBuf,
    pub(crate) calyx_only_at_seq: u64,
    pub(crate) ledger_ref: LedgerRef,
    pub(crate) rollback_gate_passed: bool,
}

pub(crate) struct ShadowRemoval;

impl ShadowRemoval {
    pub(crate) fn execute(vault: &mut ShadowVault) -> Result<RemovalReceipt, CalyxError> {
        if vault.mode() == VaultMode::Shadow {
            return Err(error(
                CALYX_VAULT_FLIP_REQUIRED,
                "vault is still in Shadow mode",
                "run calyx leapable read-flip before remove-shadow",
            ));
        }
        if vault.mode() == VaultMode::CalyxOnly {
            return existing_receipt(vault);
        }
        let (sqlite_path, calyx_dir) = vault.paths();
        let sqlite_path = sqlite_path.to_path_buf();
        let calyx_dir = calyx_dir.to_path_buf();
        let archived_path = archive_path(&sqlite_path);
        if sqlite_path.is_file() && archived_path.exists() {
            return Err(removal_failed(format!(
                "archive target {} already exists",
                archived_path.display()
            )));
        }

        vault.release_sqlite_for_archive()?;
        archive_sqlite(&sqlite_path, &archived_path)?;
        let (aster, manifest) = open_aster(&calyx_dir)?;
        let payload = serde_json::to_vec(&serde_json::json!({
            "event": "leapable_remove_shadow_v2",
            "database_name": vault.vault_name(),
            "vault_id": manifest.vault_id,
        }))
        .map_err(|error| removal_failed(format!("encode removal payload: {error}")))?;
        let subject = SubjectId::Query(
            blake3::hash(vault.vault_name().as_bytes())
                .as_bytes()
                .to_vec(),
        );
        let ledger_ref = aster.append_ledger_entry(
            EntryKind::Admin,
            subject,
            payload,
            ActorId::Service("calyx-leapable-remove-shadow".to_string()),
        )?;
        aster.flush()?;
        let receipt = RemovalReceipt {
            database_name: vault.vault_name().to_string(),
            sqlite_path: sqlite_path.clone(),
            archived_path: archived_path.clone(),
            calyx_dir: calyx_dir.clone(),
            calyx_only_at_seq: aster.latest_seq(),
            ledger_ref,
            rollback_gate_passed: false,
        };
        if let Err(error) =
            vault.set_mode_with_features(VaultMode::CalyxOnly, &receipt_features(&receipt))
        {
            let restore = rename_back(&archived_path, &sqlite_path);
            let message = match restore {
                Ok(()) => format!("write CalyxOnly MANIFEST: {}", error.message),
                Err(restore) => format!(
                    "write CalyxOnly MANIFEST: {}; restore archive failed: {restore}",
                    error.message
                ),
            };
            return Err(removal_failed(message));
        }
        Ok(receipt)
    }

    #[allow(dead_code)]
    pub(crate) fn rollback(receipt: &RemovalReceipt) -> Result<(), CalyxError> {
        if receipt.rollback_gate_passed {
            return Err(error(
                CALYX_ROLLBACK_GATE_ALREADY_PASSED,
                "V2 rollback gate already passed",
                "open a forward recovery issue; do not restore the sqlite shadow after V2 evidence",
            ));
        }
        if receipt.archived_path.is_file() {
            if receipt.sqlite_path.exists() {
                return Err(removal_failed(format!(
                    "cannot rollback because {} already exists",
                    receipt.sqlite_path.display()
                )));
            }
            fs::rename(&receipt.archived_path, &receipt.sqlite_path).map_err(|error| {
                removal_failed(format!(
                    "rename {} back to {}: {error}",
                    receipt.archived_path.display(),
                    receipt.sqlite_path.display()
                ))
            })?;
            sync_file(&receipt.sqlite_path)?;
            sync_parent(&receipt.sqlite_path)?;
        } else if !receipt.sqlite_path.is_file() {
            return Err(removal_failed(format!(
                "rollback source {} and restored sqlite {} are both absent",
                receipt.archived_path.display(),
                receipt.sqlite_path.display()
            )));
        }
        restore_calyx_mode_for_rollback(
            &receipt.calyx_dir,
            &[
                ("read_path", "calyx".to_string()),
                ("sqlite_shadow_archived", "false".to_string()),
                (
                    "rollback_from_calyx_only_at_seq",
                    receipt.calyx_only_at_seq.to_string(),
                ),
            ],
        )
    }
}

fn existing_receipt(vault: &ShadowVault) -> Result<RemovalReceipt, CalyxError> {
    let readback = vault.manifest_readback()?;
    let (sqlite_path, calyx_dir) = vault.paths();
    let archived_path = archive_path(sqlite_path);
    Ok(RemovalReceipt {
        database_name: readback.database_name,
        sqlite_path: sqlite_path.to_path_buf(),
        archived_path,
        calyx_dir: calyx_dir.to_path_buf(),
        calyx_only_at_seq: parse_u64(&readback.features, "calyx_only_at_seq").unwrap_or(0),
        ledger_ref: LedgerRef {
            seq: parse_u64(&readback.features, "calyx_only_ledger_seq").unwrap_or(0),
            hash: parse_hash(&readback.features, "calyx_only_ledger_hash").unwrap_or([0; 32]),
        },
        rollback_gate_passed: false,
    })
}

fn archive_sqlite(sqlite_path: &Path, archived_path: &Path) -> Result<(), CalyxError> {
    if sqlite_path.is_file() {
        fs::rename(sqlite_path, archived_path).map_err(|error| {
            removal_failed(format!(
                "rename {} to {}: {error}",
                sqlite_path.display(),
                archived_path.display()
            ))
        })?;
        sync_file(archived_path)?;
        sync_parent(archived_path)?;
        return Ok(());
    }
    if archived_path.is_file() {
        return Ok(());
    }
    Err(removal_failed(format!(
        "sqlite source {} and archive {} are both absent",
        sqlite_path.display(),
        archived_path.display()
    )))
}

fn rename_back(archived_path: &Path, sqlite_path: &Path) -> std::io::Result<()> {
    if archived_path.is_file() && !sqlite_path.exists() {
        fs::rename(archived_path, sqlite_path)?;
    }
    Ok(())
}

fn receipt_features(receipt: &RemovalReceipt) -> Vec<(&'static str, String)> {
    vec![
        ("read_path", "calyx-only".to_string()),
        ("sqlite_vec_role", "cold-archive".to_string()),
        ("sqlite_shadow_archived", "true".to_string()),
        (
            "sqlite_archive_path",
            receipt.archived_path.display().to_string(),
        ),
        ("calyx_only_at_seq", receipt.calyx_only_at_seq.to_string()),
        ("calyx_only_ledger_seq", receipt.ledger_ref.seq.to_string()),
        (
            "calyx_only_ledger_hash",
            hex_encode(&receipt.ledger_ref.hash),
        ),
    ]
}

fn open_aster(
    calyx_dir: &Path,
) -> Result<(calyx_aster::vault::AsterVault, MigrationManifest), CalyxError> {
    let aster_dir = aster_dir(calyx_dir);
    let manifest = MigrationManifest::load(&aster_dir)?;
    let vault = migrate::open_vault(&aster_dir, &manifest)?;
    Ok((vault, manifest))
}

fn archive_path(sqlite_path: &Path) -> PathBuf {
    let mut archive = sqlite_path.as_os_str().to_os_string();
    archive.push(".archive");
    PathBuf::from(archive)
}

fn sync_file(path: &Path) -> Result<(), CalyxError> {
    File::options()
        .read(true)
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| removal_failed(format!("sync {}: {error}", path.display())))
}

fn sync_parent(path: &Path) -> Result<(), CalyxError> {
    let parent = path
        .parent()
        .ok_or_else(|| removal_failed(format!("{} has no parent dir", path.display())))?;
    #[cfg(windows)]
    {
        let _ = parent;
        Ok(())
    }
    #[cfg(not(windows))]
    File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|error| removal_failed(format!("sync dir {}: {error}", parent.display())))
}

#[allow(dead_code)]
fn restore_calyx_mode_for_rollback(
    vault: &Path,
    entries: &[(&str, String)],
) -> Result<(), CalyxError> {
    let readback = read_shadow_manifest(vault)?;
    let mut features = readback.features;
    for (key, value) in entries {
        features.insert((*key).to_string(), value.clone());
    }
    let manifest = RollbackManifest {
        schema_version: 1,
        mode: VaultMode::Calyx,
        database_name: readback.database_name,
        sqlite_path_digest: readback.sqlite_path_digest,
        calyx_chunk_count: readback.chunk_count,
        created_at_ms: 0,
        features,
    };
    let tmp = vault.join("MANIFEST.tmp");
    let path = vault.join("MANIFEST");
    let mut bytes = b"CXSHDW1!".to_vec();
    bytes.push(1);
    bytes.extend_from_slice(
        &serde_json::to_vec(&manifest)
            .map_err(|error| removal_failed(format!("encode rollback manifest: {error}")))?,
    );
    fs::write(&tmp, bytes)
        .map_err(|error| removal_failed(format!("write {}: {error}", tmp.display())))?;
    sync_file(&tmp)?;
    fs::rename(&tmp, &path)
        .map_err(|error| removal_failed(format!("rename {}: {error}", path.display())))?;
    sync_parent(&path)
}

#[derive(Serialize)]
#[allow(dead_code)]
struct RollbackManifest {
    schema_version: u32,
    mode: VaultMode,
    database_name: String,
    sqlite_path_digest: String,
    calyx_chunk_count: u64,
    created_at_ms: u64,
    features: std::collections::BTreeMap<String, String>,
}

fn parse_u64(map: &std::collections::BTreeMap<String, String>, key: &str) -> Option<u64> {
    map.get(key)?.parse().ok()
}

fn parse_hash(map: &std::collections::BTreeMap<String, String>, key: &str) -> Option<[u8; 32]> {
    let bytes = crate::migrate::manifest::hex_decode(map.get(key)?).ok()?;
    bytes.try_into().ok()
}

fn removal_failed(message: impl Into<String>) -> CalyxError {
    error(
        CALYX_SHADOW_REMOVAL_FAILED,
        message,
        "inspect MANIFEST mode byte, sqlite archive path, and Aster ledger before retrying",
    )
}

fn error(code: &'static str, message: impl Into<String>, remediation: &'static str) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
