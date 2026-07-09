//! Direct JSON-RPC method dispatch for the Leapable engine sidecar.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;

use calyx_aster::collection::Collection;
use calyx_aster::txn::TxnHandle;
use calyx_aster::vault::{AsterVault, QuotaConfig, VaultContext, VaultOptions};
use calyx_core::{CalyxError, Ts, VaultId};
use calyx_mcp::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use serde::Deserialize;
use serde_json::{Value, json};

use self::clock::EngineClock;
use self::error::{EngineError, EngineResult, parse_params, vault_not_open, vault_open_error};
use self::identity::{salt_for_dir, vault_id_for};
use crate::config::{EngineConfig, FlushPolicy};
use crate::lifecycle::{
    CALYX_LEAPABLE_VAULT_ALREADY_EXISTS, CALYX_LEAPABLE_VAULT_OPEN, lifecycle_error, remove_dir,
    verify_restore_value,
};
use crate::paths::{VaultRef, list_vault_refs, resolve_existing_vault_dir, resolve_new_vault_dir};

/// Compile-time capability map: end-user binary is CPU-only.
pub const LEAPABLE_CAPABILITIES: &[(&str, bool)] = &[
    ("cpu-only", true),
    ("hnsw-ram", true),
    ("cuda", false),
    ("diskann", false),
    ("spann", false),
];

pub fn mutating_method_requires_id(method: &str) -> bool {
    matches!(
        method,
        "vault.create"
            | "vault.close"
            | "vault.delete"
            | "vault.snapshot"
            | "vault.restore"
            | "vault.clone"
            | "cx.put"
            | "cx.put_batch"
            | "cx.anchor"
            | "cx.delete"
            | "rel.insert"
            | "rel.update_row"
            | "rel.delete"
            | "kv.set"
            | "kv.delete"
            | "ts.write"
            | "blob.put"
            | "txn.commit"
    )
}

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
    last_flush_ts: Ts,
    requests: u64,
    clock: EngineClock,
    txn: TxnHandle,
    context: VaultContext,
    vault: AsterVault<EngineClock>,
    collections: RefCell<HashMap<String, Arc<Collection>>>,
}

impl VaultHandle {
    fn touch(&mut self, ts: Ts) {
        self.requests += 1;
        self.last_ts = self.clock.advance_to(ts);
    }

    fn flush_now(&mut self) -> EngineResult<()> {
        self.vault.flush()?;
        self.last_flush_ts = self.last_ts;
        Ok(())
    }

    fn flush_after_write(&mut self, policy: &FlushPolicy) -> EngineResult<()> {
        match policy {
            FlushPolicy::Always => self.flush_now(),
            FlushPolicy::Batch { max_delay_ms } => {
                if self.last_ts.saturating_sub(self.last_flush_ts) >= *max_delay_ms {
                    self.flush_now()?;
                }
                Ok(())
            }
        }
    }

    fn cached_collection(&self, name: &str) -> Option<Arc<Collection>> {
        self.collections.borrow().get(name).cloned()
    }

    fn cache_collection(&self, collection: Collection) -> Arc<Collection> {
        let collection = Arc::new(collection);
        self.collections
            .borrow_mut()
            .insert(collection.name.clone(), Arc::clone(&collection));
        collection
    }
}

#[derive(Deserialize)]
struct VaultParams {
    vault_ref: String,
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
        let mut handle = self.open_handle(vault_ref.clone(), dir, params.ts)?;
        cx::ensure_cx_tombstone_index(&handle)?;
        handle.flush_now()?;
        let value = vault_handle_value("created", &handle);
        self.vaults.insert(vault_ref.as_str().to_string(), handle);
        Ok(value)
    }

    fn vault_open(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<VaultParams>(params, "vault.open")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        if let Some(handle) = self.vaults.get_mut(vault_ref.as_str()) {
            handle.touch(params.ts);
            cx::ensure_cx_tombstone_index(handle)?;
            return Ok(vault_handle_value("already_open", handle));
        }
        let dir = resolve_existing_vault_dir(&self.config.data_dir, &vault_ref)?;
        let handle = self.open_handle(vault_ref.clone(), dir, params.ts)?;
        cx::ensure_cx_tombstone_index(&handle)?;
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
        handle.flush_now()?;
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

    fn vault_verify(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<VaultParams>(params, "vault.verify")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let dir = match self.vaults.get_mut(vault_ref.as_str()) {
            Some(handle) => {
                handle.touch(params.ts);
                handle.flush_now()?;
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
        let salt = salt_for_dir(&dir, vault_ref.as_str())?;
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
            last_flush_ts: ts,
            requests: 1,
            clock,
            txn,
            context,
            vault,
            collections: RefCell::default(),
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
mod hex;
mod identity;
mod lifecycle_ops;
mod storage;
mod verify;

#[cfg(test)]
mod tests;
