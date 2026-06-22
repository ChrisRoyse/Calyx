use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::{OccurrenceContext, RetentionPolicy, append_occurrence};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Constellation, CxFlags, InputRef, LedgerRef, Modality, VaultId, VaultStore};
use calyx_testkit::fsv::{
    fsv_root as test_fsv_root, list_files, reset_dir, write_blake3_sums, write_json,
};
use serde_json::{Value, json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const SALT: &[u8] = b"time-prediction-readback-salt";
const TUESDAY_2024_01_02_14H_UTC: i64 = 1_704_204_000;
const WEEK_SECS: i64 = 604_800;

#[test]
fn time_prediction_readback_reports_prediction_and_sparse_refusal() {
    let (root, keep_root) = test_fsv_root(
        "CALYX_TIME_PREDICTION_FSV_ROOT",
        "calyx-time-prediction-fsv",
    );
    let before = json!({
        "root_exists_before_reset": root.exists(),
        "files_before_reset": list_files(&root),
    });
    reset_dir(&root);

    let happy = weekly_prediction(&root);
    let sparse = sparse_refusal(&root);
    let base_only = base_only_refusal(&root);
    let duplicate = duplicate_refusal(&root);
    let invalid_ceiling = invalid_ceiling_refusal(&happy);
    let readback = json!({
        "before": before,
        "weekly": happy,
        "edges": {
            "base_only": base_only,
            "duplicate": duplicate,
            "invalid_ceiling": invalid_ceiling,
            "sparse": sparse,
        },
        "after": {"files": list_files(&root)},
    });
    write_json(&root.join("time-prediction-readback.json"), &readback);
    write_blake3_sums(&root);

    assert_eq!(
        readback["weekly"]["prediction"]["prediction"]["t_hat"],
        json!(TUESDAY_2024_01_02_14H_UTC + 12 * WEEK_SECS)
    );
    assert_eq!(readback["weekly"]["prediction"]["sufficient"], json!(true));
    assert_close(
        readback["weekly"]["prediction"]["prediction"]["confidence"]
            .as_f64()
            .expect("confidence"),
        0.91,
    );
    assert_eq!(
        readback["edges"]["sparse"]["prediction"]["sufficient"],
        json!(false)
    );
    assert_eq!(
        readback["edges"]["sparse"]["prediction"]["error"]["code"],
        json!("CALYX_ORACLE_INSUFFICIENT")
    );
    for edge in ["base_only", "duplicate", "invalid_ceiling"] {
        assert_eq!(
            readback["edges"][edge]["prediction"]["sufficient"],
            json!(false),
            "edge {edge} should fail closed"
        );
        assert_eq!(
            readback["edges"][edge]["prediction"]["error"]["code"],
            json!("CALYX_ORACLE_INSUFFICIENT"),
            "edge {edge} should report oracle insufficiency"
        );
    }

    println!("time_prediction_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

fn weekly_prediction(root: &Path) -> Value {
    let vault_dir = root.join("weekly").join("vault");
    let vault = durable_vault(&vault_dir);
    let cx_id = put_base(&vault, b"time-prediction-weekly");
    for week in 0..12 {
        append_occurrence(
            &vault,
            cx_id,
            EpochSecs(TUESDAY_2024_01_02_14H_UTC + week * WEEK_SECS),
            ctx("weekly"),
            EpochSecs(TUESDAY_2024_01_02_14H_UTC + week * WEEK_SECS),
            RetentionPolicy::default(),
        )
        .expect("append weekly");
    }
    vault.flush().expect("flush weekly");
    json!({
        "vault_dir": display(&vault_dir),
        "cx_id": cx_id.to_string(),
        "expected_t_hat": TUESDAY_2024_01_02_14H_UTC + 12 * WEEK_SECS,
        "prediction": command_json(&["readback", "time-prediction", "--vault", &display(&vault_dir), "--cx-id", &cx_id.to_string(), "--confidence-ceiling", "0.91"]),
        "series": command_json(&["readback", "recurrence-series", "--vault", &display(&vault_dir), "--cx-id", &cx_id.to_string()]),
        "raw_recurrence": command_stdout(&["readback", "--cf", "recurrence", "--vault", &display(&vault_dir)]),
        "raw_base": command_stdout(&["readback", "--cf", "base", "--vault", &display(&vault_dir)]),
        "raw_wal": command_stdout(&["readback", "--wal", "--vault", &display(&vault_dir)]),
    })
}

fn sparse_refusal(root: &Path) -> Value {
    let vault_dir = root.join("sparse").join("vault");
    let vault = durable_vault(&vault_dir);
    let cx_id = put_base(&vault, b"time-prediction-sparse");
    for offset in [0, WEEK_SECS] {
        append_occurrence(
            &vault,
            cx_id,
            EpochSecs(TUESDAY_2024_01_02_14H_UTC + offset),
            ctx("sparse"),
            EpochSecs(TUESDAY_2024_01_02_14H_UTC + offset),
            RetentionPolicy::default(),
        )
        .expect("append sparse");
    }
    vault.flush().expect("flush sparse");
    json!({
        "vault_dir": display(&vault_dir),
        "cx_id": cx_id.to_string(),
        "prediction": command_json(&["readback", "time-prediction", "--vault", &display(&vault_dir), "--cx-id", &cx_id.to_string(), "--confidence-ceiling", "0.91"]),
        "series": command_json(&["readback", "recurrence-series", "--vault", &display(&vault_dir), "--cx-id", &cx_id.to_string()]),
        "raw_recurrence": command_stdout(&["readback", "--cf", "recurrence", "--vault", &display(&vault_dir)]),
        "raw_base": command_stdout(&["readback", "--cf", "base", "--vault", &display(&vault_dir)]),
        "raw_wal": command_stdout(&["readback", "--wal", "--vault", &display(&vault_dir)]),
    })
}

fn base_only_refusal(root: &Path) -> Value {
    let vault_dir = root.join("base-only").join("vault");
    let vault = durable_vault(&vault_dir);
    let cx_id = put_base(&vault, b"time-prediction-base-only");
    vault.flush().expect("flush base-only");
    json!({
        "vault_dir": display(&vault_dir),
        "cx_id": cx_id.to_string(),
        "prediction": command_json(&["readback", "time-prediction", "--vault", &display(&vault_dir), "--cx-id", &cx_id.to_string(), "--confidence-ceiling", "0.91"]),
        "series": command_json(&["readback", "recurrence-series", "--vault", &display(&vault_dir), "--cx-id", &cx_id.to_string()]),
        "raw_recurrence": command_stdout(&["readback", "--cf", "recurrence", "--vault", &display(&vault_dir)]),
        "raw_base": command_stdout(&["readback", "--cf", "base", "--vault", &display(&vault_dir)]),
        "raw_wal": command_stdout(&["readback", "--wal", "--vault", &display(&vault_dir)]),
    })
}

fn duplicate_refusal(root: &Path) -> Value {
    let vault_dir = root.join("duplicate").join("vault");
    let vault = durable_vault(&vault_dir);
    let cx_id = put_base(&vault, b"time-prediction-duplicate");
    for time in [
        TUESDAY_2024_01_02_14H_UTC,
        TUESDAY_2024_01_02_14H_UTC,
        TUESDAY_2024_01_02_14H_UTC + WEEK_SECS,
    ] {
        append_occurrence(
            &vault,
            cx_id,
            EpochSecs(time),
            ctx("duplicate"),
            EpochSecs(time),
            RetentionPolicy::default(),
        )
        .expect("append duplicate");
    }
    vault.flush().expect("flush duplicate");
    json!({
        "vault_dir": display(&vault_dir),
        "cx_id": cx_id.to_string(),
        "prediction": command_json(&["readback", "time-prediction", "--vault", &display(&vault_dir), "--cx-id", &cx_id.to_string(), "--confidence-ceiling", "0.91"]),
        "series": command_json(&["readback", "recurrence-series", "--vault", &display(&vault_dir), "--cx-id", &cx_id.to_string()]),
        "raw_recurrence": command_stdout(&["readback", "--cf", "recurrence", "--vault", &display(&vault_dir)]),
        "raw_base": command_stdout(&["readback", "--cf", "base", "--vault", &display(&vault_dir)]),
        "raw_wal": command_stdout(&["readback", "--wal", "--vault", &display(&vault_dir)]),
    })
}

fn invalid_ceiling_refusal(happy: &Value) -> Value {
    let vault_dir = happy["vault_dir"].as_str().expect("weekly vault dir");
    let cx_id = happy["cx_id"].as_str().expect("weekly cx id");
    json!({
        "vault_dir": vault_dir,
        "cx_id": cx_id,
        "prediction": command_json(&["readback", "time-prediction", "--vault", vault_dir, "--cx-id", cx_id, "--confidence-ceiling", "1.1"]),
        "source": "weekly persisted vault with invalid confidence ceiling",
    })
}

fn durable_vault(dir: &Path) -> AsterVault {
    AsterVault::new_durable(dir, vault_id(), SALT.to_vec(), VaultOptions::default())
        .expect("open durable vault")
}

fn put_base(vault: &AsterVault, input: &[u8]) -> calyx_core::CxId {
    let cx_id = vault.cx_id_for_input(input, 42);
    vault
        .put(Constellation {
            cx_id,
            vault_id: vault_id(),
            panel_version: 42,
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
        .expect("put base");
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

fn command(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .args(args)
        .output()
        .expect("run calyx")
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1.0e-6,
        "actual={actual} expected={expected}"
    );
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

fn vault_id() -> VaultId {
    VAULT_ID.parse().expect("vault id")
}
