use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::dedup::{
    DedupAction, DedupPolicy, DedupRestoreSnapshot, DedupResult, EpochSecs, IngestInput,
    TauStrategy, TctCosineConfig, ingest_at,
};
use calyx_aster::vault::encode::{decode_write_batch, encode_constellation_base};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::wal::replay_dir;
use calyx_core::{CxId, Modality, SlotId, SlotVector, VaultId};
use calyx_ledger::decode as decode_ledger;
use serde_json::{Value, json};

#[path = "support/dedup_fsv_io.rs"]
mod dedup_fsv_io;

use dedup_fsv_io::{fsv_root, list_dir_files as list_files, reset_dir, write_blake3_sums};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const SALT: &str = "dedup-audit-readback-salt";

#[test]
fn dedup_audit_readback_prints_reversible_undo_bytes() {
    let (root, keep) = fsv_root("CALYX_DEDUP_AUDIT_FSV_ROOT", "calyx-dedup-audit-fsv");
    let before = json!({
        "root_exists_before_reset": root.exists(),
        "files_before_reset": list_files(&root),
    });
    reset_dir(&root);
    let vault_dir = root.join("audit_undo").join("vault");
    let vault = durable_vault(&vault_dir);
    let first = ingest_at(
        &vault,
        &input("audit-readback-a", [1.0, 0.0], [1.0, 0.0]),
        EpochSecs(100),
        None,
    )
    .expect("first");
    let second = ingest_at(
        &vault,
        &input("audit-readback-b", [1.0, 0.0], [0.0, 1.0]),
        EpochSecs(200),
        None,
    )
    .expect("second");
    let third = ingest_at(
        &vault,
        &input("audit-readback-c", [1.0, 0.0], [-1.0, 0.0]),
        EpochSecs(300),
        None,
    )
    .expect("third");
    vault.flush().expect("flush before readback");
    let into = new_id(first);
    assert_eq!(merge_occurrence(second), 1);
    assert_eq!(merge_occurrence(third), 2);

    let audit_before = stdout_json(readback(&[
        "readback",
        "dedup-audit",
        "--vault",
        &vault_dir.display().to_string(),
        "--cx-id",
        &into.to_string(),
    ]));
    let token = serde_json::to_string(&audit_before["reversal_token"]).expect("token");
    let cx_list_before = stdout_json(readback(&[
        "readback",
        "cx-list",
        "--vault",
        &vault_dir.display().to_string(),
    ]));
    let expected_restored_base_hex = restore_base_hex_from_ledger(&vault_dir);
    let mut bad_token = audit_before["reversal_token"].clone();
    bad_token["vault_id"] = json!("01ARZ3NDEKTSV4RRFFQ69G5FAW");
    let wrong_vault_undo = readback(&[
        "readback",
        "dedup-undo",
        "--vault",
        &vault_dir.display().to_string(),
        "--token",
        &serde_json::to_string(&bad_token).expect("bad token"),
    ]);
    assert!(
        !wrong_vault_undo.status.success(),
        "wrong-vault token unexpectedly succeeded"
    );
    let wrong_vault_stderr = String::from_utf8_lossy(&wrong_vault_undo.stderr).to_string();
    assert!(wrong_vault_stderr.contains("CALYX_DEDUP_WRONG_VAULT"));
    let undo = stdout_json(readback(&[
        "readback",
        "dedup-undo",
        "--vault",
        &vault_dir.display().to_string(),
        "--token",
        &token,
    ]));
    let cx_list_after = stdout_json(readback(&[
        "readback",
        "cx-list",
        "--vault",
        &vault_dir.display().to_string(),
    ]));
    let audit_after = stdout_json(readback(&[
        "readback",
        "dedup-audit",
        "--vault",
        &vault_dir.display().to_string(),
        "--cx-id",
        &into.to_string(),
    ]));
    let recurrence_after = stdout_json(readback(&[
        "readback",
        "recurrence-series",
        "--vault",
        &vault_dir.display().to_string(),
        "--cx-id",
        &into.to_string(),
    ]));
    let ledger_undo = stdout(readback(&[
        "readback",
        "--cf",
        "ledger",
        "--vault",
        &vault_dir.display().to_string(),
        "--seq",
        "3",
    ]));

    let value = json!({
        "before": before,
        "into": into,
        "audit_before": audit_before,
        "cx_list_before": cx_list_before,
        "expected_restored_base_hex": expected_restored_base_hex,
        "wrong_vault_undo_stderr": wrong_vault_stderr,
        "undo": undo,
        "cx_list_after": cx_list_after,
        "audit_after": audit_after,
        "recurrence_after": recurrence_after,
        "ledger_undo": ledger_undo,
        "after": {"files": list_files(&root)},
    });
    let out = root.join("dedup-audit-readback.json");
    fs::write(&out, serde_json::to_vec_pretty(&value).expect("json")).expect("write json");
    write_blake3_sums(&root);
    println!(
        "{}",
        serde_json::to_string_pretty(&value).expect("print json")
    );

    assert_eq!(value["audit_before"]["merges"].as_array().unwrap().len(), 2);
    assert_eq!(
        value["audit_before"]["occurrences"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(value["cx_list_before"].as_array().unwrap().len(), 1);
    assert_eq!(value["undo"]["restored"].as_array().unwrap().len(), 2);
    assert_eq!(value["cx_list_after"].as_array().unwrap().len(), 3);
    assert_eq!(
        value["audit_after"]["undo_entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    for expected in value["expected_restored_base_hex"].as_array().unwrap() {
        let cx_id = expected["cx_id"].as_str().unwrap();
        let restored = value["cx_list_after"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["cx_id"].as_str() == Some(cx_id))
            .expect("restored row in cx-list");
        assert_eq!(restored["base_hex"], expected["base_hex"]);
    }
    assert_eq!(value["recurrence_after"]["occurrence_count"], json!(0));
    assert!(
        value["ledger_undo"]
            .as_str()
            .unwrap()
            .contains("4465647570556e646f")
    );

    if !keep {
        let _ = fs::remove_dir_all(root);
    }
}

fn durable_vault(vault_dir: &Path) -> AsterVault {
    AsterVault::new_durable(
        vault_dir,
        vault_id(),
        SALT.as_bytes().to_vec(),
        VaultOptions {
            dedup_policy: Some(tct_policy()),
            ..VaultOptions::default()
        },
    )
    .expect("open durable vault")
}

fn tct_policy() -> DedupPolicy {
    DedupPolicy::TctCosine(
        TctCosineConfig::new(
            vec![slot(0)],
            TauStrategy::PerSlot(vec![(slot(0), 0.90)]),
            DedupAction::RecurrenceSeries,
        )
        .expect("policy"),
    )
}

fn input(name: &str, content: [f32; 2], temporal: [f32; 2]) -> IngestInput {
    IngestInput::new(name.as_bytes().to_vec(), 41, Modality::Text)
        .with_slot(
            slot(0),
            SlotVector::Dense {
                dim: 2,
                data: content.to_vec(),
            },
        )
        .with_slot(
            slot(20),
            SlotVector::Dense {
                dim: 2,
                data: temporal.to_vec(),
            },
        )
        .with_temporal_slot(slot(20))
}

fn new_id(result: DedupResult) -> CxId {
    match result {
        DedupResult::New(id) => id,
        DedupResult::DedupMerge { .. } | DedupResult::ExactDuplicate(_) => {
            panic!("expected new id")
        }
    }
}

fn merge_occurrence(result: DedupResult) -> u64 {
    match result {
        DedupResult::DedupMerge { occurrence, .. } => occurrence.0,
        DedupResult::New(_) | DedupResult::ExactDuplicate(_) => panic!("expected merge"),
    }
}

fn readback(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .args(args)
        .output()
        .expect("run calyx")
}

fn stdout_json(output: Output) -> Value {
    serde_json::from_str(&stdout(output)).expect("json stdout")
}

fn stdout(output: Output) -> String {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn restore_base_hex_from_ledger(vault_dir: &Path) -> Value {
    let replay = replay_dir(vault_dir.join("wal")).expect("replay wal");
    let mut rows = Vec::new();
    for record in replay.records {
        for row in decode_write_batch(&record.payload).expect("decode batch") {
            if row.cf != ColumnFamily::Ledger {
                continue;
            }
            let entry = decode_ledger(&row.value).expect("decode ledger");
            let Ok(payload) = serde_json::from_slice::<Value>(&entry.payload) else {
                continue;
            };
            if payload["dedup_result"] != json!("DedupMerge") {
                continue;
            }
            let restore: DedupRestoreSnapshot =
                serde_json::from_value(payload["restore"].clone()).expect("restore snapshot");
            rows.push(json!({
                "cx_id": restore.merged_from,
                "base_hex": hex_bytes(
                    &encode_constellation_base(&restore.candidate).expect("base bytes")
                ),
            }));
        }
    }
    rows.sort_by(|left, right| {
        left["cx_id"]
            .as_str()
            .unwrap()
            .cmp(right["cx_id"].as_str().unwrap())
    });
    json!(rows)
}

fn hex_bytes(bytes: &[u8]) -> String {
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

fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}

fn vault_id() -> VaultId {
    VAULT_ID.parse().expect("valid vault id")
}
