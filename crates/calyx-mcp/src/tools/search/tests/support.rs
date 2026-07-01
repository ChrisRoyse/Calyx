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
    let router = CfRouter::open(vault, 0).expect("open CF router");
    let key = ledger_key(seq);
    let mut entries = router
        .iter_cf(ColumnFamily::Ledger)
        .expect("read ledger rows");
    let row = entries
        .iter_mut()
        .find(|entry| entry.key == key)
        .expect("ledger row exists");
    let last = row
        .value
        .len()
        .checked_sub(1)
        .expect("non-empty ledger row");
    row.value[last] ^= 0x55;

    let cf_dir = vault.join("cf").join(ColumnFamily::Ledger.name());
    for entry in fs::read_dir(&cf_dir).expect("read ledger CF directory") {
        let path = entry.expect("read ledger CF entry").path();
        if path.extension().and_then(|value| value.to_str()) == Some("sst") {
            fs::remove_file(path).expect("remove original ledger SST");
        }
    }
    calyx_aster::sst::write_sst(
        cf_dir.join("00000000000000000001.sst"),
        entries
            .iter()
            .map(|entry| (entry.key.as_slice(), entry.value.as_slice())),
    )
    .expect("write tampered ledger SST");
    let wal_dir = vault.join("wal");
    if wal_dir.exists() {
        fs::remove_dir_all(wal_dir).expect("remove stale WAL after ledger SST rewrite");
    }
}

pub(super) fn remove_ledger_row(vault: &Path, seq: u64) {
    let router = CfRouter::open(vault, 0).expect("open CF router");
    let key = ledger_key(seq);
    let mut entries = router
        .iter_cf(ColumnFamily::Ledger)
        .expect("read ledger rows");
    entries.retain(|entry| entry.key != key);
    rewrite_ledger_sst(vault, &entries);
}

pub(super) fn remove_ledger_head_anchor(vault: &Path) {
    let path = vault.join("ledger_head").join("current.json");
    if path.exists() {
        fs::remove_file(path).expect("remove ledger head anchor");
    }
}

pub(super) fn ledger_head_anchor_exists(vault: &Path) -> bool {
    vault.join("ledger_head").join("current.json").is_file()
}

pub(super) fn base_exists(vault: &Path, cx_id: &str) -> bool {
    let cx_id = cx_id.parse().expect("parse cx id");
    let key = base_key(cx_id);
    CfRouter::open(vault, 0)
        .and_then(|router| router.iter_cf(ColumnFamily::Base))
        .map(|entries| entries.iter().any(|entry| entry.key == key))
        .unwrap_or(false)
}

pub(super) fn ledger_rows(vault: &Path) -> Vec<Value> {
    CfRouter::open(vault, 0)
        .and_then(|router| router.iter_cf(ColumnFamily::Ledger))
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            json!({
                "seq": u64::from_be_bytes(row.key.as_slice().try_into().expect("ledger key")),
                "bytes_len": row.value.len(),
                "bytes_blake3": blake3::hash(&row.value).to_hex().to_string(),
            })
        })
        .collect()
}

fn rewrite_ledger_sst(vault: &Path, entries: &[calyx_aster::sst::SstEntry]) {
    let cf_dir = vault.join("cf").join(ColumnFamily::Ledger.name());
    for entry in fs::read_dir(&cf_dir).expect("read ledger CF directory") {
        let path = entry.expect("read ledger CF entry").path();
        if path.extension().and_then(|value| value.to_str()) == Some("sst") {
            fs::remove_file(path).expect("remove original ledger SST");
        }
    }
    if !entries.is_empty() {
        calyx_aster::sst::write_sst(
            cf_dir.join("00000000000000000001.sst"),
            entries
                .iter()
                .map(|entry| (entry.key.as_slice(), entry.value.as_slice())),
        )
        .expect("write rewritten ledger SST");
    }
    let wal_dir = vault.join("wal");
    if wal_dir.exists() {
        fs::remove_dir_all(wal_dir).expect("remove stale WAL after ledger SST rewrite");
    }
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
                && matches!(&slot.shape, SlotShape::Dense(_))
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
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    fs::create_dir_all(&root).expect("create fsv root");
    fs::write(
        root.join(name),
        serde_json::to_vec_pretty(value).expect("fsv json"),
    )
    .expect("write fsv json");
}
