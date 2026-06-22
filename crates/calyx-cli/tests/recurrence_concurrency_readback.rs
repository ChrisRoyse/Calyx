use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::{Arc, Barrier};
use std::thread;

use calyx_aster::dedup::{
    DedupAction, DedupPolicy, DedupResult, EpochSecs, IngestInput, OccurrenceId, TauStrategy,
    TctCosineConfig, ingest_at,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId, VaultStore,
};
use calyx_loom::recurrence::{OccurrenceContext, SeriesStore};
use calyx_testkit::fsv::{
    fsv_root as test_fsv_root, list_files, reset_dir, write_blake3_sums, write_json,
};
use serde_json::{Value, json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const SALT: &[u8] = b"recurrence-concurrency-readback-salt";
const DIRECT_WORKERS: usize = 16;
const INGEST_WORKERS: usize = 12;

#[test]
fn recurrence_concurrency_readback_proves_unique_ids_and_fail_closed_retry() {
    let (root, keep_root) = test_fsv_root(
        "CALYX_RECURRENCE_CONCURRENCY_FSV_ROOT",
        "calyx-recurrence-concurrency-fsv",
    );
    let before = json!({
        "root_exists_before_reset": root.exists(),
        "files_before_reset": list_files(&root),
    });
    reset_dir(&root);

    let direct = direct_append_scenario(&root);
    let ingest = ingest_scenario(&root);
    let failed_retry = failed_retry_scenario(&root);
    let readback = json!({
        "before": before,
        "direct_append": direct,
        "ingest": ingest,
        "failed_retry": failed_retry,
        "after": {"files": list_files(&root)},
    });
    write_json(
        &root.join("recurrence-concurrency-readback.json"),
        &readback,
    );
    write_blake3_sums(&root);

    assert_ids(
        &readback["direct_append"]["returned_ids"],
        0,
        DIRECT_WORKERS as u64,
    );
    assert_ids(
        &readback["direct_append"]["stored_ids"],
        0,
        DIRECT_WORKERS as u64,
    );
    assert_eq!(
        readback["direct_append"]["after"]["series"]["frequency"],
        json!(DIRECT_WORKERS)
    );
    assert_ids(&readback["ingest"]["merge_ids"], 1, INGEST_WORKERS as u64);
    assert_ids(
        &readback["ingest"]["stored_ids"],
        0,
        INGEST_WORKERS as u64 + 1,
    );
    assert_eq!(
        readback["ingest"]["after"]["series"]["frequency"],
        json!(INGEST_WORKERS + 1)
    );
    assert_eq!(
        readback["failed_retry"]["error_code"],
        json!("CALYX_DEDUP_INVALID_EVENT_TIME")
    );
    assert_eq!(
        readback["failed_retry"]["after_failure"]["raw_recurrence"],
        readback["failed_retry"]["before_failure"]["raw_recurrence"]
    );
    assert_eq!(readback["failed_retry"]["retry_id"], json!(0));
    assert_eq!(
        readback["failed_retry"]["after_retry"]["series"]["frequency"],
        json!(1)
    );

    println!("recurrence_concurrency_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

fn direct_append_scenario(root: &Path) -> Value {
    let vault_dir = root.join("direct-append").join("vault");
    let vault = durable_vault(&vault_dir, DedupPolicy::Off);
    let cx_id = put_base(&vault, b"recurrence-direct-race");
    vault.flush().expect("flush direct base");
    let before = snapshot(&vault_dir, cx_id, "0..1");
    let barrier = Arc::new(Barrier::new(DIRECT_WORKERS));
    let handles = (0..DIRECT_WORKERS)
        .map(|index| {
            let vault_dir = vault_dir.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let vault = durable_vault(&vault_dir, DedupPolicy::Off);
                barrier.wait();
                let occurrence = SeriesStore::new(&vault).append_occurrence(
                    cx_id,
                    EpochSecs(1_000 + index as i64),
                    ctx(&format!("direct-{index}")),
                )?;
                vault.flush()?;
                Ok(occurrence)
            })
        })
        .collect::<Vec<_>>();

    let returned_ids = join_occurrence_ids(handles);
    vault.flush().expect("flush direct append");
    let after = snapshot(&vault_dir, cx_id, "0..1");
    json!({
        "vault_dir": display(&vault_dir),
        "cx_id": cx_id.to_string(),
        "worker_handle_model": "one durable AsterVault per worker, opened before the barrier",
        "before": before,
        "returned_ids": returned_ids,
        "stored_ids": occurrence_ids(&after["series"]),
        "stored_times": occurrence_times(&after["series"]),
        "after": after,
    })
}

fn ingest_scenario(root: &Path) -> Value {
    let vault_dir = root.join("ingest").join("vault");
    let vault = durable_vault(&vault_dir, tct_policy());
    let first = ingest_at(
        &vault,
        &input("recurrence-ingest-race", [1.0, 0.0]),
        EpochSecs(100),
        None,
    )
    .expect("first ingest");
    let DedupResult::New(cx_id) = first else {
        panic!("expected first ingest to create series");
    };
    vault.flush().expect("flush first ingest");
    let before = snapshot(&vault_dir, cx_id, "0..1");
    let barrier = Arc::new(Barrier::new(INGEST_WORKERS));
    let handles = (0..INGEST_WORKERS)
        .map(|index| {
            let vault_dir = vault_dir.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let vault = durable_vault(&vault_dir, tct_policy());
                barrier.wait();
                let result = ingest_at(
                    &vault,
                    &input("recurrence-ingest-race", [1.0, 0.0]),
                    EpochSecs(200 + index as i64),
                    None,
                )?;
                vault.flush()?;
                Ok(result)
            })
        })
        .collect::<Vec<_>>();

    let merge_ids = join_merge_ids(handles, cx_id);
    vault.flush().expect("flush concurrent ingest");
    let after = snapshot(&vault_dir, cx_id, "0..13");
    json!({
        "vault_dir": display(&vault_dir),
        "cx_id": cx_id.to_string(),
        "worker_handle_model": "one durable AsterVault per worker, opened before the barrier",
        "before": before,
        "merge_ids": merge_ids,
        "stored_ids": occurrence_ids(&after["series"]),
        "stored_times": occurrence_times(&after["series"]),
        "after": after,
    })
}

fn failed_retry_scenario(root: &Path) -> Value {
    let vault_dir = root.join("failed-retry").join("vault");
    let vault = durable_vault(&vault_dir, DedupPolicy::Off);
    let cx_id = put_base(&vault, b"recurrence-failed-retry");
    vault.flush().expect("flush failed retry base");
    let before_failure = snapshot(&vault_dir, cx_id, "0..1");
    let store = SeriesStore::new(&vault);
    let error = store
        .append_occurrence(cx_id, EpochSecs(-1), ctx("bad-time"))
        .expect_err("negative event time rejected");
    vault.flush().expect("flush after rejected append");
    let after_failure = snapshot(&vault_dir, cx_id, "0..1");
    let retry_id = store
        .append_occurrence(cx_id, EpochSecs(333), ctx("retry"))
        .expect("retry append");
    vault.flush().expect("flush retry");
    let after_retry = snapshot(&vault_dir, cx_id, "0..1");
    json!({
        "vault_dir": display(&vault_dir),
        "cx_id": cx_id.to_string(),
        "before_failure": before_failure,
        "error_code": error.code,
        "after_failure": after_failure,
        "retry_id": retry_id.0,
        "after_retry": after_retry,
    })
}

fn snapshot(vault_dir: &Path, cx_id: calyx_core::CxId, range: &str) -> Value {
    json!({
        "series": command_json(&[
            "readback", "recurrence-series", "--vault", &display(vault_dir), "--cx-id",
            &cx_id.to_string()
        ]),
        "raw_recurrence": command_stdout(&["readback", "--cf", "recurrence", "--vault", &display(vault_dir)]),
        "raw_base": command_stdout(&["readback", "--cf", "base", "--vault", &display(vault_dir)]),
        "raw_wal": command_stdout(&["readback", "--wal", "--vault", &display(vault_dir)]),
        "verify_chain": command_stdout(&["verify-chain", "--vault", &display(vault_dir), "--range", range]),
    })
}

fn join_occurrence_ids(
    handles: Vec<thread::JoinHandle<calyx_core::Result<OccurrenceId>>>,
) -> Vec<u64> {
    let mut ids = handles
        .into_iter()
        .map(|handle| handle.join().expect("thread").expect("append").0)
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

fn join_merge_ids(
    handles: Vec<thread::JoinHandle<calyx_core::Result<DedupResult>>>,
    cx_id: calyx_core::CxId,
) -> Vec<u64> {
    let mut ids = Vec::new();
    for handle in handles {
        match handle.join().expect("thread").expect("ingest") {
            DedupResult::DedupMerge { into, occurrence } if into == cx_id => ids.push(occurrence.0),
            result => panic!("expected merge into {cx_id}: {result:?}"),
        }
    }
    ids.sort();
    ids
}

fn occurrence_ids(series: &Value) -> Vec<u64> {
    let mut ids = series["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value["id"].as_u64().unwrap())
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

fn occurrence_times(series: &Value) -> Vec<i64> {
    let mut times = series["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value["t_k"].as_i64().unwrap())
        .collect::<Vec<_>>();
    times.sort();
    times
}

fn assert_ids(value: &Value, start: u64, count: u64) {
    let actual = value.as_array().expect("array");
    assert_eq!(actual.len() as u64, count);
    for expected in start..start + count {
        assert!(actual.contains(&json!(expected)), "missing id {expected}");
    }
}

fn durable_vault(dir: &Path, dedup_policy: DedupPolicy) -> AsterVault {
    let options = VaultOptions {
        dedup_policy: Some(dedup_policy),
        ..VaultOptions::default()
    };
    AsterVault::new_durable(dir, vault_id(), SALT.to_vec(), options)
        .expect("open durable recurrence concurrency vault")
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
        .expect("put base");
    cx_id
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

fn tct_policy() -> DedupPolicy {
    DedupPolicy::TctCosine(
        TctCosineConfig::new(
            vec![SlotId::new(0)],
            TauStrategy::PerSlot(vec![(SlotId::new(0), 0.90)]),
            DedupAction::RecurrenceSeries,
        )
        .expect("policy"),
    )
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
