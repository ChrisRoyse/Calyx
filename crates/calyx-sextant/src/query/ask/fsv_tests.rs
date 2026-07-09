use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AbsentReason, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId,
    VaultStore,
};
use serde_json::json;

use crate::error::CALYX_ANSWER_UNGROUNDED;
use crate::query::{AskSpec, CrossModelPlan, PlanStep, ask, execute};

#[test]
#[ignore = "manual FSV for issue #466"]
fn issue466_ask_fsv_writes_readback_artifacts() {
    let root = calyx_fsv::required_fsv_root("CALYX_FSV_ROOT").join("issue466-ask");
    fs::remove_dir_all(&root).ok();
    fs::create_dir_all(&root).unwrap();
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue466-ask-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let before = raw_state(&vault);
    println!("[BEFORE] {}", before);

    let first = put_dense(&vault, b"issue466-first", 101, [0.9, 0.1]);
    let second = put_dense(&vault, b"issue466-second", 102, [0.1, 0.9]);
    let missing_lens = put_absent(&vault, b"issue466-missing-lens", 103);
    vault.flush().unwrap();
    let snapshot = vault.latest_seq();

    let grounded = ask(
        &vault,
        &AskSpec {
            question: "Which synthetic constellations ground the answer?".to_string(),
            context_cx_ids: vec![first, second],
            top_k: 2,
            oracle: false,
        },
        snapshot,
    )
    .unwrap_err();
    let top_one = ask(
        &vault,
        &AskSpec {
            question: "Select one grounding".to_string(),
            context_cx_ids: vec![first, second],
            top_k: 1,
            oracle: true,
        },
        snapshot,
    )
    .unwrap_err();
    let full_vault = ask(
        &vault,
        &AskSpec {
            question: "Search the full vault".to_string(),
            context_cx_ids: Vec::new(),
            top_k: 2,
            oracle: false,
        },
        snapshot,
    )
    .unwrap_err();
    let executor = execute(
        &vault,
        CrossModelPlan {
            steps: vec![PlanStep::Ask {
                question: "Executor ASK".to_string(),
                context_cx_ids: vec![first],
                top_k: 1,
                oracle: false,
            }],
            estimated_cost_ms: 1.0,
            explain: None,
        },
    )
    .unwrap_err();
    let empty_question = ask(
        &vault,
        &AskSpec {
            question: " ".to_string(),
            context_cx_ids: vec![first],
            top_k: 1,
            oracle: false,
        },
        snapshot,
    )
    .unwrap_err();
    let ungrounded = ask(
        &vault,
        &AskSpec {
            question: "Unknown candidate".to_string(),
            context_cx_ids: vec![CxId::from_input(b"absent", 1, b"issue466")],
            top_k: 1,
            oracle: false,
        },
        snapshot,
    )
    .unwrap_err();
    let unavailable = ask(
        &vault,
        &AskSpec {
            question: "Unavailable lens".to_string(),
            context_cx_ids: vec![missing_lens],
            top_k: 1,
            oracle: false,
        },
        snapshot,
    )
    .unwrap_err();
    vault.flush().unwrap();
    let after = raw_state(&vault);
    println!("[AFTER ] {}", after);
    println!("[ASK   ] grounded = {}", grounded.code);
    println!("[ASK   ] top_one = {}", top_one.code);
    println!("[ASK   ] full_vault = {}", full_vault.code);
    println!("[ASK   ] executor = {}", executor.code);
    println!("[EDGE  ] empty_question = {}", empty_question.code);
    println!("[EDGE  ] ungrounded = {}", ungrounded.code);
    println!("[EDGE  ] unavailable = {}", unavailable.code);

    assert_eq!(grounded.code, CALYX_ANSWER_UNGROUNDED);
    assert_eq!(top_one.code, CALYX_ANSWER_UNGROUNDED);
    assert_eq!(full_vault.code, CALYX_ANSWER_UNGROUNDED);
    assert_eq!(executor.code, CALYX_ANSWER_UNGROUNDED);
    assert!(!grounded.message.contains(&hex(first.as_bytes())));
    assert!(!grounded.message.contains(&hex(second.as_bytes())));

    let readback = json!({
        "source_of_truth": "Aster durable Base/Ledger/slot_00 CF rows plus ASK error payload readback",
        "trigger": "query::ask and executor PlanStep::Ask at pinned snapshot",
        "snapshot": snapshot,
        "before": before,
        "after": after,
        "fail_closed_before_fabricated_grounding": {
            "code": grounded.code,
            "message": grounded.message,
            "visible_context_cx_ids": [first.to_string(), second.to_string()],
        },
        "top_one_error": {
            "code": top_one.code,
            "message": top_one.message,
        },
        "full_vault_error": {
            "code": full_vault.code,
            "message": full_vault.message,
        },
        "executor_error": {
            "code": executor.code,
            "message": executor.message,
        },
        "edges": {
            "empty_question_code": empty_question.code,
            "ungrounded_code": ungrounded.code,
            "unavailable_lens_code": unavailable.code,
        },
        "fixture_requested_ledger_seqs": [101, 102],
        "observed_base_keys_hex": raw_keys_hex(&vault, ColumnFamily::Base),
        "observed_ledger_rows": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger).unwrap().len(),
        "observed_slot_00_keys_hex": raw_keys_hex(&vault, ColumnFamily::slot(SlotId::new(0))),
        "physical_cf_files": {
            "base": physical_files(&vault_dir.join("cf").join("base")),
            "ledger": physical_files(&vault_dir.join("cf").join("ledger")),
            "slot_00": physical_files(&vault_dir.join("cf").join("slot_00")),
        }
    });
    fs::write(
        root.join("issue466-ask-readback.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
    println!("issue466_fsv_root={}", root.display());
}

fn put_dense(vault: &AsterVault, input: &[u8], seq: u64, data: [f32; 2]) -> CxId {
    let cx_id = CxId::from_input(input, 1, b"issue466-ask-fsv-salt");
    vault
        .put(constellation(
            cx_id,
            LedgerRef {
                seq,
                hash: [seq as u8; 32],
            },
            SlotVector::Dense {
                dim: 2,
                data: data.to_vec(),
            },
        ))
        .unwrap();
    cx_id
}

fn put_absent(vault: &AsterVault, input: &[u8], seq: u64) -> CxId {
    let cx_id = CxId::from_input(input, 1, b"issue466-ask-fsv-salt");
    vault
        .put(constellation(
            cx_id,
            LedgerRef {
                seq,
                hash: [seq as u8; 32],
            },
            SlotVector::Absent {
                reason: AbsentReason::LensUnavailable,
            },
        ))
        .unwrap();
    cx_id
}

fn constellation(
    cx_id: CxId,
    provenance: LedgerRef,
    vector: SlotVector,
) -> calyx_core::Constellation {
    let mut input_hash = [0_u8; 32];
    input_hash[..16].copy_from_slice(cx_id.as_bytes());
    let mut slots = BTreeMap::new();
    slots.insert(SlotId::new(0), vector);
    calyx_core::Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 1,
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some(format!("synthetic://issue466/{cx_id}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance,
        flags: CxFlags::default(),
    }
}

fn raw_keys_hex(vault: &AsterVault, cf: ColumnFamily) -> Vec<String> {
    vault
        .scan_cf_at(vault.latest_seq(), cf)
        .unwrap()
        .into_iter()
        .map(|(key, _)| hex(&key))
        .collect()
}

fn raw_state(vault: &AsterVault) -> serde_json::Value {
    json!({
        "latest_seq": vault.latest_seq(),
        "base_rows": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::Base).unwrap().len(),
        "ledger_rows": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger).unwrap().len(),
        "slot_00_rows": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::slot(SlotId::new(0))).unwrap().len(),
    })
}

fn physical_files(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = entries
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
}
