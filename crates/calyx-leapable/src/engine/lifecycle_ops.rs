use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Clock};
use serde::Deserialize;
use serde_json::{Value, json};

use super::Engine;
use super::error::{EngineResult, parse_params, vault_open_error};
use super::identity::{SALT_FILE_NAME, salt_for_dir};
use super::verify::{VerifyMode, lifecycle_progress, maybe_verify_path_with_crypto};
use crate::lifecycle::{CopyStats, copy_dir_new, lifecycle_error, remove_dir};
use crate::paths::{
    VaultRef, resolve_existing_snapshot_dir, resolve_existing_vault_dir, resolve_snapshot_dir,
    resolve_vault_target_dir,
};

const CALYX_LEAPABLE_CLONE_FAILED: &str = "CALYX_LEAPABLE_CLONE_FAILED";
const CLONE_WRITE_CHUNK_ROWS: usize = 512;
type CloneRow = (ColumnFamily, Vec<u8>, Vec<u8>);

#[derive(Deserialize)]
struct SnapshotParams {
    vault_ref: String,
    snapshot_ref: String,
    ts: calyx_core::Ts,
    #[serde(default)]
    verify: VerifyMode,
}

#[derive(Deserialize)]
struct RestoreParams {
    vault_ref: String,
    snapshot_ref: String,
    ts: calyx_core::Ts,
    #[serde(default)]
    overwrite: bool,
    #[serde(default)]
    verify: VerifyMode,
}

#[derive(Deserialize)]
struct CloneParams {
    vault_ref: String,
    target_vault_ref: String,
    ts: calyx_core::Ts,
    #[serde(default)]
    verify: VerifyMode,
}

impl Engine {
    pub(super) fn vault_snapshot(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<SnapshotParams>(params, "vault.snapshot")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let snapshot_ref = VaultRef::parse(&params.snapshot_ref)?;
        let (source_dir, latest_seq, source_context) = match self.vaults.get_mut(vault_ref.as_str())
        {
            Some(handle) => {
                handle.touch(params.ts);
                handle.flush_now()?;
                (
                    handle.dir.clone(),
                    Some(handle.vault.latest_seq()),
                    handle.context.clone(),
                )
            }
            None => {
                let dir = resolve_existing_vault_dir(&self.config.data_dir, &vault_ref)?;
                let context = self.context_for_path(&vault_ref, &dir)?;
                (dir, None, context)
            }
        };
        let snapshot_dir = resolve_snapshot_dir(&self.config.data_dir, &vault_ref, &snapshot_ref)?;
        let source_verify = if params.verify.verifies_source() {
            lifecycle_progress("vault.snapshot", "verify_source", vault_ref.as_str());
            maybe_verify_path_with_crypto(true, &source_dir, &source_context)?
        } else {
            None
        };
        lifecycle_progress("vault.snapshot", "copy", vault_ref.as_str());
        let copy = copy_dir_new(&source_dir, &snapshot_dir)?;
        let verify = if params.verify.verifies_target() {
            lifecycle_progress("vault.snapshot", "verify_target", vault_ref.as_str());
            maybe_verify_path_with_crypto(true, &snapshot_dir, &source_context)?
        } else {
            None
        };
        Ok(json!({
            "status": "snapshotted",
            "vault_ref": vault_ref.as_str(),
            "verify": params.verify.as_str(),
            "snapshot_ref": snapshot_ref.as_str(),
            "source_dir": source_dir,
            "snapshot_dir": snapshot_dir,
            "latest_seq": latest_seq,
            "copy": copy.to_value(),
            "source_verify_restore": source_verify,
            "verify_restore": verify
        }))
    }

    pub(super) fn vault_restore(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<RestoreParams>(params, "vault.restore")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let snapshot_ref = VaultRef::parse(&params.snapshot_ref)?;
        if self.vaults.contains_key(vault_ref.as_str()) {
            return Err(vault_open_error(vault_ref.as_str()).into());
        }
        let snapshot_dir =
            resolve_existing_snapshot_dir(&self.config.data_dir, &vault_ref, &snapshot_ref)?;
        let context = self.context_for_path(&vault_ref, &snapshot_dir)?;
        let target_dir = if params.overwrite {
            if self
                .config
                .data_dir
                .join(vault_ref.storage_dir_name())
                .exists()
            {
                let existing = resolve_existing_vault_dir(&self.config.data_dir, &vault_ref)?;
                remove_dir(&existing)?;
            }
            resolve_vault_target_dir(&self.config.data_dir, &vault_ref)?
        } else {
            resolve_vault_target_dir(&self.config.data_dir, &vault_ref)?
        };
        let source_verify = if params.verify.verifies_source() {
            lifecycle_progress("vault.restore", "verify_source", vault_ref.as_str());
            maybe_verify_path_with_crypto(true, &snapshot_dir, &context)?
        } else {
            None
        };
        lifecycle_progress("vault.restore", "copy", vault_ref.as_str());
        let copy = copy_dir_new(&snapshot_dir, &target_dir)?;
        let restored_verify = if params.verify.verifies_target() {
            lifecycle_progress("vault.restore", "verify_target", vault_ref.as_str());
            maybe_verify_path_with_crypto(true, &target_dir, &context)?
        } else {
            None
        };
        Ok(json!({
            "status": "restored",
            "vault_ref": vault_ref.as_str(),
            "verify": params.verify.as_str(),
            "snapshot_ref": snapshot_ref.as_str(),
            "snapshot_dir": snapshot_dir,
            "vault_dir": target_dir,
            "restored_at": params.ts,
            "overwrite": params.overwrite,
            "copy": copy.to_value(),
            "source_verify_restore": source_verify,
            "verify_restore": restored_verify
        }))
    }

    pub(super) fn vault_clone(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<CloneParams>(params, "vault.clone")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let target_vault_ref = VaultRef::parse(&params.target_vault_ref)?;
        if self.vaults.contains_key(target_vault_ref.as_str()) {
            return Err(vault_open_error(target_vault_ref.as_str()).into());
        }
        if let Some(handle) = self.vaults.get_mut(vault_ref.as_str()) {
            handle.touch(params.ts);
            handle.flush_now()?;
        }
        let source_dir = resolve_existing_vault_dir(&self.config.data_dir, &vault_ref)?;
        let source_context = self.context_for_path(&vault_ref, &source_dir)?;
        let target_dir = resolve_vault_target_dir(&self.config.data_dir, &target_vault_ref)?;
        let source_verify = if params.verify.verifies_source() {
            lifecycle_progress("vault.clone", "verify_source", vault_ref.as_str());
            maybe_verify_path_with_crypto(true, &source_dir, &source_context)?
        } else {
            None
        };
        lifecycle_progress("vault.clone", "copy", vault_ref.as_str());
        let copy = self.clone_reencrypted(
            &vault_ref,
            &target_vault_ref,
            &source_dir,
            &target_dir,
            params.ts,
        )?;
        let target_context = self.context_for_path(&target_vault_ref, &target_dir)?;
        let clone_verify = if params.verify.verifies_target() {
            lifecycle_progress("vault.clone", "verify_target", target_vault_ref.as_str());
            maybe_verify_path_with_crypto(true, &target_dir, &target_context)?
        } else {
            None
        };
        Ok(json!({
            "status": "cloned",
            "vault_ref": vault_ref.as_str(),
            "target_vault_ref": target_vault_ref.as_str(),
            "verify": params.verify.as_str(),
            "source_dir": source_dir,
            "target_dir": target_dir,
            "cloned_at": params.ts,
            "copy": copy.to_value(),
            "source_verify_restore": source_verify,
            "verify_restore": clone_verify
        }))
    }

    fn clone_reencrypted(
        &mut self,
        vault_ref: &VaultRef,
        target_vault_ref: &VaultRef,
        source_dir: &Path,
        target_dir: &Path,
        ts: calyx_core::Ts,
    ) -> EngineResult<CopyStats> {
        let source_salt = salt_for_dir(source_dir, vault_ref.as_str())?;
        fs::create_dir_all(target_dir).map_err(|error| {
            clone_error(
                format!("create clone target {}: {error}", target_dir.display()),
                "choose a writable Leapable data directory and retry vault.clone",
            )
        })?;
        fs::write(target_dir.join(SALT_FILE_NAME), &source_salt).map_err(|error| {
            clone_error(
                format!("write clone salt {}: {error}", target_dir.display()),
                "ensure the target vault directory is writable and retry vault.clone",
            )
        })?;

        let result = (|| {
            let rows = self.collect_clone_rows(vault_ref, source_dir, ts)?;
            let mut target =
                self.open_handle(target_vault_ref.clone(), target_dir.to_path_buf(), ts)?;
            for chunk in rows.chunks(CLONE_WRITE_CHUNK_ROWS) {
                target.vault.write_cf_batch(chunk.iter().cloned())?;
            }
            target.flush_now()?;
            dir_stats(target_dir)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(target_dir);
        }
        result
    }

    fn collect_clone_rows(
        &mut self,
        vault_ref: &VaultRef,
        source_dir: &Path,
        ts: calyx_core::Ts,
    ) -> EngineResult<Vec<CloneRow>> {
        if let Some(handle) = self.vaults.get_mut(vault_ref.as_str()) {
            handle.touch(ts);
            handle.flush_now()?;
            collect_visible_rows(&handle.vault, source_dir)
        } else {
            let mut source = self.open_handle(vault_ref.clone(), source_dir.to_path_buf(), ts)?;
            source.flush_now()?;
            collect_visible_rows(&source.vault, source_dir)
        }
    }
}

fn collect_visible_rows<C: Clock>(
    vault: &AsterVault<C>,
    source_dir: &Path,
) -> EngineResult<Vec<CloneRow>> {
    let snapshot = vault.latest_seq();
    let mut rows = Vec::new();
    for cf in clone_column_families(source_dir)? {
        if cf == ColumnFamily::TimeIndex {
            continue;
        }
        rows.extend(
            vault
                .scan_cf_at(snapshot, cf)?
                .into_iter()
                .map(|(key, value)| (cf, key, value)),
        );
    }
    Ok(rows)
}

fn clone_column_families(source_dir: &Path) -> EngineResult<Vec<ColumnFamily>> {
    let mut cfs = BTreeSet::from(ColumnFamily::STATIC);
    let cf_root = source_dir.join("cf");
    if cf_root.is_dir() {
        for entry in fs::read_dir(&cf_root).map_err(|error| {
            clone_error(
                format!("read source CF dir {}: {error}", cf_root.display()),
                "verify the source vault directory and retry vault.clone",
            )
        })? {
            let entry = entry.map_err(|error| {
                clone_error(
                    format!("read source CF dir entry {}: {error}", cf_root.display()),
                    "verify the source vault directory and retry vault.clone",
                )
            })?;
            if !entry.path().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let cf = ColumnFamily::from_name(&name).ok_or_else(|| {
                clone_error(
                    format!("unrecognized source CF directory {name:?}"),
                    "open the source vault with a compatible Calyx build before cloning",
                )
            })?;
            cfs.insert(cf);
        }
    }
    Ok(cfs.into_iter().collect())
}

fn dir_stats(path: &Path) -> EngineResult<CopyStats> {
    let mut stats = CopyStats {
        dirs: 1,
        files: 0,
        bytes: 0,
    };
    for entry in fs::read_dir(path).map_err(|error| {
        clone_error(
            format!("read clone target {}: {error}", path.display()),
            "verify the clone target directory and retry vault.clone",
        )
    })? {
        let entry = entry.map_err(|error| {
            clone_error(
                format!("read clone target entry {}: {error}", path.display()),
                "verify the clone target directory and retry vault.clone",
            )
        })?;
        let file_type = entry.file_type().map_err(|error| {
            clone_error(
                format!("stat clone target {}: {error}", entry.path().display()),
                "verify the clone target directory and retry vault.clone",
            )
        })?;
        if file_type.is_dir() {
            let child = dir_stats(&entry.path())?;
            stats.dirs += child.dirs;
            stats.files += child.files;
            stats.bytes += child.bytes;
        } else if file_type.is_file() {
            stats.files += 1;
            stats.bytes += entry
                .metadata()
                .map_err(|error| {
                    clone_error(
                        format!("stat clone target file {}: {error}", entry.path().display()),
                        "verify the clone target directory and retry vault.clone",
                    )
                })?
                .len();
        }
    }
    Ok(stats)
}

fn clone_error(message: impl Into<String>, remediation: &'static str) -> CalyxError {
    lifecycle_error(CALYX_LEAPABLE_CLONE_FAILED, message, remediation)
}
