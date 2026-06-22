use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Output};

use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::dedup::{
    DedupAction, DedupOnlineKind, DedupPolicy, DedupResult, EpochSecs, IngestInput, OccurrenceId,
    TauStrategy, TctCosineConfig, contested_with_key, decode_contested_with,
    decode_dedup_online_event, dedup_online_key, ingest_at,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxId, Modality, SlotId, SlotVector, VaultId, VaultStore,
};
use calyx_ledger::decode as decode_ledger;
use serde_json::{Value, json};

#[path = "dedup_fsv_io.rs"]
mod dedup_fsv_io;

pub(crate) use dedup_fsv_io::{
    fsv_root, list_dir_files as list_files, reset_dir, write_blake3_sums, write_json,
};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const SALT: &str = "dedup-ingest-at-readback-salt";

pub(crate) fn recurrence_scenario(root: &Path) -> Value {
    let vault_dir = root.join("recurrence").join("vault");
    let vault = durable_vault(&vault_dir, tct_policy(DedupAction::RecurrenceSeries));
    let results = [
        ingest_at(
            &vault,
            &temporal_input("same-event", [1.0, 0.0], [1.0, 0.0]),
            EpochSecs(100),
            None,
        )
        .expect("ingest 100"),
        ingest_at(
            &vault,
            &temporal_input("same-event", [1.0, 0.0], [0.0, 1.0]),
            EpochSecs(200),
            None,
        )
        .expect("ingest 200"),
        ingest_at(
            &vault,
            &temporal_input("same-event", [1.0, 0.0], [-1.0, 0.0]),
            EpochSecs(300),
            None,
        )
        .expect("ingest 300"),
    ];
    vault.flush().expect("flush recurrence");
    let id = result_new_id(&results[0]);
    let occurrence_times = (0..=2)
        .map(|occ| occurrence_at(&vault, id, occ))
        .collect::<Vec<_>>();
    scenario_json(
        &vault,
        &vault_dir,
        json!(results),
        json!({
            "cx_id": id,
            "occurrence_times": occurrence_times,
            "occurrences": occurrence_values(&vault, id, 0..=2),
            "ledger_payloads": ledger_payloads(&vault, 0..=2),
        }),
    )
}

pub(crate) fn same_temporal_signature_scenario(root: &Path) -> Value {
    let vault_dir = root.join("same_temporal_signature").join("vault");
    let vault = durable_vault(&vault_dir, tct_policy(DedupAction::RecurrenceSeries));
    let first = temporal_input("same-time-signature", [1.0, 0.0], [1.0, 0.0]);
    let second = temporal_input("same-time-signature", [1.0, 0.0], [1.0, 0.0]);
    let first_result = ingest_at(&vault, &first, EpochSecs(100), None).expect("first same-time");
    let second_result = ingest_at(&vault, &second, EpochSecs(200), None).expect("second same-time");
    vault.flush().expect("flush same-time signature");
    let id = result_new_id(&first_result);
    scenario_json(
        &vault,
        &vault_dir,
        json!([first_result]),
        json!({
            "cx_id": id,
            "second_result": second_result,
            "ledger_payloads": ledger_payloads(&vault, 0..=1),
        }),
    )
}

pub(crate) fn event_time_fallback_signature_scenario(root: &Path) -> Value {
    let vault_dir = root.join("event_time_fallback_signature").join("vault");
    let vault = durable_vault(&vault_dir, tct_policy(DedupAction::RecurrenceSeries));
    let input = input("fallback-signature", [1.0, 0.0]);
    let first = ingest_at(&vault, &input, EpochSecs(100), None).expect("first fallback");
    let second = ingest_at(&vault, &input, EpochSecs(200), None).expect("second fallback");
    vault.flush().expect("flush fallback signature");
    let id = result_new_id(&first);
    scenario_json(
        &vault,
        &vault_dir,
        json!([first, second]),
        json!({
            "cx_id": id,
            "occurrence_times": [occurrence_at(&vault, id, 0), occurrence_at(&vault, id, 1)],
            "occurrences": occurrence_values(&vault, id, 0..=1),
            "ledger_payloads": ledger_payloads(&vault, 0..=1),
        }),
    )
}

pub(crate) fn missing_temporal_signature_scenario(root: &Path) -> Value {
    let vault_dir = root.join("missing_temporal_signature").join("vault");
    let vault = durable_vault(&vault_dir, tct_policy(DedupAction::RecurrenceSeries));
    let first = temporal_input("missing-signature", [1.0, 0.0], [1.0, 0.0]);
    let second = input("missing-signature", [1.0, 0.0]).with_temporal_slot(temporal_slot());
    let first_result = ingest_at(&vault, &first, EpochSecs(100), None).expect("first missing");
    let error = ingest_at(&vault, &second, EpochSecs(200), None)
        .expect_err("missing temporal signature slot");
    vault.flush().expect("flush missing temporal signature");
    let id = result_new_id(&first_result);
    scenario_json(
        &vault,
        &vault_dir,
        json!([first_result]),
        json!({
            "cx_id": id,
            "error_code": error.code,
            "ledger_payloads": ledger_payloads(&vault, 0..=0),
        }),
    )
}

pub(crate) fn exact_duplicate_scenario(root: &Path) -> Value {
    let vault_dir = root.join("exact").join("vault");
    let vault = durable_vault(&vault_dir, DedupPolicy::Exact);
    let input = input("exact-event", [1.0, 0.0]);
    let first = ingest_at(&vault, &input, EpochSecs(700), None).expect("first exact");
    let second = ingest_at(&vault, &input, EpochSecs(700), None).expect("second exact");
    vault.flush().expect("flush exact");
    let id = result_new_id(&first);
    scenario_json(
        &vault,
        &vault_dir,
        json!([first]),
        json!({
            "cx_id": id,
            "second_result": second,
            "ledger_payloads": ledger_payloads(&vault, 0..=1),
        }),
    )
}

pub(crate) fn anchor_conflict_scenario(root: &Path) -> Value {
    let vault_dir = root.join("anchor_conflict").join("vault");
    let vault = durable_vault(&vault_dir, tct_policy(DedupAction::RecurrenceSeries));
    let first = input("speaker-a", [1.0, 0.0]).with_anchor(speaker("alice"));
    let second = input("speaker-b", [1.0, 0.0]).with_anchor(speaker("bob"));
    let first_result = ingest_at(&vault, &first, EpochSecs(1000), None).expect("first speaker");
    let second_result = ingest_at(&vault, &second, EpochSecs(2000), None).expect("second speaker");
    vault.flush().expect("flush conflict");
    let first_id = result_new_id(&first_result);
    let second_id = result_new_id(&second_result);
    scenario_json(
        &vault,
        &vault_dir,
        json!([first_result]),
        json!({
            "second_result": second_result,
            "first_contested": contested(&vault, first_id),
            "second_contested": contested(&vault, second_id),
            "ledger_payloads": ledger_payloads(&vault, 0..=1),
        }),
    )
}

pub(crate) fn event_time_edges_scenario(root: &Path) -> Value {
    let vault_dir = root.join("event_time_edges").join("vault");
    let vault = durable_vault(&vault_dir, DedupPolicy::Off);
    let early = input("epoch-zero", [1.0, 0.0]);
    let future = input("future-event", [0.0, 1.0]);
    let early_result = ingest_at(&vault, &early, EpochSecs(0), None).expect("epoch zero");
    let future_result = ingest_at(&vault, &future, EpochSecs(4_102_444_800), None).expect("future");
    vault.flush().expect("flush edges");
    let ids = [result_new_id(&early_result), result_new_id(&future_result)];
    let stored_times = ids
        .iter()
        .map(|id| vault.get(*id, vault.snapshot()).unwrap().created_at as i64)
        .collect::<Vec<_>>();
    scenario_json(
        &vault,
        &vault_dir,
        json!([early_result, future_result]),
        json!({
            "stored_times": stored_times,
            "ledger_payloads": ledger_payloads(&vault, 0..=1),
        }),
    )
}

pub(crate) fn negative_time_scenario(root: &Path) -> Value {
    let vault_dir = root.join("negative").join("vault");
    let vault = durable_vault(&vault_dir, DedupPolicy::Off);
    let error = ingest_at(&vault, &input("negative", [1.0, 0.0]), EpochSecs(-1), None)
        .expect_err("negative rejected");
    vault.flush().expect("flush negative");
    json!({
        "error_code": error.code,
        "base_stdout": stdout(&readback_cf(&vault_dir, "base")),
        "ledger_stdout": stdout(&readback_cf(&vault_dir, "ledger")),
    })
}

fn scenario_json(vault: &AsterVault, vault_dir: &Path, results: Value, extra: Value) -> Value {
    let recurrence = readback_cf(vault_dir, "recurrence");
    let mut value = json!({
        "results": results,
        "base_row_count": row_count(&readback_cf(vault_dir, "base")),
        "online_row_count": row_count(&readback_cf(vault_dir, "online")),
        "recurrence_row_count": row_count(&recurrence),
        "ledger_row_count": row_count(&readback_cf(vault_dir, "ledger")),
        "base_stdout": stdout(&readback_cf(vault_dir, "base")),
        "online_stdout": stdout(&readback_cf(vault_dir, "online")),
        "recurrence_stdout": stdout(&recurrence),
        "ledger_stdout": stdout(&readback_cf(vault_dir, "ledger")),
        "snapshot": vault.snapshot(),
    });
    merge_object(&mut value, extra);
    value
}

fn occurrence_values(
    vault: &AsterVault,
    id: CxId,
    range: std::ops::RangeInclusive<u64>,
) -> Vec<Value> {
    range
        .map(|occ| {
            let key = dedup_online_key(DedupOnlineKind::Occurrence, id, OccurrenceId(occ));
            let bytes = vault
                .read_cf_at(vault.snapshot(), ColumnFamily::Online, &key)
                .unwrap()
                .expect("occurrence row");
            serde_json::to_value(decode_dedup_online_event(&bytes).unwrap()).unwrap()
        })
        .collect()
}

fn occurrence_at(vault: &AsterVault, id: CxId, occ: u64) -> i64 {
    occurrence_values(vault, id, occ..=occ)[0]["at"]
        .as_i64()
        .expect("occurrence at")
}

fn ledger_payloads(vault: &AsterVault, range: std::ops::RangeInclusive<u64>) -> Vec<Value> {
    range
        .map(|seq| {
            let bytes = vault
                .read_cf_at(vault.snapshot(), ColumnFamily::Ledger, &ledger_key(seq))
                .unwrap()
                .expect("ledger row");
            let entry = decode_ledger(&bytes).expect("decode ledger");
            serde_json::from_slice(&entry.payload).expect("payload json")
        })
        .collect()
}

fn contested(vault: &AsterVault, id: CxId) -> Value {
    let bytes = vault
        .read_cf_at(
            vault.snapshot(),
            ColumnFamily::Online,
            &contested_with_key(id),
        )
        .unwrap()
        .expect("contested row");
    serde_json::to_value(decode_contested_with(&bytes).unwrap()).unwrap()
}

fn result_new_id(result: &DedupResult) -> CxId {
    match result {
        DedupResult::New(id) => *id,
        DedupResult::DedupMerge { .. } | DedupResult::ExactDuplicate(_) => {
            panic!("expected New result")
        }
    }
}

fn durable_vault(vault_dir: &Path, dedup_policy: DedupPolicy) -> AsterVault {
    AsterVault::new_durable(
        vault_dir,
        vault_id(),
        SALT.as_bytes().to_vec(),
        VaultOptions {
            dedup_policy: Some(dedup_policy),
            ..VaultOptions::default()
        },
    )
    .expect("open durable vault")
}

fn tct_policy(action: DedupAction) -> DedupPolicy {
    DedupPolicy::TctCosine(
        TctCosineConfig::new(
            vec![slot(0)],
            TauStrategy::PerSlot(vec![(slot(0), 0.90)]),
            action,
        )
        .expect("policy"),
    )
}

fn input(name: &str, dense_values: [f32; 2]) -> IngestInput {
    IngestInput::new(name.as_bytes().to_vec(), 41, Modality::Text).with_slot(
        slot(0),
        SlotVector::Dense {
            dim: 2,
            data: dense_values.to_vec(),
        },
    )
}

fn temporal_input(name: &str, dense_values: [f32; 2], temporal_values: [f32; 2]) -> IngestInput {
    input(name, dense_values)
        .with_slot(
            temporal_slot(),
            SlotVector::Dense {
                dim: 2,
                data: temporal_values.to_vec(),
            },
        )
        .with_temporal_slot(temporal_slot())
}

fn temporal_slot() -> SlotId {
    slot(20)
}

fn speaker(name: &str) -> Anchor {
    Anchor {
        kind: AnchorKind::SpeakerMatch,
        value: AnchorValue::Text(name.to_string()),
        source: "synthetic-ingest-at-readback".to_string(),
        observed_at: 1_786_406_600,
        confidence: 1.0,
    }
}

fn readback_cf(vault_dir: &Path, cf: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .arg("readback")
        .arg("--cf")
        .arg(cf)
        .arg("--vault")
        .arg(vault_dir)
        .output()
        .expect("run readback")
}

fn row_count(output: &Output) -> usize {
    stdout(output)
        .lines()
        .filter_map(readback_key)
        .collect::<BTreeSet<_>>()
        .len()
}

fn readback_key(line: &str) -> Option<String> {
    let mut parts = line.split('\t');
    while let Some(part) = parts.next() {
        if part == "KEY" {
            return parts.next().map(ToString::to_string);
        }
    }
    None
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn merge_object(target: &mut Value, extra: Value) {
    let target = target.as_object_mut().expect("target object");
    for (key, value) in extra.as_object().expect("extra object") {
        target.insert(key.clone(), value.clone());
    }
}

fn vault_id() -> VaultId {
    VAULT_ID.parse().expect("valid vault id")
}

fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}
