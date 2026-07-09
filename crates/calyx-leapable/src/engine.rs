//! Direct JSON-RPC method dispatch for the Leapable engine sidecar.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

use calyx_aster::txn::TxnHandle;
use calyx_aster::vault::{AsterVault, QuotaConfig, VaultContext, VaultOptions};
use calyx_core::{CalyxError, Ts, VaultId};
use calyx_mcp::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use serde::Deserialize;
use serde_json::{Value, json};

use self::clock::EngineClock;
use self::error::{EngineError, EngineResult, parse_params, vault_not_open, vault_open_error};
use self::identity::{salt_for, vault_id_for};
use crate::config::EngineConfig;
use crate::lifecycle::{
    CALYX_LEAPABLE_VAULT_ALREADY_EXISTS, CALYX_LEAPABLE_VAULT_OPEN, copy_dir_new, lifecycle_error,
    remove_dir, verify_restore_value,
};
use crate::paths::{
    VaultRef, list_vault_refs, resolve_existing_snapshot_dir, resolve_existing_vault_dir,
    resolve_new_vault_dir, resolve_snapshot_dir, resolve_vault_target_dir,
};

/// Compile-time capability map: end-user binary is CPU-only.
pub const LEAPABLE_CAPABILITIES: &[(&str, bool)] = &[
    ("cpu-only", true),
    ("hnsw-ram", true),
    ("cuda", false),
    ("diskann", false),
    ("spann", false),
];

const PANIC_PROBE_ENV: &str = "CALYX_LEAPABLE_ENABLE_PANIC_PROBE";
const ZFS_DATASET_UNAVAILABLE: &str = "leapable/local";

/// Local code for requests that address a vault before `vault.open`.
pub const CALYX_LEAPABLE_VAULT_NOT_OPEN: &str = "CALYX_LEAPABLE_VAULT_NOT_OPEN";
/// Local code for the env-gated panic probe.
pub const CALYX_LEAPABLE_PANIC_PROBE_DISABLED: &str = "CALYX_LEAPABLE_PANIC_PROBE_DISABLED";

/// One process owns all opened vault contexts.
pub struct Engine {
    config: EngineConfig,
    vaults: BTreeMap<String, VaultHandle>,
}

struct VaultHandle {
    vault_ref: VaultRef,
    vault_id: VaultId,
    dir: PathBuf,
    opened_at: Ts,
    last_ts: Ts,
    requests: u64,
    clock: EngineClock,
    txn: TxnHandle,
    context: VaultContext,
    vault: AsterVault<EngineClock>,
}

impl VaultHandle {
    fn touch(&mut self, ts: Ts) {
        self.requests += 1;
        self.last_ts = ts;
        self.clock.set(ts);
    }
}

#[derive(Deserialize)]
struct VaultParams {
    vault_ref: String,
    ts: Ts,
}

#[derive(Deserialize)]
struct SnapshotParams {
    vault_ref: String,
    snapshot_ref: String,
    ts: Ts,
}

#[derive(Deserialize)]
struct RestoreParams {
    vault_ref: String,
    snapshot_ref: String,
    ts: Ts,
    #[serde(default)]
    overwrite: bool,
}

#[derive(Deserialize)]
struct CloneParams {
    vault_ref: String,
    target_vault_ref: String,
    ts: Ts,
}

impl Engine {
    /// Creates an engine with no open vaults.
    pub fn new(config: EngineConfig) -> Self {
        Self {
            config,
            vaults: BTreeMap::new(),
        }
    }

    /// Dispatches one decoded request, catching panics at the request boundary.
    pub fn dispatch(&mut self, request: JsonRpcRequest) -> JsonRpcResponse {
        let id = request.id.clone();
        let outcome = catch_unwind(AssertUnwindSafe(|| self.dispatch_inner(request)));
        match outcome {
            Ok(response) => response,
            Err(_) => JsonRpcResponse::error(id, JsonRpcError::internal("internal server error")),
        }
    }

    fn dispatch_inner(&mut self, request: JsonRpcRequest) -> JsonRpcResponse {
        let id = request.id.clone();
        let result = match request.method.as_str() {
            "engine.info" => self.engine_info(),
            "vault.create" => self.vault_create(request.params),
            "vault.open" => self.vault_open(request.params),
            "vault.close" => self.vault_close(request.params),
            "vault.list" => self.vault_list(),
            "vault.delete" => self.vault_delete(request.params),
            "vault.snapshot" => self.vault_snapshot(request.params),
            "vault.restore" => self.vault_restore(request.params),
            "vault.clone" => self.vault_clone(request.params),
            "vault.verify" => self.vault_verify(request.params),
            "vault.stat" => self.vault_stat(request.params),
            "cx.put" => self.cx_put(request.params),
            "cx.put_batch" => self.cx_put_batch(request.params),
            "cx.get" => self.cx_get(request.params),
            "cx.scan" => self.cx_scan(request.params),
            "cx.anchor" => self.cx_anchor(request.params),
            "cx.delete" => self.cx_delete(request.params),
            method if storage::is_storage_method(method) => {
                self.dispatch_storage(method, request.params)
            }
            "engine.panic_probe" => self.panic_probe(),
            other => {
                return JsonRpcResponse::error(id, JsonRpcError::method_not_found(other));
            }
        };
        match result {
            Ok(value) => JsonRpcResponse::success(id, value),
            Err(EngineError::InvalidParams(message)) => {
                JsonRpcResponse::error(id, JsonRpcError::invalid_params(message))
            }
            Err(EngineError::Calyx(error)) => {
                JsonRpcResponse::error(id, JsonRpcError::from_calyx(&error))
            }
        }
    }

    fn engine_info(&self) -> EngineResult<Value> {
        Ok(json!({
            "engine": "calyx-leapable",
            "transport": "stdio-jsonrpc-2.0-ndjson",
            "data_dir": self.config.data_dir,
            "open_vaults": self.vaults.keys().collect::<Vec<_>>(),
            "cpu_profile": {
                "cpu_only": true,
                "hnsw": "ram",
                "cuda": false,
                "diskann": false,
                "spann": false
            }
        }))
    }

    fn vault_create(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<VaultParams>(params, "vault.create")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        if self.vaults.contains_key(vault_ref.as_str()) {
            return Err(lifecycle_error(
                CALYX_LEAPABLE_VAULT_ALREADY_EXISTS,
                format!("vault_ref {:?} is already open", vault_ref.as_str()),
                "close the open handle or choose a different vault_ref",
            )
            .into());
        }
        let dir = resolve_new_vault_dir(&self.config.data_dir, &vault_ref)?;
        let handle = self.open_handle(vault_ref.clone(), dir, params.ts)?;
        handle.vault.flush()?;
        let value = vault_handle_value("created", &handle);
        self.vaults.insert(vault_ref.as_str().to_string(), handle);
        Ok(value)
    }

    fn vault_open(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<VaultParams>(params, "vault.open")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        if let Some(handle) = self.vaults.get_mut(vault_ref.as_str()) {
            handle.touch(params.ts);
            return Ok(vault_handle_value("already_open", handle));
        }
        let dir = resolve_existing_vault_dir(&self.config.data_dir, &vault_ref)?;
        let handle = self.open_handle(vault_ref.clone(), dir, params.ts)?;
        let value = vault_handle_value("opened", &handle);
        self.vaults.insert(vault_ref.as_str().to_string(), handle);
        Ok(value)
    }

    fn vault_close(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<VaultParams>(params, "vault.close")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let Some(mut handle) = self.vaults.remove(vault_ref.as_str()) else {
            return Err(vault_not_open(vault_ref.as_str()).into());
        };
        handle.touch(params.ts);
        handle.vault.flush()?;
        Ok(json!({
            "status": "closed",
            "vault_ref": handle.vault_ref.as_str(),
            "vault_id": handle.vault_id.to_string(),
            "vault_dir": handle.dir,
            "latest_seq": handle.vault.latest_seq(),
            "last_ts": handle.last_ts,
            "requests": handle.requests
        }))
    }

    fn vault_list(&self) -> EngineResult<Value> {
        let vaults = list_vault_refs(&self.config.data_dir)?
            .into_iter()
            .map(|(vault_ref, dir)| {
                json!({
                    "vault_ref": vault_ref.as_str(),
                    "vault_dir": dir,
                    "open": self.vaults.contains_key(vault_ref.as_str())
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "status": "listed",
            "vaults": vaults
        }))
    }

    fn vault_delete(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<VaultParams>(params, "vault.delete")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        if self.vaults.contains_key(vault_ref.as_str()) {
            return Err(vault_open_error(vault_ref.as_str()).into());
        }
        let dir = resolve_existing_vault_dir(&self.config.data_dir, &vault_ref)?;
        remove_dir(&dir)?;
        Ok(json!({
            "status": "deleted",
            "vault_ref": vault_ref.as_str(),
            "vault_dir": dir,
            "deleted_at": params.ts
        }))
    }

    fn vault_snapshot(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<SnapshotParams>(params, "vault.snapshot")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let snapshot_ref = VaultRef::parse(&params.snapshot_ref)?;
        let (source_dir, latest_seq) = match self.vaults.get_mut(vault_ref.as_str()) {
            Some(handle) => {
                handle.touch(params.ts);
                handle.vault.flush()?;
                (handle.dir.clone(), Some(handle.vault.latest_seq()))
            }
            None => (
                resolve_existing_vault_dir(&self.config.data_dir, &vault_ref)?,
                None,
            ),
        };
        let snapshot_dir = resolve_snapshot_dir(&self.config.data_dir, &vault_ref, &snapshot_ref)?;
        let copy = copy_dir_new(&source_dir, &snapshot_dir)?;
        let verify = verify_restore_value(&snapshot_dir)?;
        Ok(json!({
            "status": "snapshotted",
            "vault_ref": vault_ref.as_str(),
            "snapshot_ref": snapshot_ref.as_str(),
            "source_dir": source_dir,
            "snapshot_dir": snapshot_dir,
            "latest_seq": latest_seq,
            "copy": copy.to_value(),
            "verify_restore": verify
        }))
    }

    fn vault_restore(&mut self, params: Option<Value>) -> EngineResult<Value> {
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
        let source_verify = verify_restore_value(&snapshot_dir)?;
        let copy = copy_dir_new(&snapshot_dir, &target_dir)?;
        let restored_verify = verify_restore_value(&target_dir)?;
        Ok(json!({
            "status": "restored",
            "vault_ref": vault_ref.as_str(),
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

    fn vault_clone(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<CloneParams>(params, "vault.clone")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let target_vault_ref = VaultRef::parse(&params.target_vault_ref)?;
        if self.vaults.contains_key(target_vault_ref.as_str()) {
            return Err(vault_open_error(target_vault_ref.as_str()).into());
        }
        if let Some(handle) = self.vaults.get_mut(vault_ref.as_str()) {
            handle.touch(params.ts);
            handle.vault.flush()?;
        }
        let source_dir = resolve_existing_vault_dir(&self.config.data_dir, &vault_ref)?;
        let target_dir = resolve_vault_target_dir(&self.config.data_dir, &target_vault_ref)?;
        let source_verify = verify_restore_value(&source_dir)?;
        let copy = copy_dir_new(&source_dir, &target_dir)?;
        let clone_verify = verify_restore_value(&target_dir)?;
        Ok(json!({
            "status": "cloned",
            "vault_ref": vault_ref.as_str(),
            "target_vault_ref": target_vault_ref.as_str(),
            "source_dir": source_dir,
            "target_dir": target_dir,
            "cloned_at": params.ts,
            "copy": copy.to_value(),
            "source_verify_restore": source_verify,
            "verify_restore": clone_verify
        }))
    }

    fn vault_verify(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<VaultParams>(params, "vault.verify")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let dir = match self.vaults.get_mut(vault_ref.as_str()) {
            Some(handle) => {
                handle.touch(params.ts);
                handle.vault.flush()?;
                handle.dir.clone()
            }
            None => resolve_existing_vault_dir(&self.config.data_dir, &vault_ref)?,
        };
        let verify = verify_restore_value(&dir)?;
        Ok(json!({
            "status": "verified",
            "vault_ref": vault_ref.as_str(),
            "vault_dir": dir,
            "verified_at": params.ts,
            "verify_restore": verify
        }))
    }

    fn vault_stat(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<VaultParams>(params, "vault.stat")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let Some(handle) = self.vaults.get_mut(vault_ref.as_str()) else {
            return Err(vault_not_open(vault_ref.as_str()).into());
        };
        handle.touch(params.ts);
        Ok(json!({
            "vault_ref": handle.vault_ref.as_str(),
            "vault_id": handle.vault_id.to_string(),
            "vault_dir": handle.dir,
            "context_vault_id": handle.context.vault_id().to_string(),
            "opened_at": handle.opened_at,
            "last_ts": handle.last_ts,
            "requests": handle.requests,
            "latest_seq": handle.vault.latest_seq(),
            "recovered_seq": handle.vault.recovery_report().last_recovered_seq
        }))
    }

    fn panic_probe(&self) -> EngineResult<Value> {
        if std::env::var_os(PANIC_PROBE_ENV).as_deref() != Some(std::ffi::OsStr::new("1")) {
            return Err(CalyxError {
                code: CALYX_LEAPABLE_PANIC_PROBE_DISABLED,
                message: "engine.panic_probe requires CALYX_LEAPABLE_ENABLE_PANIC_PROBE=1".into(),
                remediation: "enable the panic probe only in FSV/test sessions",
            }
            .into());
        }
        panic!("calyx-leapable panic isolation probe");
    }

    fn open_handle(&self, vault_ref: VaultRef, dir: PathBuf, ts: Ts) -> EngineResult<VaultHandle> {
        let vault_id = vault_id_for(vault_ref.as_str());
        let salt = salt_for(vault_ref.as_str());
        let clock = EngineClock::new(ts);
        let txn = TxnHandle::new(vault_id);
        let context = VaultContext::new(
            vault_id,
            &self.config.master_key,
            QuotaConfig::default(),
            ZFS_DATASET_UNAVAILABLE,
        )?;
        let vault = AsterVault::new_durable_with_clock(
            &dir,
            vault_id,
            salt,
            VaultOptions::default(),
            clock.clone(),
        )?;
        Ok(VaultHandle {
            vault_ref,
            vault_id,
            dir,
            opened_at: ts,
            last_ts: ts,
            requests: 1,
            clock,
            txn,
            context,
            vault,
        })
    }
}

fn vault_handle_value(status: &str, handle: &VaultHandle) -> Value {
    json!({
        "status": status,
        "vault_ref": handle.vault_ref.as_str(),
        "vault_id": handle.vault_id.to_string(),
        "vault_dir": handle.dir,
        "context_vault_id": handle.context.vault_id().to_string(),
        "opened_at": handle.opened_at,
        "last_ts": handle.last_ts,
        "requests": handle.requests,
        "latest_seq": handle.vault.latest_seq(),
        "recovered_seq": handle.vault.recovery_report().last_recovered_seq
    })
}

mod clock;
mod cx;
mod error;
mod identity;
mod storage;

#[cfg(test)]
mod tests;
