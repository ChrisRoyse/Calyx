use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId, VaultStore,
};
use serde_json::json;

use crate::error::{CALYX_SEXTANT_QUERY_SHAPE, CALYX_SEXTANT_VECTOR_FUSION_UNWIRED};
use crate::query::PlanStep;

use super::{ExecState, execute, execute_vector_fusion};

#[test]
fn issue919_vector_fusion_fails_closed_with_persisted_candidates() {
    let root = fsv_root().join("issue919-vector-fusion-unwired");
    fs::remove_dir_all(&root).ok();
    fs::create_dir_all(&root).unwrap();
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue919-vector-fusion-unwired-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();

    let before_empty = vector_source_state(&vault);
    println!("[ISSUE919 BEFORE EMPTY] {before_empty}");
    let first = put_dense(&vault, b"issue919-candidate-a", 101, [1.0, 0.0]);
    let second = put_dense(&vault, b"issue919-candidate-b", 102, [0.0, 1.0]);
    vault.flush().unwrap();
    let before = vector_source_state(&vault);
    println!("[ISSUE919 BEFORE CANDIDATES] {before}");

    let mut state = ExecState {
        rows: Vec::new(),
        candidates: BTreeSet::from([first, second]),
        total_scanned: 0,
    };
    let before_candidates = state.candidates.clone();
    let err = execute_vector_fusion(&vault, vault.latest_seq(), &mut state, 1, &[1.0, 0.0], 2)
        .unwrap_err();
    let after_fail_closed = vector_source_state(&vault);
    println!("[ISSUE919 AFTER FAIL CLOSED] {after_fail_closed}");

    assert_eq!(err.code, CALYX_SEXTANT_VECTOR_FUSION_UNWIRED);
    assert!(err.message.contains("refusing synthetic ranking"));
    assert_eq!(state.candidates, before_candidates);
    assert!(state.rows.is_empty());
    assert_eq!(before, after_fail_closed);

    let edge_empty_before = vector_source_state(&vault);
    println!("[ISSUE919 EDGE empty BEFORE] {edge_empty_before}");
    let mut empty_state = ExecState {
        rows: Vec::new(),
        candidates: BTreeSet::new(),
        total_scanned: 0,
    };
    execute_vector_fusion(
        &vault,
        vault.latest_seq(),
        &mut empty_state,
        1,
        &[1.0, 0.0],
        2,
    )
    .unwrap();
    let edge_empty_after = vector_source_state(&vault);
    println!("[ISSUE919 EDGE empty AFTER] {edge_empty_after}");
    assert!(empty_state.rows.is_empty());
    assert!(empty_state.candidates.is_empty());
    assert_eq!(edge_empty_before, edge_empty_after);

    let edge_zero_limit_before = vector_source_state(&vault);
    println!("[ISSUE919 EDGE zero_limit BEFORE] {edge_zero_limit_before}");
    let mut zero_limit_state = ExecState {
        rows: Vec::new(),
        candidates: BTreeSet::from([first]),
        total_scanned: 0,
    };
    let zero_limit = execute_vector_fusion(
        &vault,
        vault.latest_seq(),
        &mut zero_limit_state,
        1,
        &[1.0, 0.0],
        0,
    )
    .unwrap_err();
    let edge_zero_limit_after = vector_source_state(&vault);
    println!("[ISSUE919 EDGE zero_limit AFTER] {edge_zero_limit_after}");
    assert_eq!(zero_limit.code, CALYX_SEXTANT_QUERY_SHAPE);
    assert!(zero_limit.message.contains("positive limit"));
    assert!(zero_limit_state.rows.is_empty());
    assert_eq!(edge_zero_limit_before, edge_zero_limit_after);

    let edge_nonfinite_before = vector_source_state(&vault);
    println!("[ISSUE919 EDGE nonfinite BEFORE] {edge_nonfinite_before}");
    let mut nonfinite_state = ExecState {
        rows: Vec::new(),
        candidates: BTreeSet::from([first]),
        total_scanned: 0,
    };
    let nonfinite = execute_vector_fusion(
        &vault,
        vault.latest_seq(),
        &mut nonfinite_state,
        1,
        &[1.0, f32::NAN],
        2,
    )
    .unwrap_err();
    let edge_nonfinite_after = vector_source_state(&vault);
    println!("[ISSUE919 EDGE nonfinite AFTER] {edge_nonfinite_after}");
    assert_eq!(nonfinite.code, CALYX_SEXTANT_QUERY_SHAPE);
    assert!(nonfinite.message.contains("finite query vector"));
    assert!(nonfinite_state.rows.is_empty());
    assert_eq!(edge_nonfinite_before, edge_nonfinite_after);

    let public_empty_query = execute(
        &vault,
        crate::query::CrossModelPlan {
            steps: vec![PlanStep::VectorFusion {
                lens_ids: vec![calyx_core::LensId::from_parts(
                    "issue919", b"weights", b"corpus", b"2xf32",
                )],
                query_vec: vec![1.0, 0.0],
                limit: 2,
            }],
            estimated_cost_ms: 1.0,
            explain: None,
        },
    )
    .unwrap();
    assert!(public_empty_query.rows.is_empty());

    let readback = json!({
        "source_of_truth": "Aster durable Base, Ledger, and slot_00 column-family rows plus this readback artifact",
        "trigger": "PlanStep::VectorFusion internal executor path at pinned durable Aster snapshot",
        "expected_behavior": "non-empty persisted candidates must not produce synthetic QueryResult rows until vector fusion is wired to real slot-index search",
        "snapshot": vault.latest_seq(),
        "persisted_candidates": [first.to_string(), second.to_string()],
        "persisted_candidate_hex": [hex(first.as_bytes()), hex(second.as_bytes())],
        "before_empty": before_empty,
        "before_candidates": before,
        "after_fail_closed": after_fail_closed,
        "fail_closed_error": {
            "code": err.code,
            "message": err.message,
        },
        "state_after_fail_closed": {
            "rows": state.rows.len(),
            "candidates": state.candidates.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "total_scanned": state.total_scanned,
        },
        "public_empty_candidate_query": {
            "rows": public_empty_query.rows.len(),
            "total_scanned": public_empty_query.total_scanned,
        },
        "edges": {
            "empty_candidates": {
                "before": edge_empty_before,
                "after": edge_empty_after,
                "rows": empty_state.rows.len(),
                "candidates": empty_state.candidates.len(),
            },
            "zero_limit": {
                "before": edge_zero_limit_before,
                "after": edge_zero_limit_after,
                "code": zero_limit.code,
                "message": zero_limit.message,
                "rows": zero_limit_state.rows.len(),
            },
            "nonfinite_query": {
                "before": edge_nonfinite_before,
                "after": edge_nonfinite_after,
                "code": nonfinite.code,
                "message": nonfinite.message,
                "rows": nonfinite_state.rows.len(),
            }
        },
        "observed_base_keys_hex": raw_keys_hex(&vault, ColumnFamily::Base),
        "observed_slot_00_keys_hex": raw_keys_hex(&vault, ColumnFamily::slot(SlotId::new(0))),
        "physical_cf_files": {
            "base": physical_files(&vault_dir.join("cf").join("base")),
            "ledger": physical_files(&vault_dir.join("cf").join("ledger")),
            "slot_00": physical_files(&vault_dir.join("cf").join("slot_00")),
        }
    });
    let readback_path = root.join("issue919-vector-fusion-unwired-readback.json");
    fs::write(
        &readback_path,
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
    let stored: serde_json::Value =
        serde_json::from_slice(&fs::read(&readback_path).unwrap()).unwrap();
    assert_eq!(stored, readback);
    println!("issue919_fsv_root={}", root.display());
    println!(
        "issue919_readback={}",
        fs::read_to_string(&readback_path).unwrap()
    );
}

fn fsv_root() -> PathBuf {
    calyx_fsv::fsv_root_or_else("CALYX_FSV_ROOT", || PathBuf::from("target").join("fsv"))
}

fn put_dense(vault: &AsterVault, input: &[u8], seq: u64, data: [f32; 2]) -> CxId {
    let cx_id = CxId::from_input(input, 1, b"issue919-vector-fusion-unwired-salt");
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
            pointer: Some(format!("synthetic://issue919/{cx_id}")),
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

fn vector_source_state(vault: &AsterVault) -> serde_json::Value {
    json!({
        "latest_seq": vault.latest_seq(),
        "base_rows": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::Base).unwrap().len(),
        "ledger_rows": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger).unwrap().len(),
        "slot_00_rows": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::slot(SlotId::new(0))).unwrap().len(),
        "base_keys_hex": raw_keys_hex(vault, ColumnFamily::Base),
        "slot_00_keys_hex": raw_keys_hex(vault, ColumnFamily::slot(SlotId::new(0))),
    })
}

fn raw_keys_hex(vault: &AsterVault, cf: ColumnFamily) -> Vec<String> {
    vault
        .scan_cf_at(vault.latest_seq(), cf)
        .unwrap()
        .into_iter()
        .map(|(key, _)| hex(&key))
        .collect()
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
