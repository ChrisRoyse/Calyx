use std::collections::BTreeMap;
use std::fs;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::ledger_view::AsterLedgerCfStore;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Asymmetry, Constellation, CxFlags, InputRef, LedgerRef,
    LensId, Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState, SlotVector,
    VaultId, VaultStore,
};
use calyx_ledger::{ActorId, EntryKind, LedgerEntry, SubjectId, get_provenance};
use serde_json::{Value, json};
use ulid::Ulid;

use super::*;

#[test]
fn provenance_lineage_reports_ingest_hash_and_anchor_seq() {
    let (root, resolved, vault) = test_vault("lineage");
    let cx = sample_constellation(&vault, resolved.vault_id);
    let cx_id = cx.cx_id;
    vault.put(cx).unwrap();
    vault.flush().unwrap();
    let stored = vault.get(cx_id, vault.snapshot()).unwrap();
    let anchor = Anchor {
        kind: AnchorKind::TestPass,
        value: AnchorValue::Bool(true),
        source: "unit".to_string(),
        observed_at: 12,
        confidence: 1.0,
    };
    vault.anchor(cx_id, anchor).unwrap();
    let anchor_ref = vault
        .append_ledger_entry(
            EntryKind::Ingest,
            SubjectId::Cx(cx_id),
            serde_json::to_vec(&json!({
                "mode": "cli-anchor",
                "anchor_kind": "test_pass",
            }))
            .unwrap(),
            ActorId::Service("calyx-cli-test".to_string()),
        )
        .unwrap();
    vault.flush().unwrap();

    let out = lineage(&resolved, cx_id).unwrap();

    assert_eq!(out.cx_id, cx_id.to_string());
    assert_eq!(out.ingest_seq, stored.provenance.seq);
    assert_eq!(out.ledger_chain_hash, hex_bytes(&stored.provenance.hash));
    assert_eq!(out.lens_measures.len(), 1);
    assert_eq!(out.anchors[0].kind, "test_pass");
    assert_eq!(out.anchors[0].ledger_seq, anchor_ref.seq);
    fs::remove_dir_all(root).ok();
}

#[test]
fn provenance_lineage_handles_cli_anchor_base_provenance() {
    let (root, resolved, vault) = test_vault("cli-anchor-lineage");
    let cx = sample_constellation(&vault, resolved.vault_id);
    let cx_id = cx.cx_id;
    vault.put(cx).unwrap();
    vault.flush().unwrap();
    let ingest_ref = vault.get(cx_id, vault.snapshot()).unwrap().provenance;
    let anchor = Anchor {
        kind: AnchorKind::Label("issue691".to_string()),
        value: AnchorValue::Enum("stable".to_string()),
        source: "unit".to_string(),
        observed_at: 13,
        confidence: 0.75,
    };
    let anchor_ref = vault
        .anchor_with_ledger_entry(
            cx_id,
            anchor,
            EntryKind::Ingest,
            SubjectId::Cx(cx_id),
            serde_json::to_vec(&json!({
                "mode": "cli-anchor",
                "anchor_kind": "label:issue691",
            }))
            .unwrap(),
            ActorId::Service("calyx-cli-test".to_string()),
        )
        .unwrap();
    vault.flush().unwrap();
    let anchored = vault.get(cx_id, vault.snapshot()).unwrap();
    assert_eq!(anchored.provenance.seq, anchor_ref.seq);

    let out = lineage(&resolved, cx_id).unwrap();

    assert_eq!(out.ingest_seq, ingest_ref.seq);
    assert_eq!(out.ledger_chain_hash, hex_bytes(&anchor_ref.hash));
    assert_eq!(out.anchors[0].kind, "label:issue691");
    assert_eq!(out.anchors[0].ledger_seq, anchor_ref.seq);
    fs::remove_dir_all(root).ok();
}

#[test]
fn provenance_lineage_refuses_generic_anchor_ledger_fallback() {
    let (root, resolved, vault) = test_vault("anchor-fail-closed");
    let cx = sample_constellation(&vault, resolved.vault_id);
    let cx_id = cx.cx_id;
    vault.put(cx).unwrap();
    vault.flush().unwrap();
    let anchor = Anchor {
        kind: AnchorKind::TestPass,
        value: AnchorValue::Bool(true),
        source: "unit".to_string(),
        observed_at: 12,
        confidence: 1.0,
    };
    vault.anchor(cx_id, anchor).unwrap();
    let generic_ref = vault
        .append_ledger_entry(
            EntryKind::Ingest,
            SubjectId::Cx(cx_id),
            serde_json::to_vec(&json!({
                "mode": "cli-anchor",
            }))
            .unwrap(),
            ActorId::Service("calyx-cli-test".to_string()),
        )
        .unwrap();
    vault.flush().unwrap();

    let stored = vault.get(cx_id, vault.snapshot()).unwrap();
    let ledger_store = AsterLedgerCfStore::open(&resolved.path).unwrap();
    let entries = get_provenance(&ledger_store, &NoQuarantine, cx_id).unwrap();
    let err = lineage(&resolved, cx_id).unwrap_err();

    assert_eq!(stored.anchors.len(), 1);
    assert_eq!(generic_ref.seq, entries.last().unwrap().seq);
    assert_eq!(err.code(), "CALYX_LEDGER_CORRUPT");
    assert!(err.to_string().contains("no exact cli anchor ledger row"));
    write_anchor_fsv(
        "cli-provenance-anchor-fail-closed.json",
        cx_id,
        &stored,
        &entries,
        err.code(),
        &err.to_string(),
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn verify_chain_intact_report_is_valid_json_shape() {
    let report = VerifyChainOut {
        status: "ok",
        checked: 2,
        break_at: None,
    };
    let value: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&report).unwrap()).expect("verify-chain JSON");

    assert_eq!(value["status"], "ok");
    assert_eq!(value["checked"], 2);
    assert!(value["break_at"].is_null());
}

#[test]
fn verify_chain_rejects_inverted_range() {
    let parsed = parse_verify_chain(&tokens(["v", "--from", "999", "--to", "1"])).unwrap();
    assert_eq!(
        parsed,
        Subcommand::VerifyChain(VerifyChainArgs {
            vault: "v".to_string(),
            from: Some(999),
            to: Some(1),
            progress_jsonl: None,
            time_budget_ms: None,
            batch_size: 8192,
        })
    );
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

    let out = status::anneal_status(&resolved.path, &vault).unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&out).unwrap()).expect("anneal-status JSON");

    assert_eq!(value["phase"], "tuning");
    assert!(value.get("tripwires").is_some());
    assert!(value.get("proposals").is_some());
    assert!(value.get("p99_latency_ms").is_some());
    assert_eq!(value["proposals"][0]["rationale"], "unit proposal");
    fs::remove_dir_all(root).ok();
}

#[test]
fn reproduce_missing_answer_and_mismatch_fail_closed() {
    let missing = reproduce_report(&[], b"missing").unwrap_err();
    assert_eq!(missing.code(), "CALYX_VAULT_ACCESS_DENIED");

    let answer_id = b"answer-1".to_vec();
    let entry = LedgerEntry::new(
        0,
        [0; 32],
        EntryKind::Admin,
        SubjectId::Query(answer_id.clone()),
        serde_json::to_vec(&json!({
            "type": REPRODUCE_PAYLOAD_TAG,
            "answer_id": hex_bytes(&answer_id),
            "reproduced": false,
            "original_hits": [{"cx_id":"00000000000000000000000000000001","score":1.0}],
            "reproduced_hits": [{"cx_id":"00000000000000000000000000000002","score":1.0}],
        }))
        .unwrap(),
        ActorId::Service("unit".to_string()),
        1,
    );
    let report = reproduce_report(&[entry], &answer_id).unwrap();

    assert!(!report.bit_parity);
    assert_ne!(report.original_hash, report.reproduced_hash);
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

fn sample_constellation(vault: &AsterVault, vault_id: VaultId) -> Constellation {
    let input = b"lineage input";
    let cx_id = vault.cx_id_for_input(input, 1);
    Constellation {
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

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-cli-provenance-{name}-{}-{}",
        std::process::id(),
        crate::cmd::vault::now_ms()
    ))
}

fn tokens<const N: usize>(items: [&str; N]) -> Vec<String> {
    items.into_iter().map(str::to_string).collect()
}

fn write_anchor_fsv(
    name: &str,
    cx_id: CxId,
    stored: &Constellation,
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
        "trigger": "CLI provenance lineage for base anchor with only a generic cli-anchor ledger row",
        "cx_id": cx_id.to_string(),
        "base_anchor_kinds": stored
            .anchors
            .iter()
            .map(|anchor| anchor_kind_key(&anchor.kind))
            .collect::<Vec<_>>(),
        "ledger_entries": entries
            .iter()
            .map(|entry| {
                let payload = json_payload(entry);
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
