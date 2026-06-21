use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::sync::MutexGuard;

use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Asymmetry, AuthN, CxFlags, CxId, InputRef, LedgerRef, LensId,
    Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState, SlotVector, VaultId,
};
use serde_json::{Value, json};

use crate::jsonrpc::decode_jsonrpc_request;
use crate::protocol::JsonRpcError;
use crate::server::McpServer;
use crate::tools::test_support::ENV_LOCK;

use super::{metrics, model};

mod propose_driver;

struct TestEnv {
    home: PathBuf,
    old_home: Option<OsString>,
    _guard: MutexGuard<'static, ()>,
}

impl TestEnv {
    fn new(name: &str) -> Self {
        let guard = ENV_LOCK.lock().unwrap();
        let home = std::env::temp_dir().join(format!(
            "calyx-mcp-intelligence-{name}-{}",
            std::process::id()
        ));
        if home.exists() {
            fs::remove_dir_all(&home).expect("remove stale home");
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

    fn path(&self, relative: &str) -> PathBuf {
        self.home.join(relative)
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

fn server() -> McpServer {
    let mut server = McpServer::new();
    crate::tools::register_all(&mut server).unwrap();
    server
}

fn authn() -> AuthN {
    AuthN::InProcess {
        host_app_id: "calyx-mcp-test".into(),
    }
}

fn call_ok(server: &McpServer, id: u64, name: &str, arguments: Value) -> Value {
    let request = request(id, name, arguments);
    let authn = authn();
    let response = server.dispatch_with_authn(request, Some(&authn));
    assert!(response.error.is_none(), "{:?}", response.error);
    let result = response.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).unwrap()
}

fn call_err(server: &McpServer, id: u64, name: &str, arguments: Value) -> JsonRpcError {
    let authn = authn();
    server
        .dispatch_with_authn(request(id, name, arguments), Some(&authn))
        .error
        .unwrap()
}

fn request(id: u64, name: &str, arguments: Value) -> crate::jsonrpc::JsonRpcRequest {
    decode_jsonrpc_request(
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap()
}

#[test]
fn abundance_on_100_constellations_and_two_slots_matches_card() {
    let docs = docs_with_signal(100, true, false);
    let report = metrics::abundance(&docs, &[SlotId::new(0), SlotId::new(1)]);

    assert_eq!(report.n, 100);
    assert_eq!(report.pairs, 4950);
    assert_eq!(report.panel_size, 2);
}

#[test]
fn bits_insufficient_samples_has_exact_remediation() {
    let docs = docs_with_signal(30, true, false);
    let err = metrics::bits(
        &panel_two_active(),
        &docs,
        &AnchorKind::TestPass,
        "test_pass",
        false,
        &model::assay_key("test_pass"),
    )
    .unwrap_err();

    match err {
        crate::server::ToolError::Calyx(error) => {
            assert_eq!(error.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
            assert_eq!(error.remediation, "anchor ≥50 outcomes first");
        }
        other => panic!("expected Calyx error, got {other:?}"),
    }
}

#[test]
fn bits_reports_low_signal_slot_and_fails_when_all_low() {
    let docs = docs_with_signal(100, true, false);
    let report = metrics::bits(
        &panel_two_active(),
        &docs,
        &AnchorKind::TestPass,
        "test_pass",
        true,
        &model::assay_key("test_pass"),
    )
    .unwrap();
    assert!(report.per_slot.iter().any(|slot| slot.bits >= 0.05));
    assert!(report.per_slot.iter().any(|slot| slot.low_signal));

    let low_docs = docs_with_signal(100, false, false);
    let err = metrics::bits(
        &panel_one_active(),
        &low_docs,
        &AnchorKind::TestPass,
        "test_pass",
        false,
        &model::assay_key("test_pass"),
    )
    .unwrap_err();
    assert_calyx_code(err, "CALYX_ASSAY_LOW_SIGNAL");
}

#[test]
fn bits_redundant_slots_fail_closed() {
    let docs = docs_with_signal(100, true, true);
    let err = metrics::bits(
        &panel_two_active(),
        &docs,
        &AnchorKind::TestPass,
        "test_pass",
        false,
        &model::assay_key("test_pass"),
    )
    .unwrap_err();

    assert_calyx_code(err, "CALYX_ASSAY_REDUNDANT");
}

#[test]
fn kernel_without_anchors_is_ungrounded() {
    let docs = docs_without_anchors(5);
    let err = metrics::kernel(&docs, Some(&AnchorKind::TestPass)).unwrap_err();

    assert_calyx_code(err, "CALYX_KERNEL_UNGROUNDED");
}

#[test]
fn mcp_abundance_empty_vault_is_zeroes() {
    let _env = TestEnv::new("abundance-empty");
    let server = server();
    call_ok(&server, 1, "calyx.create_vault", json!({"name": "v"}));

    let report = call_ok(&server, 2, "calyx.abundance", json!({"vault": "v"}));

    assert_eq!(report["n"], 0);
    assert_eq!(report["pairs"], 0);
}

#[test]
fn mcp_guard_check_before_calibration_is_provisional() {
    let _env = TestEnv::new("guard-provisional");
    let server = server();
    let cx_id = one_dense_doc(&server, "v");

    let error = call_err(
        &server,
        10,
        "calyx.guard.check",
        json!({"vault": "v", "cx_id": cx_id}),
    );

    assert_eq!(error.code, -32000);
    assert_eq!(error.data.unwrap()["calyx_code"], "CALYX_GUARD_PROVISIONAL");
}

#[test]
fn mcp_guard_calibrate_then_check_returns_verdict() {
    let env = TestEnv::new("guard-calibrated");
    let server = server();
    let cx_id = one_dense_doc(&server, "v");
    let set = env.path("guard.jsonl");
    fs::write(&set, calibration_jsonl(8)).unwrap();

    let profile = call_ok(
        &server,
        20,
        "calyx.guard.calibrate",
        json!({"vault": "v", "domain": "unit", "set": set, "target_far": 0.01}),
    );
    let verdict = call_ok(
        &server,
        21,
        "calyx.guard.check",
        json!({"vault": "v", "cx_id": cx_id}),
    );

    assert!(profile["tau"].as_f64().unwrap() > 0.0);
    assert!(profile["n_corpus"].as_u64().unwrap() >= 10);
    assert_eq!(verdict["verdict"], "pass");
    assert!(verdict["distance"].as_f64().unwrap() >= 0.0);
}

#[test]
fn mcp_kernel_and_propose_lens_edges() {
    let _env = TestEnv::new("kernel-propose");
    let server = server();
    one_dense_doc(&server, "v");

    let kernel = call_err(
        &server,
        30,
        "calyx.kernel",
        json!({"vault": "v", "anchor": "test_pass"}),
    );
    let proposal = call_err(
        &server,
        31,
        "calyx.propose_lens",
        json!({"vault": "v", "anchor": "test_pass"}),
    );

    assert_eq!(
        kernel.data.unwrap()["calyx_code"],
        "CALYX_KERNEL_UNGROUNDED"
    );
    assert_eq!(
        proposal.data.unwrap()["calyx_code"],
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
    );
}

fn one_dense_doc(server: &McpServer, vault: &str) -> String {
    call_ok(server, 1, "calyx.create_vault", json!({"name": vault}));
    call_ok(
        server,
        2,
        "calyx.add_lens",
        json!({"vault": vault, "name": "byte_axis", "runtime": "algorithmic", "shape": "Dense(16)"}),
    );
    call_ok(
        server,
        3,
        "calyx.ingest",
        json!({"vault": vault, "input": "alpha"}),
    )["cx_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn calibration_jsonl(slot: u16) -> String {
    let mut out = String::new();
    for _ in 0..50 {
        out.push_str(&format!(r#"{{"slot":{slot},"score":0.99,"class":"good"}}"#));
        out.push('\n');
        out.push_str(&format!(
            r#"{{"slot":{slot},"score":0.10,"class":"injection"}}"#
        ));
        out.push('\n');
    }
    out
}

fn assert_calyx_code(error: crate::server::ToolError, code: &str) {
    match error {
        crate::server::ToolError::Calyx(error) => assert_eq!(error.code, code),
        other => panic!("expected Calyx error {code}, got {other:?}"),
    }
}

fn docs_with_signal(
    n: usize,
    slot0_separates: bool,
    slot1_separates: bool,
) -> BTreeMap<CxId, calyx_core::Constellation> {
    (0..n)
        .map(|idx| {
            let positive = idx < n / 2;
            let cx = constellation(
                idx as u8,
                Some(AnchorValue::Bool(positive)),
                vector_for(positive, slot0_separates),
                vector_for(positive, slot1_separates),
            );
            (cx.cx_id, cx)
        })
        .collect()
}

fn docs_without_anchors(n: usize) -> BTreeMap<CxId, calyx_core::Constellation> {
    (0..n)
        .map(|idx| {
            let cx = constellation(idx as u8, None, vec![1.0, 0.0], vec![1.0, 0.0]);
            (cx.cx_id, cx)
        })
        .collect()
}

fn vector_for(positive: bool, separates: bool) -> Vec<f32> {
    if !separates || positive {
        vec![1.0, 0.0]
    } else {
        vec![0.0, 1.0]
    }
}

fn panel_one_active() -> Panel {
    Panel {
        slots: vec![slot(0)],
        ..panel_two_active()
    }
}

fn panel_two_active() -> Panel {
    Panel {
        version: 1,
        slots: vec![slot(0), slot(1)],
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn slot(id: u16) -> Slot {
    let slot_id = SlotId::new(id);
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, format!("slot-{id}")),
        lens_id: LensId::from_bytes([id as u8; 16]),
        shape: SlotShape::Dense(2),
        modality: Modality::Text,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: Some("unit".to_string()),
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: BTreeMap::new(),
        state: SlotState::Active,
        added_at_panel_version: 1,
    }
}

fn constellation(
    seed: u8,
    anchor_value: Option<AnchorValue>,
    slot0: Vec<f32>,
    slot1: Vec<f32>,
) -> calyx_core::Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: slot0,
        },
    );
    slots.insert(
        SlotId::new(1),
        SlotVector::Dense {
            dim: 2,
            data: slot1,
        },
    );
    calyx_core::Constellation {
        cx_id: CxId::from_bytes([seed; 16]),
        vault_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse::<VaultId>().unwrap(),
        panel_version: 1,
        created_at: u64::from(seed),
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: anchor_value
            .map(|value| {
                vec![Anchor {
                    kind: AnchorKind::TestPass,
                    value,
                    source: "unit".to_string(),
                    observed_at: 1,
                    confidence: 1.0,
                }]
            })
            .unwrap_or_default(),
        provenance: LedgerRef {
            seq: u64::from(seed),
            hash: [seed; 32],
        },
        flags: CxFlags {
            ungrounded: false,
            degraded: false,
            novel_region: false,
            redacted_input: false,
        },
    }
}
