use serde::Deserialize;
use serde_json::{Value, json};

use super::Engine;
use super::error::{EngineResult, parse_params, vault_open_error};
use super::verify::{VerifyMode, lifecycle_progress, maybe_verify_path};
use crate::lifecycle::{copy_dir_new, remove_dir};
use crate::paths::{
    VaultRef, resolve_existing_snapshot_dir, resolve_existing_vault_dir, resolve_snapshot_dir,
    resolve_vault_target_dir,
};

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
        let (source_dir, latest_seq) = match self.vaults.get_mut(vault_ref.as_str()) {
            Some(handle) => {
                handle.touch(params.ts);
                handle.flush_now()?;
                (handle.dir.clone(), Some(handle.vault.latest_seq()))
            }
            None => (
                resolve_existing_vault_dir(&self.config.data_dir, &vault_ref)?,
                None,
            ),
        };
        let snapshot_dir = resolve_snapshot_dir(&self.config.data_dir, &vault_ref, &snapshot_ref)?;
        let source_verify = if params.verify.verifies_source() {
            lifecycle_progress("vault.snapshot", "verify_source", vault_ref.as_str());
            maybe_verify_path(true, &source_dir)?
        } else {
            None
        };
        lifecycle_progress("vault.snapshot", "copy", vault_ref.as_str());
        let copy = copy_dir_new(&source_dir, &snapshot_dir)?;
        let verify = if params.verify.verifies_target() {
            lifecycle_progress("vault.snapshot", "verify_target", vault_ref.as_str());
            maybe_verify_path(true, &snapshot_dir)?
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
            maybe_verify_path(true, &snapshot_dir)?
        } else {
            None
        };
        lifecycle_progress("vault.restore", "copy", vault_ref.as_str());
        let copy = copy_dir_new(&snapshot_dir, &target_dir)?;
        let restored_verify = if params.verify.verifies_target() {
            lifecycle_progress("vault.restore", "verify_target", vault_ref.as_str());
            maybe_verify_path(true, &target_dir)?
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
        let target_dir = resolve_vault_target_dir(&self.config.data_dir, &target_vault_ref)?;
        let source_verify = if params.verify.verifies_source() {
            lifecycle_progress("vault.clone", "verify_source", vault_ref.as_str());
            maybe_verify_path(true, &source_dir)?
        } else {
            None
        };
        lifecycle_progress("vault.clone", "copy", vault_ref.as_str());
        let copy = copy_dir_new(&source_dir, &target_dir)?;
        let clone_verify = if params.verify.verifies_target() {
            lifecycle_progress("vault.clone", "verify_target", target_vault_ref.as_str());
            maybe_verify_path(true, &target_dir)?
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
}
