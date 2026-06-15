use std::path::{Path, PathBuf};

use calyx_core::LedgerRef;
use serde::Serialize;

use calyx_core::CalyxError;

use super::{
    CALYX_SHADOW_REMOVAL_FAILED, CALYX_VAULT_FLIP_REQUIRED, DefaultPanels, PanelReceipt,
    RemovalReceipt, ShadowRemoval, VaultType,
};
use crate::error::{CliError, CliResult};
use crate::leapable::ShadowVault;
use crate::leapable::shadow_harness::{ShadowManifestReadback, VaultMode, read_shadow_manifest};
use crate::migrate::manifest::hex_decode;
use crate::output::print_json;

#[derive(Clone, Debug, PartialEq, Serialize)]
struct RemoveShadowReport {
    sqlite_path: PathBuf,
    calyx_dir: PathBuf,
    manifest_before: ShadowManifestReadback,
    panel: PanelReceipt,
    removal: RemovalReceipt,
    manifest_after: ShadowManifestReadback,
    gate: String,
}

pub(crate) fn run_remove_shadow(args: &[String]) -> CliResult {
    let args = parse_args(args)?;
    let Some(before) = preexisting_manifest(&args.calyx)? else {
        return Err(flip_required().into());
    };
    if before.mode == VaultMode::Shadow {
        return Err(flip_required().into());
    }
    if before.mode == VaultMode::CalyxOnly {
        let removal = removal_from_manifest(&args.sqlite, &args.calyx, &before)?;
        let panel = panel_from_manifest(&args.calyx, args.vault_type, &before)?;
        let after = read_shadow_manifest(&args.calyx)?;
        let report = RemoveShadowReport {
            sqlite_path: args.sqlite,
            calyx_dir: args.calyx,
            manifest_before: before,
            panel,
            removal,
            manifest_after: after,
            gate: "PASS".to_string(),
        };
        print_json(&report)?;
        return Ok(());
    }
    preflight_archive_state(&args.sqlite)?;
    let archive = archive_path(&args.sqlite);
    let mut vault = if !args.sqlite.is_file() && archive.is_file() {
        ShadowVault::open_with_archived_sqlite(&args.sqlite, &archive, &args.calyx)?
    } else {
        ShadowVault::open(&args.sqlite, &args.calyx)?
    };
    let panel = match DefaultPanels::install(&mut vault, args.vault_type) {
        Ok(panel) => panel,
        Err(error) => {
            let _ = vault.close();
            return Err(error.into());
        }
    };
    let removal = match ShadowRemoval::execute(&mut vault) {
        Ok(receipt) => receipt,
        Err(error) => {
            let _ = vault.close();
            return Err(error.into());
        }
    };
    let after = read_shadow_manifest(&args.calyx)?;
    let report = RemoveShadowReport {
        sqlite_path: args.sqlite,
        calyx_dir: args.calyx,
        manifest_before: before,
        panel,
        removal,
        manifest_after: after,
        gate: "PASS".to_string(),
    };
    print_json(&report)?;
    vault.close()?;
    Ok(())
}

fn preflight_archive_state(sqlite_path: &Path) -> Result<(), CalyxError> {
    let archived_path = archive_path(sqlite_path);
    if sqlite_path.is_file() && archived_path.exists() {
        return Err(removal_failed(format!(
            "archive target {} already exists",
            archived_path.display()
        )));
    }
    if !sqlite_path.is_file() && !archived_path.is_file() {
        return Err(removal_failed(format!(
            "sqlite source {} and archive {} are both absent",
            sqlite_path.display(),
            archived_path.display()
        )));
    }
    Ok(())
}

fn archive_path(sqlite_path: &Path) -> PathBuf {
    let mut archive = sqlite_path.as_os_str().to_os_string();
    archive.push(".archive");
    PathBuf::from(archive)
}

fn parse_args(args: &[String]) -> CliResult<RemoveShadowArgs> {
    let mut out = RemoveShadowArgsParse::default();
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--sqlite" => out.sqlite = value(args, idx, "--sqlite")?.into(),
            "--calyx" | "--vault" => out.calyx = value(args, idx, "--calyx")?.into(),
            "--vault-type" => {
                out.vault_type = Some(value(args, idx, "--vault-type")?.parse()?);
            }
            other => {
                return Err(CliError::usage(format!(
                    "unknown remove-shadow arg {other}"
                )));
            }
        }
        idx += 2;
    }
    if out.sqlite.as_os_str().is_empty()
        || out.calyx.as_os_str().is_empty()
        || out.vault_type.is_none()
    {
        return Err(CliError::usage(
            "usage: calyx leapable remove-shadow --sqlite <db> (--calyx|--vault) <dir> --vault-type <text|code|civic|media>",
        ));
    }
    Ok(RemoveShadowArgs {
        sqlite: out.sqlite,
        calyx: out.calyx,
        vault_type: out.vault_type.expect("vault_type checked"),
    })
}

fn preexisting_manifest(calyx_dir: &Path) -> Result<Option<ShadowManifestReadback>, CalyxError> {
    if calyx_dir.join("MANIFEST").is_file() {
        return read_shadow_manifest(calyx_dir).map(Some);
    }
    Ok(None)
}

fn removal_from_manifest(
    sqlite_path: &Path,
    calyx_dir: &Path,
    manifest: &ShadowManifestReadback,
) -> Result<RemovalReceipt, CalyxError> {
    let archived_path = manifest
        .features
        .get("sqlite_archive_path")
        .map(PathBuf::from)
        .unwrap_or_else(|| archive_path(sqlite_path));
    Ok(RemovalReceipt {
        database_name: manifest.database_name.clone(),
        sqlite_path: sqlite_path.to_path_buf(),
        archived_path,
        calyx_dir: calyx_dir.to_path_buf(),
        calyx_only_at_seq: parse_feature_u64(manifest, "calyx_only_at_seq")?,
        ledger_ref: LedgerRef {
            seq: parse_feature_u64(manifest, "calyx_only_ledger_seq")?,
            hash: parse_feature_hash(manifest, "calyx_only_ledger_hash")?,
        },
        rollback_gate_passed: false,
    })
}

fn panel_from_manifest(
    calyx_dir: &Path,
    vault_type: VaultType,
    manifest: &ShadowManifestReadback,
) -> Result<PanelReceipt, CalyxError> {
    Ok(PanelReceipt {
        vault_type,
        template: required_feature(manifest, "default_panel_template")?.to_string(),
        lens_count: required_feature(manifest, "default_panel_lens_count")?
            .parse()
            .map_err(|error| removal_failed(format!("parse default_panel_lens_count: {error}")))?,
        backfill_pending: required_feature(manifest, "default_panel_backfill_pending")?
            .parse()
            .map_err(|error| {
                removal_failed(format!("parse default_panel_backfill_pending: {error}"))
            })?,
        panel_path: calyx_dir.join("aster").join("migration-panel.json"),
    })
}

fn required_feature<'a>(
    manifest: &'a ShadowManifestReadback,
    key: &str,
) -> Result<&'a str, CalyxError> {
    manifest
        .features
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| removal_failed(format!("CalyxOnly manifest missing {key}")))
}

fn parse_feature_u64(manifest: &ShadowManifestReadback, key: &str) -> Result<u64, CalyxError> {
    required_feature(manifest, key)?
        .parse()
        .map_err(|error| removal_failed(format!("parse {key}: {error}")))
}

fn parse_feature_hash(
    manifest: &ShadowManifestReadback,
    key: &str,
) -> Result<[u8; 32], CalyxError> {
    let bytes = hex_decode(required_feature(manifest, key)?)
        .map_err(|error| removal_failed(format!("parse {key}: {error}")))?;
    bytes
        .try_into()
        .map_err(|_| removal_failed(format!("{key} is not 32 bytes")))
}

fn flip_required() -> CalyxError {
    CalyxError {
        code: CALYX_VAULT_FLIP_REQUIRED,
        message: "vault is still in Shadow mode".to_string(),
        remediation: "run calyx leapable read-flip before remove-shadow",
    }
}

fn removal_failed(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_SHADOW_REMOVAL_FAILED,
        message: message.into(),
        remediation: "inspect MANIFEST mode byte, sqlite archive path, and Aster ledger before retrying",
    }
}

fn value<'a>(args: &'a [String], idx: usize, flag: &str) -> CliResult<&'a str> {
    args.get(idx + 1)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

#[derive(Default)]
struct RemoveShadowArgsParse {
    sqlite: PathBuf,
    calyx: PathBuf,
    vault_type: Option<VaultType>,
}

struct RemoveShadowArgs {
    sqlite: PathBuf,
    calyx: PathBuf,
    vault_type: VaultType,
}
