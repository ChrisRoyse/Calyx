use super::*;

pub(super) struct TestEnv {
    home: PathBuf,
    old_home: Option<OsString>,
    _guard: MutexGuard<'static, ()>,
}

impl TestEnv {
    pub(super) fn new(name: &str) -> Self {
        let guard = ENV_LOCK.lock().unwrap();
        let home =
            std::env::temp_dir().join(format!("calyx-mcp-search-{name}-{}", std::process::id()));
        if home.exists() {
            fs::remove_dir_all(&home).expect("remove stale test home");
        }
        fs::create_dir_all(&home).expect("create test home");
        let old_home = std::env::var_os("CALYX_HOME");
        unsafe {
            std::env::set_var("CALYX_HOME", &home);
        }
        Self {
            home,
            old_home,
            _guard: guard,
        }
    }

    pub(super) fn vault_path(&self, vault_id: &str) -> PathBuf {
        self.home.join("vaults").join(vault_id)
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        match &self.old_home {
            Some(value) => unsafe {
                std::env::set_var("CALYX_HOME", value);
            },
            None => unsafe {
                std::env::remove_var("CALYX_HOME");
            },
        }
        if self.home.starts_with(std::env::temp_dir()) {
            let _ = fs::remove_dir_all(&self.home);
        }
    }
}

pub(super) fn server() -> McpServer {
    let mut server = McpServer::new();
    crate::tools::register_all(&mut server).unwrap();
    server
}

pub(super) fn authn() -> AuthN {
    AuthN::InProcess {
        host_app_id: "calyx-mcp-test".into(),
    }
}

pub(super) fn call_ok(server: &McpServer, id: u64, name: &str, arguments: Value) -> Value {
    let request = decode_jsonrpc_request(
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let authn = authn();
    let response = server.dispatch_with_authn(request, Some(&authn));
    assert!(response.error.is_none(), "{:?}", response.error);
    let result = response.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).unwrap()
}

pub(super) fn call_err(server: &McpServer, id: u64, name: &str, arguments: Value) -> JsonRpcError {
    let request = decode_jsonrpc_request(
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let authn = authn();
    server
        .dispatch_with_authn(request, Some(&authn))
        .error
        .unwrap()
}

pub(super) fn vault_with_algorithmic_data(server: &McpServer, name: &str) -> Vec<Value> {
    call_ok(server, 1, "calyx.create_vault", json!({"name": name}));
    call_ok(
        server,
        2,
        "calyx.add_lens",
        json!({"vault": name, "name": "byte_axis", "runtime": "algorithmic"}),
    );
    ["alpha", "beta"]
        .into_iter()
        .enumerate()
        .map(|(idx, text)| {
            call_ok(
                server,
                3 + idx as u64,
                "calyx.ingest",
                json!({"vault": name, "input": text}),
            )
        })
        .collect()
}

pub(super) fn tamper_ledger_row(vault: &Path, seq: u64) {
    let mut router = CfRouter::open(vault, 0).expect("open CF router");
    let key = ledger_key(seq);
    let mut bytes = router
        .get(ColumnFamily::Ledger, &key)
        .expect("read ledger row")
        .expect("ledger row exists");
    let last = bytes.len().checked_sub(1).expect("non-empty ledger row");
    bytes[last] ^= 0x55;
    router
        .put(ColumnFamily::Ledger, &key, &bytes)
        .expect("write tampered ledger row");
    router
        .flush_cf(ColumnFamily::Ledger)
        .expect("flush tampered ledger row");
}

pub(super) fn write_calibrated_default_guard(vault: &Path, vault_id: &str, name: &str, tau: f32) {
    let state = load_vault_panel_state(vault).expect("load panel state");
    let slot = state
        .panel
        .slots
        .iter()
        .find(|slot| {
            slot.state == SlotState::Active
                && slot.modality == Modality::Text
                && state.registry.contains(slot.lens_id)
        })
        .expect("active registered text slot")
        .slot_id;
    let mut per_slot = BTreeMap::new();
    per_slot.insert(
        slot,
        SlotCalibrationMeta {
            corpus_hash: [9; 32],
            estimator: "synthetic-fsv".to_string(),
            far: 0.0,
            frr: 0.0,
            confidence: 0.99,
            ts: 1_785_400_000,
            slot_kind: Some(SlotKind::Content),
        },
    );
    let mut tau_by_slot = BTreeMap::new();
    tau_by_slot.insert(slot, tau);
    let profile = GuardProfile {
        guard_id: GuardId::from_str("018f48a4-9a79-74d2-8a5c-9ad7f6b8c101").expect("guard id"),
        panel_version: u64::from(state.panel.version),
        domain: "default".to_string(),
        tau: tau_by_slot,
        required_slots: vec![slot],
        policy: GuardPolicy::AllRequired,
        calibration: Some(CalibrationMeta {
            corpus_hash: [9; 32],
            estimator: "synthetic-fsv".to_string(),
            far: 0.0,
            frr: 0.0,
            confidence: 0.99,
            ts: 1_785_400_000,
            per_slot,
        }),
        novelty_action: NoveltyAction::RejectClosed,
    };
    let vault_id = vault_id.parse().expect("parse vault id");
    let vault = AsterVault::open(
        vault,
        vault_id,
        crate::tools::vault::store::vault_salt(vault_id, name),
        VaultOptions::default(),
    )
    .expect("open vault");
    let bytes = serde_json::to_vec(&profile).expect("serialize profile");
    vault
        .write_cf(ColumnFamily::Guard, b"profile\0default".to_vec(), bytes)
        .expect("write guard profile");
    vault.flush().expect("flush guard profile");
}

pub(super) fn maybe_write_fsv_json(name: &str, value: &Value) {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    fs::create_dir_all(&root).expect("create fsv root");
    fs::write(
        root.join(name),
        serde_json::to_vec_pretty(value).expect("fsv json"),
    )
    .expect("write fsv json");
}
