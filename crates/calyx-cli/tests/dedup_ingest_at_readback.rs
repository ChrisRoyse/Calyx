use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
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

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const SALT: &str = "dedup-ingest-at-readback-salt";

#[test]
fn dedup_ingest_at_writes_event_time_merge_and_ledger_bytes() {
    let (root, keep_root) = fsv_root();
    let before = json!({
        "root_exists_before_reset": root.exists(),
        "files_before_reset": list_files(&root),
    });
    reset_dir(&root);

    let recurrence = recurrence_scenario(&root);
    let exact = exact_duplicate_scenario(&root);
    let conflict = anchor_conflict_scenario(&root);
    let edges = event_time_edges_scenario(&root);
    let negative = negative_time_scenario(&root);

    let readback = json!({
        "before": before,
        "recurrence": recurrence,
        "exact_duplicate": exact,
        "anchor_conflict": conflict,
        "event_time_edges": edges,
        "negative_time": negative,
        "after": {
            "files": list_files(&root),
        }
    });
    write_json(&root.join("dedup-ingest-at-readback.json"), &readback);
    write_blake3_sums(&root);

    assert_eq!(readback["recurrence"]["base_row_count"], json!(1));
    assert_eq!(
        readback["recurrence"]["ledger_payloads"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        readback["recurrence"]["occurrence_times"],
        json!([100, 200, 300])
    );
    assert_eq!(
        readback["recurrence"]["results"][1]["DedupMerge"]["occurrence"],
        json!(1)
    );
    assert_eq!(
        readback["recurrence"]["ledger_payloads"][1]["recurrence_signature"],
        json!(true)
    );
    assert_eq!(readback["exact_duplicate"]["base_row_count"], json!(1));
    assert_eq!(
        readback["exact_duplicate"]["second_result"]["ExactDuplicate"],
        readback["exact_duplicate"]["cx_id"]
    );
    assert_eq!(readback["anchor_conflict"]["base_row_count"], json!(2));
    assert!(
        readback["anchor_conflict"]["second_result"]
            .get("New")
            .is_some()
    );
    assert_eq!(
        readback["event_time_edges"]["stored_times"],
        json!([0, 4_102_444_800_i64])
    );
    assert_eq!(
        readback["negative_time"]["error_code"],
        json!("CALYX_DEDUP_INVALID_EVENT_TIME")
    );
    assert_eq!(readback["negative_time"]["base_stdout"], json!(""));

    println!("dedup_ingest_at_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

fn recurrence_scenario(root: &Path) -> Value {
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

fn exact_duplicate_scenario(root: &Path) -> Value {
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

fn anchor_conflict_scenario(root: &Path) -> Value {
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

fn event_time_edges_scenario(root: &Path) -> Value {
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

fn negative_time_scenario(root: &Path) -> Value {
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
    let mut value = json!({
        "results": results,
        "base_row_count": row_count(&readback_cf(vault_dir, "base")),
        "online_row_count": row_count(&readback_cf(vault_dir, "online")),
        "ledger_row_count": row_count(&readback_cf(vault_dir, "ledger")),
        "base_stdout": stdout(&readback_cf(vault_dir, "base")),
        "online_stdout": stdout(&readback_cf(vault_dir, "online")),
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

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("json")).expect("write json");
}

fn write_blake3_sums(root: &Path) {
    let mut files = Vec::new();
    collect_files(root, root, &mut files);
    files.sort();
    let mut lines = String::new();
    for relative in files {
        if relative == Path::new("BLAKE3SUMS.txt") {
            continue;
        }
        let bytes = fs::read(root.join(&relative)).expect("read checksum file");
        lines.push_str(&format!(
            "{}  {}\n",
            blake3::hash(&bytes).to_hex(),
            relative.to_string_lossy().replace('\\', "/")
        ));
    }
    fs::write(root.join("BLAKE3SUMS.txt"), lines).expect("write checksum manifest");
}

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_files(root, &path, files);
        } else {
            files.push(path.strip_prefix(root).unwrap().to_path_buf());
        }
    }
}

fn list_files(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = entries
        .map(|entry| {
            entry
                .expect("entry")
                .path()
                .strip_prefix(dir)
                .expect("relative")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn fsv_root() -> (PathBuf, bool) {
    if let Ok(root) = std::env::var("CALYX_DEDUP_INGEST_AT_FSV_ROOT") {
        return (PathBuf::from(root), true);
    }
    (
        std::env::temp_dir().join(format!("calyx-dedup-ingest-at-fsv-{}", std::process::id())),
        false,
    )
}

fn reset_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).expect("create fsv root");
}

fn vault_id() -> VaultId {
    VAULT_ID.parse().expect("valid vault id")
}

fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}
