//! Direct JSON-RPC method dispatch for the Leapable engine sidecar.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

use calyx_aster::vault::{AsterVault, QuotaConfig, VaultContext, VaultOptions};
use calyx_core::{CalyxError, FixedClock, Ts, VaultId, content_address};
use calyx_mcp::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use serde::Deserialize;
use serde_json::{Value, json};
use ulid::Ulid;

use crate::config::EngineConfig;
use crate::paths::{VaultRef, resolve_vault_dir};

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
    context: VaultContext,
    vault: AsterVault<FixedClock>,
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
            "vault.open" => self.vault_open(request.params),
            "vault.stat" => self.vault_stat(request.params),
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

    fn vault_open(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<VaultParams>(params, "vault.open")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let dir = resolve_vault_dir(&self.config.data_dir, &vault_ref)?;
        let vault_id = vault_id_for(vault_ref.as_str());
        let salt = salt_for(vault_ref.as_str());
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
            FixedClock::new(params.ts),
        )?;
        vault.flush()?;
        let reopened = self.vaults.contains_key(vault_ref.as_str());
        let handle = VaultHandle {
            vault_ref: vault_ref.clone(),
            vault_id,
            dir: dir.clone(),
            opened_at: params.ts,
            last_ts: params.ts,
            requests: 1,
            context,
            vault,
        };
        self.vaults.insert(vault_ref.as_str().to_string(), handle);
        Ok(json!({
            "status": if reopened { "reopened" } else { "opened" },
            "vault_ref": vault_ref.as_str(),
            "vault_id": vault_id.to_string(),
            "vault_dir": dir,
            "opened_at": params.ts,
            "latest_seq": self.vaults[vault_ref.as_str()].vault.latest_seq()
        }))
    }

    fn vault_stat(&mut self, params: Option<Value>) -> EngineResult<Value> {
        let params = parse_params::<VaultParams>(params, "vault.stat")?;
        let vault_ref = VaultRef::parse(&params.vault_ref)?;
        let Some(handle) = self.vaults.get_mut(vault_ref.as_str()) else {
            return Err(vault_not_open(vault_ref.as_str()).into());
        };
        handle.requests += 1;
        handle.last_ts = params.ts;
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
}

type EngineResult<T> = std::result::Result<T, EngineError>;

enum EngineError {
    InvalidParams(String),
    Calyx(CalyxError),
}

impl From<CalyxError> for EngineError {
    fn from(error: CalyxError) -> Self {
        Self::Calyx(error)
    }
}

fn parse_params<T: for<'de> Deserialize<'de>>(
    params: Option<Value>,
    method: &str,
) -> EngineResult<T> {
    let params = params.unwrap_or(Value::Null);
    serde_json::from_value(params).map_err(|error| {
        EngineError::InvalidParams(format!("{method} params do not match schema: {error}"))
    })
}

fn vault_not_open(vault_ref: &str) -> CalyxError {
    CalyxError {
        code: CALYX_LEAPABLE_VAULT_NOT_OPEN,
        message: format!("vault_ref {vault_ref:?} is not open in this engine process"),
        remediation: "call vault.open for this vault_ref before issuing vault-scoped requests",
    }
}

fn vault_id_for(vault_ref: &str) -> VaultId {
    VaultId::from_ulid(Ulid::from_bytes(content_address([vault_ref.as_bytes()])))
}

fn salt_for(vault_ref: &str) -> Vec<u8> {
    content_address([
        b"calyx-leapable-vault-salt".as_slice(),
        vault_ref.as_bytes(),
    ])
    .to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_mcp::decode_jsonrpc_request;
    use std::fs;

    fn temp_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("calyx-leapable-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    fn req(line: &str) -> calyx_mcp::JsonRpcRequest {
        decode_jsonrpc_request(line.as_bytes()).unwrap()
    }

    fn config(root: PathBuf) -> EngineConfig {
        EngineConfig {
            data_dir: root,
            master_key: vec![0xA5; 32],
        }
    }

    #[test]
    fn two_vaults_are_multiplexed_by_vault_ref() {
        let root = temp_root("multiplex");
        let mut engine = Engine::new(config(root.clone()));
        let open_a = engine.dispatch(req(
            r#"{"jsonrpc":"2.0","id":1,"method":"vault.open","params":{"vault_ref":"alpha","ts":1785500000}}"#,
        ));
        let open_b = engine.dispatch(req(
            r#"{"jsonrpc":"2.0","id":2,"method":"vault.open","params":{"vault_ref":"beta","ts":1785500010}}"#,
        ));
        assert!(open_a.error.is_none());
        assert!(open_b.error.is_none());

        let stat_a = engine.dispatch(req(
            r#"{"jsonrpc":"2.0","id":3,"method":"vault.stat","params":{"vault_ref":"alpha","ts":1785500020}}"#,
        ));
        let stat_b = engine.dispatch(req(
            r#"{"jsonrpc":"2.0","id":4,"method":"vault.stat","params":{"vault_ref":"beta","ts":1785500030}}"#,
        ));
        assert_eq!(stat_a.result.unwrap()["vault_ref"], "alpha");
        assert_eq!(stat_b.result.unwrap()["vault_ref"], "beta");
        assert_ne!(vault_id_for("alpha"), vault_id_for("beta"));
        assert!(root.join("alpha").join("cf").join("base").exists());
        assert!(root.join("alpha").join("wal").exists());
        assert!(root.join("beta").join("cf").join("base").exists());
        assert!(root.join("beta").join("wal").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn vault_stat_requires_prior_open() {
        let mut engine = Engine::new(config(temp_root("not-open")));
        let response = engine.dispatch(req(
            r#"{"jsonrpc":"2.0","id":1,"method":"vault.stat","params":{"vault_ref":"ghost","ts":1}}"#,
        ));
        let error = response.error.unwrap();
        assert_eq!(error.code, -32000);
        assert_eq!(
            error.data.unwrap()["calyx_code"],
            CALYX_LEAPABLE_VAULT_NOT_OPEN
        );
    }

    #[test]
    fn invalid_vault_ref_fails_closed() {
        let mut engine = Engine::new(config(temp_root("bad-ref")));
        let response = engine.dispatch(req(
            r#"{"jsonrpc":"2.0","id":1,"method":"vault.open","params":{"vault_ref":"../escape","ts":1}}"#,
        ));
        let error = response.error.unwrap();
        assert_eq!(error.code, -32000);
        assert_eq!(
            error.data.unwrap()["calyx_code"],
            crate::paths::CALYX_LEAPABLE_PATH_INVALID
        );
    }

    #[test]
    fn missing_timestamp_is_invalid_params() {
        let mut engine = Engine::new(config(temp_root("missing-ts")));
        let response = engine.dispatch(req(
            r#"{"jsonrpc":"2.0","id":1,"method":"vault.open","params":{"vault_ref":"alpha"}}"#,
        ));
        assert_eq!(response.error.unwrap().code, -32602);
    }
}
