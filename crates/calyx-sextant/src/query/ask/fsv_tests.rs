use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AbsentReason, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId,
    VaultStore,
};
use serde_json::json;

use crate::query::{AskSpec, CrossModelPlan, PlanStep, ask, execute};

#[test]
#[ignore = "manual FSV for issue #466"]
fn issue466_ask_fsv_writes_readback_artifacts() {
    let root = std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .expect("set CALYX_FSV_ROOT to the issue #466 FSV directory");
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

    let happy = ask(
        &vault,
        &AskSpec {
            question: "Which synthetic constellations ground the answer?".to_string(),
            context_cx_ids: vec![first, second],
            top_k: 2,
            oracle: false,
        },
        snapshot,
    )
    .unwrap();
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
    .unwrap();
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
    .unwrap();
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
    .unwrap();
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
    println!("[ASK   ] answer = {}", happy.answer);
    println!("[ASK   ] grounding = {:?}", rows_json(&happy.grounding));
    println!("[EDGE  ] empty_question = {}", empty_question.code);
    println!("[EDGE  ] ungrounded = {}", ungrounded.code);
    println!("[EDGE  ] unavailable = {}", unavailable.code);

    let readback = json!({
        "source_of_truth": "Aster durable Base/Ledger/slot_00 CF rows plus ASK readback JSON",
        "trigger": "query::ask and executor PlanStep::Ask at pinned snapshot",
        "snapshot": snapshot,
        "before": before,
        "after": after,
        "happy": {
            "answer": happy.answer,
            "grounding": rows_json(&happy.grounding),
            "gaps": happy.gaps,
            "oracle_conf": happy.oracle_conf,
        },
        "top_one_grounding_count": top_one.grounding.len(),
        "full_vault_grounding_count": full_vault.grounding.len(),
        "executor_rows": rows_json(&executor.rows),
        "edges": {
            "empty_question_code": empty_question.code,
            "ungrounded_code": ungrounded.code,
            "unavailable_lens_code": unavailable.code,
            "oracle_stub_conf": top_one.oracle_conf,
        },
        "fixture_requested_ledger_seqs": [101, 102],
        "observed_happy_ledger_seqs": ledger_seqs(&happy.grounding),
        "observed_executor_ledger_seqs": ledger_seqs(&executor.rows),
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

fn rows_json(rows: &[crate::query::ProvenancedRow]) -> Vec<serde_json::Value> {
    rows.iter()
        .map(|row| {
            json!({
                "key_hex": hex(row.key.as_bytes()),
                "score": row.score,
                "ledger_ref": row.ledger_ref,
            })
        })
        .collect()
}

fn ledger_seqs(rows: &[crate::query::ProvenancedRow]) -> Vec<u64> {
    rows.iter()
        .filter_map(|row| row.ledger_ref.as_ref().map(|ledger| ledger.seq))
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
