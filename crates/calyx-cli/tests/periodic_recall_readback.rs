use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use calyx_aster::dedup::EpochSecs;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Constellation, CxFlags, InputRef, LedgerRef, Modality, VaultId, VaultStore};
use calyx_loom::recurrence::{OccurrenceContext, SeriesStore};
use calyx_testkit::fsv::{
    fsv_root as test_fsv_root, list_files, reset_dir, write_blake3_sums, write_json,
};
use serde_json::{Value, json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const SALT: &[u8] = b"periodic-recall-readback-salt";
const TUESDAY_2024_01_02_14H_UTC: i64 = 1_704_204_000;
const WEEK_SECS: i64 = 604_800;

#[test]
fn periodic_recall_readback_writes_fit_recall_and_edges() {
    let (root, keep_root) = test_fsv_root(
        "CALYX_PERIODIC_RECALL_FSV_ROOT",
        "calyx-periodic-recall-fsv",
    );
    let before = json!({
        "root_exists_before_reset": root.exists(),
        "files_before_reset": list_files(&root),
    });
    reset_dir(&root);

    let happy = happy_path(&root);
    let single = single_occurrence_edge(&root);
    let invalid = invalid_hour_edge(&root);
    let readback = json!({
        "before": before,
        "happy": happy,
        "single_occurrence": single,
        "invalid_hour": invalid,
        "after": {"files": list_files(&root)},
    });
    write_json(&root.join("periodic-recall-readback.json"), &readback);
    write_blake3_sums(&root);

    assert_eq!(
        readback["happy"]["series"]["periodic_fit"]["target_hour"],
        json!(14)
    );
    assert_eq!(
        readback["happy"]["series"]["periodic_fit"]["target_day_of_week"],
        json!(1)
    );
    assert_eq!(
        readback["happy"]["series"]["periodic_fit"]["dominant_period_secs"],
        json!(WEEK_SECS as f64)
    );
    assert_eq!(
        readback["happy"]["recall"]["hits"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        readback["happy"]["recall"]["hits"][0]["cx_id"],
        readback["happy"]["tuesday_id"]
    );
    assert_eq!(
        readback["happy"]["no_match"]["hits"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert!(
        readback["happy"]["no_filter"]["stderr"]
            .as_str()
            .unwrap()
            .contains("CALYX_TEMPORAL_INVALID_PERIOD")
    );
    assert_eq!(
        readback["single_occurrence"]["recall"]["hits"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        readback["single_occurrence"]["series"]["periodic_fit"]["target_hour"],
        Value::Null
    );
    assert_eq!(
        readback["single_occurrence"]["series"]["periodic_fit"]["target_day_of_week"],
        Value::Null
    );
    assert!(
        readback["invalid_hour"]["stderr"]
            .as_str()
            .unwrap()
            .contains("CALYX_TEMPORAL_INVALID_PERIOD")
    );

    println!("periodic_recall_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

fn happy_path(root: &Path) -> Value {
    let vault_dir = root.join("happy").join("vault");
    let vault = durable_vault(&vault_dir);
    let tuesday = put_base(&vault, b"periodic-tuesday-14");
    let wednesday = put_base(&vault, b"periodic-wednesday-09");
    let store = SeriesStore::new(&vault);
    for week in 0..6 {
        store
            .append_occurrence(
                tuesday,
                EpochSecs(TUESDAY_2024_01_02_14H_UTC + week * WEEK_SECS),
                ctx("tue"),
            )
            .expect("append tuesday");
        store
            .append_occurrence(
                wednesday,
                EpochSecs(TUESDAY_2024_01_02_14H_UTC + 19 * 3_600 + week * WEEK_SECS),
                ctx("wed"),
            )
            .expect("append wednesday");
    }
    vault.flush().expect("flush happy");
    json!({
        "vault_dir": display(&vault_dir),
        "tuesday_id": tuesday.to_string(),
        "wednesday_id": wednesday.to_string(),
        "series": command_json(&["readback", "recurrence-series", "--vault", &display(&vault_dir), "--cx-id", &tuesday.to_string()]),
        "recall": command_json(&["readback", "periodic-recall", "--vault", &display(&vault_dir), "--hour", "14", "--day", "1"]),
        "no_match": command_json(&["readback", "periodic-recall", "--vault", &display(&vault_dir), "--hour", "9", "--day", "1"]),
        "no_filter": command_error(&["readback", "periodic-recall", "--vault", &display(&vault_dir)]),
        "raw_recurrence": command_stdout(&["readback", "--cf", "recurrence", "--vault", &display(&vault_dir)]),
        "raw_base": command_stdout(&["readback", "--cf", "base", "--vault", &display(&vault_dir)]),
        "raw_wal": command_stdout(&["readback", "--wal", "--vault", &display(&vault_dir)]),
    })
}

fn single_occurrence_edge(root: &Path) -> Value {
    let vault_dir = root.join("single").join("vault");
    let vault = durable_vault(&vault_dir);
    let cx_id = put_base(&vault, b"periodic-single");
    SeriesStore::new(&vault)
        .append_occurrence(cx_id, EpochSecs(TUESDAY_2024_01_02_14H_UTC), ctx("one"))
        .expect("append single");
    vault.flush().expect("flush single");
    json!({
        "vault_dir": display(&vault_dir),
        "cx_id": cx_id.to_string(),
        "series": command_json(&["readback", "recurrence-series", "--vault", &display(&vault_dir), "--cx-id", &cx_id.to_string()]),
        "recall": command_json(&["readback", "periodic-recall", "--vault", &display(&vault_dir), "--hour", "14", "--day", "1"]),
        "raw_recurrence": command_stdout(&["readback", "--cf", "recurrence", "--vault", &display(&vault_dir)]),
    })
}

fn invalid_hour_edge(root: &Path) -> Value {
    let vault_dir = root.join("invalid").join("vault");
    let vault = durable_vault(&vault_dir);
    let cx_id = put_base(&vault, b"periodic-invalid");
    SeriesStore::new(&vault)
        .append_occurrence(cx_id, EpochSecs(TUESDAY_2024_01_02_14H_UTC), ctx("invalid"))
        .expect("append invalid");
    vault.flush().expect("flush invalid");
    let output = command(&[
        "readback",
        "periodic-recall",
        "--vault",
        &display(&vault_dir),
        "--hour",
        "24",
    ]);
    json!({
        "vault_dir": display(&vault_dir),
        "status_success": output.status.success(),
        "stderr": String::from_utf8(output.stderr).expect("stderr utf8"),
        "raw_recurrence": command_stdout(&["readback", "--cf", "recurrence", "--vault", &display(&vault_dir)]),
    })
}

fn durable_vault(dir: &Path) -> AsterVault {
    AsterVault::new_durable(dir, vault_id(), SALT.to_vec(), VaultOptions::default())
        .expect("open durable periodic vault")
}

fn put_base(vault: &AsterVault, input: &[u8]) -> calyx_core::CxId {
    let cx_id = vault.cx_id_for_input(input, 41);
    vault
        .put(Constellation {
            cx_id,
            vault_id: vault_id(),
            panel_version: 41,
            created_at: 100,
            input_ref: InputRef {
                hash: *blake3::hash(input).as_bytes(),
                pointer: None,
                redacted: true,
            },
            modality: Modality::Text,
            slots: BTreeMap::new(),
            scalars: BTreeMap::new(),
            metadata: BTreeMap::new(),
            anchors: Vec::new(),
            provenance: LedgerRef {
                seq: 0,
                hash: [0; 32],
            },
            flags: CxFlags {
                ungrounded: true,
                redacted_input: true,
                ..CxFlags::default()
            },
        })
        .expect("put periodic base");
    cx_id
}

fn ctx(value: &str) -> OccurrenceContext {
    OccurrenceContext::new(value.as_bytes().to_vec()).expect("context")
}

fn command_json(args: &[&str]) -> Value {
    let stdout = command_stdout(args);
    serde_json::from_str(&stdout).expect("parse command json")
}

fn command_stdout(args: &[&str]) -> String {
    let output = command(args);
    assert!(
        output.status.success(),
        "command failed: {}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout utf8")
}

fn command_error(args: &[&str]) -> Value {
    let output = command(args);
    assert!(
        !output.status.success(),
        "command unexpectedly passed: {}",
        output.status
    );
    json!({
        "status_success": output.status.success(),
        "stderr": String::from_utf8(output.stderr).expect("stderr utf8"),
    })
}

fn command(args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_calyx"));
    command.args(args);
    command.output().expect("run calyx")
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

fn vault_id() -> VaultId {
    VAULT_ID.parse().expect("vault id")
}
