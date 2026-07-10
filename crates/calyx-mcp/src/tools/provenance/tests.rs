use std::collections::BTreeMap;
use std::fs;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::ledger_view::AsterLedgerCfStore;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Asymmetry, CxFlags, CxId, FixedClock, InputRef, LedgerRef,
    LensId, Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState, SlotVector,
    VaultId, VaultStore,
};
use calyx_ledger::{
    ActorId, EntryKind, FusionMode, FusionWeights, LedgerAppender, LedgerCfStore, LedgerEntry,
    LedgerRow, MemoryLedgerStore, REPRODUCE_PAYLOAD_TAG, SubjectId, get_provenance,
};
use serde_json::{Value, json};
use ulid::Ulid;

use super::quarantine::NoQuarantine;
use super::{core, status};
use crate::tools::test_support::ENV_LOCK;
use crate::tools::vault::store::{ResolvedVault, vault_salt};

#[test]
fn lineage_reports_original_ingest_and_mcp_anchor_sequences() {
    let (root, resolved, vault) = test_vault("lineage");
    let cx = sample_constellation(&vault, resolved.vault_id);
    let cx_id = cx.cx_id;
    vault.put(cx).unwrap();
    vault.flush().unwrap();
    let ingested = vault.get(cx_id, vault.snapshot()).unwrap();
    let anchor_ref = vault
        .anchor_with_ledger_entry(
            cx_id,
            test_anchor(),
            EntryKind::Ingest,
            SubjectId::Cx(cx_id),
            serde_json::to_vec(&json!({
                "mode": "mcp-anchor",
                "anchor_kind": "test_pass",
            }))
            .unwrap(),
            ActorId::Service("calyx-mcp-test".to_string()),
        )
        .unwrap();
    vault.flush().unwrap();

    let out = serde_json::to_value(core::lineage_for_resolved(&resolved, cx_id).unwrap()).unwrap();

    assert_eq!(out["cx_id"], cx_id.to_string());
    assert_eq!(out["ingest_seq"], ingested.provenance.seq);
    assert_eq!(
        out["ledger_chain_hash"],
        core::hex(&ingested.provenance.hash)
    );
    assert_eq!(out["lens_measures"][0]["slot"], 0);
    assert_eq!(out["anchors"][0]["kind"], "test_pass");
    assert_eq!(out["anchors"][0]["ledger_seq"], anchor_ref.seq);
    fs::remove_dir_all(root).ok();
}

#[test]
fn lineage_refuses_generic_anchor_ledger_fallback() {
    let (root, resolved, vault) = test_vault("anchor-fail-closed");
    let cx = sample_constellation(&vault, resolved.vault_id);
    let cx_id = cx.cx_id;
    vault.put(cx).unwrap();
    vault.flush().unwrap();
    vault.anchor(cx_id, test_anchor()).unwrap();
    let generic_ref = vault
        .append_ledger_entry(
            EntryKind::Ingest,
            SubjectId::Cx(cx_id),
            serde_json::to_vec(&json!({
                "mode": "mcp-anchor",
            }))
            .unwrap(),
            ActorId::Service("calyx-mcp-test".to_string()),
        )
        .unwrap();
    vault.flush().unwrap();

    let stored = vault.get(cx_id, vault.snapshot()).unwrap();
    let ledger_store = AsterLedgerCfStore::open(&resolved.path).unwrap();
    let entries = get_provenance(&ledger_store, &NoQuarantine, cx_id).unwrap();
    let err = core::lineage_for_resolved(&resolved, cx_id).unwrap_err();

    assert_eq!(stored.anchors.len(), 1);
    assert_eq!(generic_ref.seq, entries.last().unwrap().seq);
    assert_eq!(err_code(&err), "CALYX_LEDGER_CORRUPT");
    let err_message = tool_error_message(&err);
    assert!(err_message.contains("no exact mcp/cli anchor ledger row"));
    write_anchor_fsv(
        "mcp-provenance-anchor-fail-closed.json",
        cx_id,
        &stored,
        &entries,
        err_code(&err),
        &err_message,
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn lineage_rejects_malformed_anchor_payload() {
    let (root, resolved, vault) = test_vault("anchor-malformed");
    let cx = sample_constellation(&vault, resolved.vault_id);
    let cx_id = cx.cx_id;
    vault.put(cx).unwrap();
    vault.flush().unwrap();
    let corrupt_ref = vault
        .anchor_with_ledger_entry(
            cx_id,
            test_anchor(),
            EntryKind::Ingest,
            SubjectId::Cx(cx_id),
            b"{not-json".to_vec(),
            ActorId::Service("calyx-mcp-test".to_string()),
        )
        .unwrap();
    vault.flush().unwrap();

    let err = core::lineage_for_resolved(&resolved, cx_id).unwrap_err();

    assert_eq!(err_code(&err), "CALYX_LEDGER_CORRUPT");
    assert!(tool_error_message(&err).contains(&format!(
        "decode ledger payload seq={} kind=Ingest",
        corrupt_ref.seq
    )));
    fs::remove_dir_all(root).ok();
}

#[test]
fn verify_chain_reports_ok_and_fails_closed_when_broken() {
    let mut store = chain_store(3);
    let ok =
        serde_json::to_value(core::verify_chain_for_store(&store, None, None).unwrap()).unwrap();

    assert_eq!(ok, json!({"status":"ok","checked":3,"break_at":null}));

    mutate_row(&mut store, 1, |bytes| bytes[8] ^= 1);
    let err = core::verify_chain_for_store(&store, None, None).unwrap_err();

    assert_eq!(err_code(&err), "CALYX_LEDGER_CHAIN_BROKEN");
}

#[test]
fn verify_chain_rejects_inverted_range_as_invalid_params() {
    let store = chain_store(1);
    let err = core::verify_chain_for_store(&store, Some(9), Some(1)).unwrap_err();

    assert!(matches!(err, crate::server::ToolError::InvalidParams(_)));
}

#[test]
fn missing_aster_ledger_state_maps_to_aster_corrupt_for_mcp() {
    let root = temp_root("missing-ledger-state");
    fs::create_dir_all(&root).unwrap();
    let err = super::open_ledger_view(&root).unwrap_err();

    assert_eq!(err_code(&err), "CALYX_ASTER_CORRUPT_SHARD");
    fs::remove_dir_all(root).ok();
}

#[test]
fn answer_trace_scans_home_and_returns_retrieval_steps() {
    let _guard = ENV_LOCK.lock().unwrap();
    let (root, _resolved, vault) = test_vault("answer-trace");
    let old_home = std::env::var_os("CALYX_HOME");
    unsafe {
        std::env::set_var("CALYX_HOME", &root);
    }
    let cx_id = CxId::from_bytes([9; 16]);
    let answer_id = b"answer-523".to_vec();
    let payload = json!({
        "complete": true,
        "expected_hops": 1,
        "path": [{
            "hop": 0,
            "cx_id": cx_id.to_string(),
            "score": 0.75,
            "ledger_ref": {"seq": 0}
        }],
        "fusion_weights": FusionWeights {
            mode: FusionMode::Rrf,
            k: 1,
            candidates: vec![cx_id],
            weights: Vec::new(),
            single_slot: None,
        },
    });
    vault
        .append_ledger_entry(
            EntryKind::Answer,
            SubjectId::Query(answer_id.clone()),
            serde_json::to_vec(&payload).unwrap(),
            ActorId::Service("unit".to_string()),
        )
        .unwrap();
    vault.flush().unwrap();

    let out = serde_json::to_value(core::answer_trace("answer-523").unwrap()).unwrap();

    assert_eq!(out["answer_id"], core::hex(&answer_id));
    assert_eq!(out["complete"], true);
    assert_eq!(out["trusted"], true);
    assert_eq!(out["answer_seq"], 0);
    assert_eq!(out["retrieval_steps"][0]["cx_id"], cx_id.to_string());
    assert_eq!(out["kernel_cx_ids"][0], cx_id.to_string());
    restore_home(old_home);
    fs::remove_dir_all(root).ok();
}

#[test]
fn reproduce_missing_and_mismatch_fail_closed() {
    let missing = core::reproduce_report(&[], b"missing").unwrap_err();
    assert_eq!(err_code(&missing), "CALYX_VAULT_ACCESS_DENIED");
    let answer_id = b"answer-1".to_vec();
    let entry = LedgerEntry::new(
        0,
        [0; 32],
        EntryKind::Admin,
        SubjectId::Query(answer_id.clone()),
        serde_json::to_vec(&json!({
            "type": REPRODUCE_PAYLOAD_TAG,
            "answer_id": core::hex(&answer_id),
            "reproduced": false,
            "original_hits": [{"cx_id":"00000000000000000000000000000001","score":1.0}],
            "reproduced_hits": [{"cx_id":"00000000000000000000000000000002","score":1.0}],
        }))
        .unwrap(),
        ActorId::Service("unit".to_string()),
        1,
    );

    let report =
        serde_json::to_value(core::reproduce_report(&[entry], &answer_id).unwrap()).unwrap();

    assert_eq!(report["bit_parity"], false);
    assert_ne!(report["original_hash"], report["reproduced_hash"]);
}

#[test]
fn reproduce_rejects_malformed_reproduce_payload() {
    let answer_id = b"answer-bad".to_vec();
    let entry = LedgerEntry::new(
        7,
        [0; 32],
        EntryKind::Admin,
        SubjectId::Query(answer_id.clone()),
        b"{not-json".to_vec(),
        ActorId::Service("unit".to_string()),
        1,
    );

    let err = core::reproduce_report(&[entry], &answer_id).unwrap_err();

    assert_eq!(err_code(&err), "CALYX_LEDGER_CORRUPT");
    assert!(tool_error_message(&err).contains("decode ledger payload seq=7 kind=Admin"));
}

#[test]
fn anneal_status_contains_required_fields_from_proposal_row() {
    let (root, resolved, vault) = test_vault("anneal-status");
    vault
        .write_cf(
            ColumnFamily::AnnealOperators,
            b"propose-lens\0unit".to_vec(),
            serde_json::to_vec(&json!({
                "type": "add_lens",
                "name": "unit-lens",
                "rationale": "unit proposal",
            }))
            .unwrap(),
        )
        .unwrap();
    vault.flush().unwrap();

    let out = serde_json::to_value(status::anneal_status_for_resolved(&resolved).unwrap()).unwrap();

    assert_eq!(out["phase"], "tuning");
    assert_eq!(out["proposals"][0]["rationale"], "unit proposal");
    assert!(out.get("tripwires").is_some());
    assert!(out.get("p99_latency_ms").is_some());
    fs::remove_dir_all(root).ok();
}

fn test_vault(name: &str) -> (std::path::PathBuf, ResolvedVault, AsterVault) {
    let root = temp_root(name);
    let vault_id = VaultId::from_ulid(Ulid::new());
    let path = root.join("vaults").join(vault_id.to_string());
    let vault = AsterVault::new_durable(
        &path,
        vault_id,
        vault_salt(vault_id, name),
        VaultOptions {
            panel: Some(panel()),
            ..VaultOptions::default()
        },
    )
    .unwrap();
    let resolved = ResolvedVault {
        path,
        name: name.to_string(),
        vault_id,
    };
    (root, resolved, vault)
}

fn sample_constellation(vault: &AsterVault, vault_id: VaultId) -> calyx_core::Constellation {
    let input = b"lineage input";
    let cx_id = vault.cx_id_for_input(input, 1);
    calyx_core::Constellation {
        cx_id,
        vault_id,
        panel_version: 1,
        created_at: 11,
        input_ref: InputRef {
            hash: *blake3::hash(input).as_bytes(),
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::from([(
            SlotId::new(0),
            SlotVector::Dense {
                dim: 2,
                data: vec![0.25, 0.75],
            },
        )]),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: vault.latest_seq().saturating_add(1),
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}

fn test_anchor() -> Anchor {
    Anchor {
        kind: AnchorKind::TestPass,
        value: AnchorValue::Bool(true),
        source: "unit".to_string(),
        observed_at: 12,
        confidence: 1.0,
    }
}

fn panel() -> Panel {
    let slot = SlotId::new(0);
    Panel {
        version: 1,
        slots: vec![Slot {
            slot_id: slot,
            slot_key: SlotKey::new(slot, "unit"),
            lens_id: LensId::from_bytes([4; 16]),
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
        }],
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn chain_store(count: usize) -> MemoryLedgerStore {
    let mut appender = LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(10))
        .expect("open appender");
    for seq in 0..count {
        appender
            .append(
                EntryKind::Ingest,
                SubjectId::Cx(CxId::from_bytes([seq as u8; 16])),
                format!("payload-{seq}").into_bytes(),
                ActorId::Service("verify-test".to_string()),
            )
            .expect("append entry");
    }
    appender.into_store()
}

fn mutate_row(store: &mut MemoryLedgerStore, seq: u64, mutate: impl FnOnce(&mut Vec<u8>)) {
    let mut rows = store.scan().unwrap();
    let row = rows
        .iter_mut()
        .find(|row| row.seq == seq)
        .expect("row to mutate");
    mutate(&mut row.bytes);
    let mut mutated = MemoryLedgerStore::default();
    for LedgerRow { seq, bytes } in rows {
        mutated.insert_raw(seq, bytes);
    }
    *store = mutated;
}

fn err_code(err: &crate::server::ToolError) -> &str {
    match err {
        crate::server::ToolError::Calyx(error) => error.code,
        crate::server::ToolError::InvalidParams(_) => "INVALID_PARAMS",
    }
}

fn tool_error_message(err: &crate::server::ToolError) -> String {
    match err {
        crate::server::ToolError::Calyx(error) => error.to_string(),
        crate::server::ToolError::InvalidParams(message) => message.clone(),
    }
}

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-mcp-provenance-{name}-{}-{}",
        std::process::id(),
        crate::tools::vault::now_ms()
    ))
}

fn restore_home(old_home: Option<std::ffi::OsString>) {
    match old_home {
        Some(value) => unsafe {
            std::env::set_var("CALYX_HOME", value);
        },
        None => unsafe {
            std::env::remove_var("CALYX_HOME");
        },
    }
}

fn write_anchor_fsv(
    name: &str,
    cx_id: CxId,
    stored: &calyx_core::Constellation,
    entries: &[LedgerEntry],
    error_code: &str,
    error_message: &str,
) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    fs::create_dir_all(&root).unwrap();
    let payload = json!({
        "source_of_truth": "Aster durable Base anchor list plus Ledger CF rows read through AsterLedgerCfStore",
        "trigger": "MCP provenance lineage for base anchor with only a generic mcp-anchor ledger row",
        "cx_id": cx_id.to_string(),
        "base_anchor_kinds": stored
            .anchors
            .iter()
            .map(|anchor| format!("{:?}", anchor.kind))
            .collect::<Vec<_>>(),
        "ledger_entries": entries
            .iter()
            .map(|entry| {
                let payload = serde_json::from_slice::<Value>(&entry.payload).unwrap_or_else(|error| {
                    json!({"decode_error": error.to_string()})
                });
                json!({
                    "seq": entry.seq,
                    "kind": format!("{:?}", entry.kind),
                    "mode": payload.get("mode").cloned().unwrap_or(Value::Null),
                    "anchor_kind": payload.get("anchor_kind").cloned().unwrap_or(Value::Null),
                })
            })
            .collect::<Vec<_>>(),
        "error": {
            "code": error_code,
            "message": error_message,
        },
    });
    fs::write(
        root.join(name),
        serde_json::to_vec_pretty(&payload).unwrap(),
    )
    .unwrap();
}
