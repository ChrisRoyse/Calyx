use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Output};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::dedup::{
    DedupAction, DedupPolicy, DedupRestoreSnapshot, DedupResult, EpochSecs, IngestInput,
    TauStrategy, TctCosineConfig, ingest_at,
};
use calyx_aster::recurrence::FREQUENCY_SCALAR;
use calyx_aster::vault::encode::{decode_write_batch, encode_constellation_base};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::wal::replay_dir;
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxId, GuardTauProfile, Modality, SlotId, SlotVector, VaultId,
    VaultStore,
};
use calyx_ledger::decode as decode_ledger;
use calyx_loom::recurrence::SeriesStore;
use serde_json::{Value, json};

#[path = "dedup_invariants_io.rs"]
mod io;

pub(crate) use io::{fsv_root, list_files, reset_dir, write_blake3_sums, write_json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const SALT: &str = "dedup-invariants-readback-salt";

pub(crate) fn near_distinct_scenario(root: &Path) -> Value {
    let vault_dir = root.join("near_distinct").join("vault");
    let vault = durable_vault(&vault_dir, calibrated_policy(DedupAction::Collapse));
    let guard = FixedTau::new([(content_slot(), 0.92)]);
    let first = ingest_at(
        &vault,
        &input("near-a", unit_x()),
        EpochSecs(100),
        Some(&guard),
    )
    .expect("near first");
    let second = ingest_at(
        &vault,
        &input("near-b", cos_vector(0.87)),
        EpochSecs(200),
        Some(&guard),
    )
    .expect("near second");
    vault.flush().expect("flush near distinct");
    json!({
        "expected_content_cos": 0.87,
        "calibrated_tau": 0.92,
        "first_result": first,
        "second_result": second,
        "cx_list": cx_list(&vault_dir),
        "raw_base": command_stdout(&["readback", "--cf", "base", "--vault", &display(&vault_dir)]),
        "raw_slot_00": command_stdout(&["readback", "--cf", "slot_00", "--vault", &display(&vault_dir)]),
        "raw_ledger": command_stdout(&["readback", "--cf", "ledger", "--vault", &display(&vault_dir)]),
    })
}

pub(crate) fn anchor_conflict_scenario(root: &Path) -> Value {
    let vault_dir = root.join("anchor_conflict").join("vault");
    let vault = durable_vault(&vault_dir, recurrence_policy(0.90));
    let first = ingest_at(
        &vault,
        &input("speaker-a", unit_x()).with_anchor(speaker("alice")),
        EpochSecs(100),
        None,
    )
    .expect("anchor first");
    let second = ingest_at(
        &vault,
        &input("speaker-b", unit_x()).with_anchor(speaker("bob")),
        EpochSecs(200),
        None,
    )
    .expect("anchor second");
    vault.flush().expect("flush anchor conflict");
    let first_id = new_id(&first);
    let second_id = new_id(&second);
    json!({
        "first_id": first_id,
        "second_id": second_id,
        "first_result": first,
        "second_result": second,
        "cx_list": cx_list(&vault_dir),
        "audit_second": dedup_audit(&vault_dir, second_id),
        "raw_online": command_stdout(&["readback", "--cf", "online", "--vault", &display(&vault_dir)]),
        "raw_base": command_stdout(&["readback", "--cf", "base", "--vault", &display(&vault_dir)]),
    })
}

pub(crate) fn recurring_reversible_scenario(root: &Path) -> Value {
    let vault_dir = root.join("recurring_reversible").join("vault");
    let vault = durable_vault(&vault_dir, recurrence_policy(0.90));
    let first = ingest_at(
        &vault,
        &temporal_input("recurring-a", unit_x(), unit_x()),
        EpochSecs(1000),
        None,
    )
    .expect("recurring first");
    let first_id = new_id(&first);
    let first_base_hex = base_hex_without_frequency(&vault, first_id);
    let second = ingest_at(
        &vault,
        &temporal_input("recurring-b", unit_x(), unit_y()),
        EpochSecs(2000),
        None,
    )
    .expect("recurring second");
    let third = ingest_at(
        &vault,
        &temporal_input("recurring-c", unit_x(), neg_x()),
        EpochSecs(3000),
        None,
    )
    .expect("recurring third");
    vault.flush().expect("flush recurring before undo");

    let audit_before = dedup_audit(&vault_dir, first_id);
    let token = serde_json::to_string(&audit_before["reversal_token"]).expect("token");
    let cx_list_before = cx_list(&vault_dir);
    let mut expected = restore_base_hex_from_ledger(&vault_dir);
    expected.push(json!({"cx_id": first_id, "base_hex": first_base_hex}));
    sort_expected(&mut expected);
    let undo = stdout_json(readback(&[
        "readback",
        "dedup-undo",
        "--vault",
        &display(&vault_dir),
        "--token",
        &token,
    ]));
    let cx_list_after = cx_list(&vault_dir);
    let series_after = recurrence_series(&vault_dir, first_id);
    let audit_after = dedup_audit(&vault_dir, first_id);
    json!({
        "results": [first, second, third],
        "target_id": first_id,
        "cx_list_before": cx_list_before,
        "audit_before": audit_before,
        "expected_base_hex": expected,
        "undo": undo,
        "cx_list_after": cx_list_after,
        "series_after": series_after,
        "audit_after": audit_after,
        "raw_recurrence": command_stdout(&["readback", "--cf", "recurrence", "--vault", &display(&vault_dir)]),
        "raw_ledger_seq3": command_stdout(&["readback", "--cf", "ledger", "--vault", &display(&vault_dir), "--seq", "3"]),
        "verify_chain": command_stdout(&["verify-chain", "--vault", &display(&vault_dir), "--range", "0..4"]),
    })
}

pub(crate) fn temporal_excluded_scenario(root: &Path) -> Value {
    let vault_dir = root.join("temporal_excluded").join("vault");
    let vault = durable_vault(&vault_dir, recurrence_policy(0.90));
    let first = ingest_at(
        &vault,
        &temporal_input("temporal-excluded-a", unit_x(), unit_x()),
        EpochSecs(100),
        None,
    )
    .expect("temporal excluded first");
    let first_id = new_id(&first);
    let second = ingest_at(
        &vault,
        &temporal_input("temporal-excluded-b", cos_vector(0.95), cos_vector(0.30)),
        EpochSecs(200),
        None,
    )
    .expect("temporal excluded second");
    vault.flush().expect("flush temporal excluded");
    json!({
        "expected_content_cos": 0.95,
        "temporal_cos": 0.30,
        "required_slots": [content_slot()],
        "first_result": first,
        "second_result": second,
        "audit": dedup_audit(&vault_dir, first_id),
        "series": recurrence_series(&vault_dir, first_id),
        "raw_ledger": command_stdout(&["readback", "--cf", "ledger", "--vault", &display(&vault_dir)]),
    })
}

pub(crate) fn frequency_count_scenario(root: &Path) -> Value {
    let vault_dir = root.join("frequency_count").join("vault");
    let vault = durable_vault(&vault_dir, recurrence_policy(0.99));
    let mut results = Vec::new();
    let mut target = None;
    for index in 0..10 {
        let result = ingest_at(
            &vault,
            &input("frequency-count", unit_x()),
            EpochSecs(1000 + i64::from(index) * 100),
            None,
        )
        .expect("frequency ingest");
        if let DedupResult::New(cx_id) = result {
            target = Some(cx_id);
        }
        results.push(json!(result));
    }
    vault.flush().expect("flush frequency count");
    let target = target.expect("target id");
    let store = SeriesStore::new(&vault);
    json!({
        "target_id": target,
        "results": results,
        "store_occurrence_count": store.occurrence_count(target).expect("store count"),
        "series": recurrence_series(&vault_dir, target),
        "cx_list": cx_list(&vault_dir),
        "raw_recurrence": command_stdout(&["readback", "--cf", "recurrence", "--vault", &display(&vault_dir)]),
        "raw_base": command_stdout(&["readback", "--cf", "base", "--vault", &display(&vault_dir)]),
        "raw_wal": command_stdout(&["readback", "--wal", "--vault", &display(&vault_dir)]),
    })
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

fn calibrated_policy(action: DedupAction) -> DedupPolicy {
    DedupPolicy::TctCosine(
        TctCosineConfig::new(vec![content_slot()], TauStrategy::Calibrated, action)
            .expect("calibrated policy"),
    )
}

fn recurrence_policy(tau: f32) -> DedupPolicy {
    DedupPolicy::TctCosine(
        TctCosineConfig::new(
            vec![content_slot()],
            TauStrategy::PerSlot(vec![(content_slot(), tau)]),
            DedupAction::RecurrenceSeries,
        )
        .expect("recurrence policy"),
    )
}

fn input(name: &str, content: [f32; 2]) -> IngestInput {
    IngestInput::new(name.as_bytes().to_vec(), 41, Modality::Text).with_slot(
        content_slot(),
        SlotVector::Dense {
            dim: 2,
            data: content.to_vec(),
        },
    )
}

fn temporal_input(name: &str, content: [f32; 2], temporal: [f32; 2]) -> IngestInput {
    input(name, content)
        .with_slot(
            temporal_slot(),
            SlotVector::Dense {
                dim: 2,
                data: temporal.to_vec(),
            },
        )
        .with_temporal_slot(temporal_slot())
}

#[derive(Default)]
struct FixedTau(BTreeMap<SlotId, f32>);

impl FixedTau {
    fn new<const N: usize>(entries: [(SlotId, f32); N]) -> Self {
        Self(entries.into_iter().collect())
    }
}

impl GuardTauProfile for FixedTau {
    fn tau_for(&self, slot: &SlotId) -> Option<f32> {
        self.0.get(slot).copied()
    }
}

fn speaker(name: &str) -> Anchor {
    Anchor {
        kind: AnchorKind::SpeakerMatch,
        value: AnchorValue::Text(name.to_string()),
        source: "synthetic-ph41-invariants".to_string(),
        observed_at: 1_786_406_600,
        confidence: 1.0,
    }
}

fn new_id(result: &DedupResult) -> CxId {
    match result {
        DedupResult::New(id) => *id,
        DedupResult::DedupMerge { .. } | DedupResult::ExactDuplicate(_) => {
            panic!("expected new id")
        }
    }
}

fn base_hex_without_frequency(vault: &AsterVault, cx_id: CxId) -> String {
    let mut cx = vault.get(cx_id, vault.snapshot()).expect("base cx");
    cx.scalars.remove(FREQUENCY_SCALAR);
    hex_bytes(&encode_constellation_base(&cx).expect("base bytes"))
}

fn restore_base_hex_from_ledger(vault_dir: &Path) -> Vec<Value> {
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
    rows
}

fn sort_expected(rows: &mut [Value]) {
    rows.sort_by(|left, right| {
        left["cx_id"]
            .as_str()
            .unwrap()
            .cmp(right["cx_id"].as_str().unwrap())
    });
}

fn cx_list(vault_dir: &Path) -> Value {
    stdout_json(readback(&[
        "readback",
        "cx-list",
        "--vault",
        &display(vault_dir),
    ]))
}

fn dedup_audit(vault_dir: &Path, cx_id: CxId) -> Value {
    stdout_json(readback(&[
        "readback",
        "dedup-audit",
        "--vault",
        &display(vault_dir),
        "--cx-id",
        &cx_id.to_string(),
    ]))
}

fn recurrence_series(vault_dir: &Path, cx_id: CxId) -> Value {
    stdout_json(readback(&[
        "readback",
        "recurrence-series",
        "--vault",
        &display(vault_dir),
        "--cx-id",
        &cx_id.to_string(),
    ]))
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

fn command_stdout(args: &[&str]) -> String {
    stdout(readback(args))
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

fn display(path: &Path) -> String {
    path.display().to_string()
}

fn unit_x() -> [f32; 2] {
    [1.0, 0.0]
}

fn unit_y() -> [f32; 2] {
    [0.0, 1.0]
}

fn neg_x() -> [f32; 2] {
    [-1.0, 0.0]
}

fn cos_vector(cos: f32) -> [f32; 2] {
    [cos, (1.0 - cos * cos).sqrt()]
}

fn content_slot() -> SlotId {
    SlotId::new(0)
}

fn temporal_slot() -> SlotId {
    SlotId::new(20)
}

fn vault_id() -> VaultId {
    VAULT_ID.parse().expect("valid vault id")
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
