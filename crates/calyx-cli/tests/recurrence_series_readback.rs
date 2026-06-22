use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use calyx_aster::dedup::{
    DedupAction, DedupPolicy, DedupResult, EpochSecs, IngestInput, TauStrategy, TctCosineConfig,
    ingest_at,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId, VaultStore,
};
use calyx_loom::recurrence::{MAX_CONTEXT_BYTES, OccurrenceContext, RetentionPolicy, SeriesStore};
use calyx_testkit::fsv::{
    fsv_root as test_fsv_root, list_files, reset_dir, write_blake3_sums, write_json,
};
use serde_json::{Value, json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const SALT: &[u8] = b"recurrence-series-readback-salt";

#[test]
fn recurrence_series_readback_writes_rollup_edges_and_fail_closed_bytes() {
    let (root, keep_root) = test_fsv_root(
        "CALYX_RECURRENCE_SERIES_FSV_ROOT",
        "calyx-recurrence-series-fsv",
    );
    let before = json!({
        "root_exists_before_reset": root.exists(),
        "files_before_reset": list_files(&root),
    });
    reset_dir(&root);

    let ingest = ingest_scenario(&root);
    let rollup = rollup_scenario(&root);
    let empty = empty_scenario(&root);
    let oversized = oversized_context_scenario(&root);
    let readback = json!({
        "before": before,
        "ingest": ingest,
        "rollup": rollup,
        "empty": empty,
        "oversized_context": oversized,
        "after": {"files": list_files(&root)},
    });
    write_json(&root.join("recurrence-series-readback.json"), &readback);
    write_blake3_sums(&root);

    assert_eq!(readback["ingest"]["series"]["frequency"], json!(5));
    assert_eq!(
        readback["ingest"]["times"],
        json!([100, 200, 300, 400, 500])
    );
    assert_eq!(readback["ingest"]["series"]["cadence_secs"], json!(100.0));
    assert_eq!(readback["rollup"]["series"]["frequency"], json!(6));
    assert_eq!(readback["rollup"]["times"], json!([1, 2, 3, 4, 5]));
    assert_eq!(
        readback["rollup"]["series"]["rollup_summary"]["count_rolled"],
        json!(1)
    );
    assert_eq!(readback["empty"]["series"]["occurrence_count"], json!(0));
    assert_eq!(
        readback["oversized_context"]["error_code"],
        json!("CALYX_RECURRENCE_CONTEXT_TOO_LARGE")
    );
    assert_eq!(
        readback["oversized_context"]["raw_recurrence_stdout"],
        json!("")
    );

    println!("recurrence_series_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

fn ingest_scenario(root: &Path) -> Value {
    let vault_dir = root.join("ingest").join("vault");
    let vault = durable_vault_with_policy(&vault_dir, tct_policy());
    let mut first_id = None;
    let mut results = Vec::new();
    for (index, time) in [100, 200, 300, 400, 500].into_iter().enumerate() {
        let input = temporal_input("recurrence-ingest", [1.0, 0.0], temporal_vector(index));
        let result = ingest_at(&vault, &input, EpochSecs(time), None).expect("ingest recurrence");
        if let DedupResult::New(cx_id) = &result {
            first_id = Some(*cx_id);
        }
        results.push(json!(result));
    }
    vault.flush().expect("flush ingest recurrence");
    let mut value = scenario_json(&vault_dir, first_id.expect("first recurrence id"));
    value["dedup_results"] = json!(results);
    value
}

fn rollup_scenario(root: &Path) -> Value {
    let vault_dir = root.join("rollup").join("vault");
    let vault = durable_vault(&vault_dir);
    let cx_id = put_base(&vault, b"recurrence-rollup", 41);
    let policy = RetentionPolicy::new(5, u64::MAX).expect("retention");
    let store = SeriesStore::with_retention(&vault, policy).expect("store");
    for time in 0..=5 {
        store
            .append_occurrence(cx_id, EpochSecs(time), ctx("r"))
            .expect("append rollup occurrence");
    }
    vault.flush().expect("flush rollup");
    scenario_json(&vault_dir, cx_id)
}

fn empty_scenario(root: &Path) -> Value {
    let vault_dir = root.join("empty").join("vault");
    let vault = durable_vault(&vault_dir);
    let cx_id = put_base(&vault, b"recurrence-empty", 41);
    vault.flush().expect("flush empty");
    scenario_json(&vault_dir, cx_id)
}

fn oversized_context_scenario(root: &Path) -> Value {
    let vault_dir = root.join("oversized").join("vault");
    let vault = durable_vault(&vault_dir);
    let cx_id = put_base(&vault, b"recurrence-oversized", 41);
    let store = SeriesStore::new(&vault);
    let error = OccurrenceContext::new(vec![7; MAX_CONTEXT_BYTES + 1])
        .and_then(|context| store.append_occurrence(cx_id, EpochSecs(1), context))
        .expect_err("oversized context rejected");
    vault.flush().expect("flush oversized");
    let mut value = scenario_json(&vault_dir, cx_id);
    value["error_code"] = json!(error.code);
    value
}

fn scenario_json(vault_dir: &Path, cx_id: calyx_core::CxId) -> Value {
    let series = command_json([
        "readback",
        "recurrence-series",
        "--vault",
        &vault_dir.display().to_string(),
        "--cx-id",
        &cx_id.to_string(),
    ]);
    let raw_recurrence = command_stdout([
        "readback",
        "--cf",
        "recurrence",
        "--vault",
        &vault_dir.display().to_string(),
    ]);
    let raw_base = command_stdout([
        "readback",
        "--cf",
        "base",
        "--vault",
        &vault_dir.display().to_string(),
    ]);
    let raw_wal = command_stdout([
        "readback",
        "--wal",
        "--vault",
        &vault_dir.display().to_string(),
    ]);
    let times = series["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value["t_k"].as_i64().unwrap())
        .collect::<Vec<_>>();
    json!({
        "vault_dir": vault_dir.display().to_string(),
        "cx_id": cx_id.to_string(),
        "series": series,
        "times": times,
        "raw_recurrence_stdout": raw_recurrence,
        "raw_base_stdout": raw_base,
        "raw_wal_stdout": raw_wal,
    })
}

fn durable_vault(dir: &Path) -> AsterVault {
    durable_vault_with_policy(dir, DedupPolicy::Off)
}

fn durable_vault_with_policy(dir: &Path, dedup_policy: DedupPolicy) -> AsterVault {
    let options = VaultOptions {
        dedup_policy: Some(dedup_policy),
        ..VaultOptions::default()
    };
    AsterVault::new_durable(dir, vault_id(), SALT.to_vec(), options)
        .expect("open durable recurrence vault")
}

fn put_base(vault: &AsterVault, input: &[u8], panel_version: u32) -> calyx_core::CxId {
    let cx_id = vault.cx_id_for_input(input, panel_version);
    vault
        .put(Constellation {
            cx_id,
            vault_id: vault_id(),
            panel_version,
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
        .expect("put recurrence base");
    cx_id
}

fn ctx(value: &str) -> OccurrenceContext {
    OccurrenceContext::new(value.as_bytes().to_vec()).expect("context")
}

fn input(raw: &str, vector: [f32; 2]) -> IngestInput {
    IngestInput::new(raw.as_bytes().to_vec(), 41, Modality::Text).with_slot(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vector.to_vec(),
        },
    )
}

fn temporal_input(raw: &str, vector: [f32; 2], temporal: [f32; 2]) -> IngestInput {
    input(raw, vector)
        .with_slot(
            temporal_slot(),
            SlotVector::Dense {
                dim: 2,
                data: temporal.to_vec(),
            },
        )
        .with_temporal_slot(temporal_slot())
}

fn temporal_vector(index: usize) -> [f32; 2] {
    match index {
        0 => [1.0, 0.0],
        1 => [0.0, 1.0],
        2 => [-1.0, 0.0],
        3 => [0.0, -1.0],
        _ => [0.70710677, 0.70710677],
    }
}

fn temporal_slot() -> SlotId {
    SlotId::new(20)
}

fn tct_policy() -> DedupPolicy {
    DedupPolicy::TctCosine(
        TctCosineConfig::new(
            vec![SlotId::new(0)],
            TauStrategy::PerSlot(vec![(SlotId::new(0), 0.99)]),
            DedupAction::RecurrenceSeries,
        )
        .expect("valid recurrence tct policy"),
    )
}

fn command_json<const N: usize>(args: [&str; N]) -> Value {
    let stdout = command_stdout(args);
    serde_json::from_str(&stdout).expect("parse command json")
}

fn command_stdout<const N: usize>(args: [&str; N]) -> String {
    let output = command(args);
    assert!(
        output.status.success(),
        "command failed: {}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

fn command<const N: usize>(args: [&str; N]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_calyx"));
    command.args(args);
    command.output().expect("run calyx")
}

fn vault_id() -> VaultId {
    VAULT_ID.parse().expect("vault id")
}
