use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use calyx_aster::ledger_view::AsterLedgerCfStore;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, CxId, Input, InputRef, LedgerRef, Modality, SlotShape, SlotState,
    SlotVector, VaultId, VaultStore,
};
use calyx_ledger::{
    ActorId, EntryKind, FusionMode, FusionWeights, LedgerCfStore, RecordedSlot, RemeasuredSlot,
    SubjectId, VerifyResult, decode, rerun_fusion, verify_chain,
};
use calyx_registry::{code_default, materialize_panel_template, persist_vault_panel_state};
use serde_json::{Value, json};
use ulid::Ulid;

use super::*;

const RETAINED_INPUT_PREFIX: &str = "calyx-vault://inputs/";

#[test]
fn reproduce_record_appends_row_and_keeps_ledger_chain_intact() {
    let seeded = seed_reproduce_fixture(temp_root("record"), "record-vault");
    let before = decoded_entries(&seeded.path);
    let before_report = reproduce_report(&before, &seeded.answer_id).unwrap_err();
    assert_eq!(before_report.code(), "CALYX_REPRODUCE_NONDETERMINISTIC");

    let report = reproduce_record::record(&seeded.resolved, &seeded.answer_id).unwrap();
    let store = AsterLedgerCfStore::open(&seeded.path).unwrap();
    let rows = store.scan().unwrap();
    let entries = decoded_entries(&seeded.path);
    let payloads = reproduce_payloads(&entries, &seeded.answer_id);
    let verify = verify_chain(&store, 0..rows.len() as u64).unwrap();

    assert!(report.bit_parity);
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0]["type"], REPRODUCE_PAYLOAD_TAG);
    assert_eq!(payloads[0]["reproduced"], true);
    assert_eq!(rows.len(), seeded.before_rows + 1);
    assert_eq!(
        verify,
        VerifyResult::Intact {
            count: rows.len() as u64
        }
    );
    write_fsv(
        &seeded.path,
        seeded.before_rows,
        rows.len(),
        &seeded.answer_id,
        &report,
        &verify,
    );
    fs::remove_dir_all(seeded.root).ok();
}

#[test]
#[ignore = "manual FSV seed for calyx reproduce --record CLI readback"]
fn reproduce_record_cli_manual_fsv_seed() {
    let root = manual_fsv_root();
    let seeded = seed_reproduce_fixture(root, "issue1364-record-cli");
    println!("ISSUE1364_MANUAL_ROOT={}", seeded.root.display());
    println!("ISSUE1364_MANUAL_VAULT={}", seeded.path.display());
    println!(
        "ISSUE1364_MANUAL_ANSWER_ID={}",
        hex_bytes(&seeded.answer_id)
    );
    println!("ISSUE1364_MANUAL_BEFORE_ROWS={}", seeded.before_rows);
}

struct SeededVault {
    root: PathBuf,
    path: PathBuf,
    resolved: ResolvedVault,
    answer_id: Vec<u8>,
    before_rows: usize,
}

fn seed_reproduce_fixture(root: PathBuf, vault_name: &str) -> SeededVault {
    let vault_id = VaultId::from_ulid(Ulid::new());
    let path = root.join("vaults").join(vault_id.to_string());
    let materialized = materialize_panel_template(&code_default(), 42).unwrap();
    let vault = AsterVault::new_durable(
        &path,
        vault_id,
        vault_salt(vault_id, vault_name),
        VaultOptions {
            panel: Some(materialized.panel.clone()),
            ..VaultOptions::default()
        },
    )
    .unwrap();
    persist_vault_panel_state(&path, &materialized.panel, &materialized.registry).unwrap();
    let resolved = ResolvedVault {
        path: path.clone(),
        name: vault_name.to_string(),
        vault_id,
    };

    let slot = materialized
        .panel
        .slots
        .iter()
        .find(|slot| {
            slot.state == SlotState::Active
                && slot.modality == Modality::Code
                && matches!(slot.shape, SlotShape::Dense(_))
                && materialized.registry.contains(slot.lens_id)
        })
        .expect("registered dense code slot");
    let input = retained_input(&path, b"issue1364 retained reproduce bytes");
    let vector = materialized.registry.measure(slot.lens_id, &input).unwrap();
    let cx_id = vault.cx_id_for_input(&input.bytes, materialized.panel.version);
    vault
        .put(Constellation {
            cx_id,
            vault_id,
            panel_version: materialized.panel.version,
            created_at: 43,
            input_ref: InputRef {
                hash: *blake3::hash(&input.bytes).as_bytes(),
                pointer: input.pointer.clone(),
                redacted: false,
            },
            modality: input.modality,
            slots: BTreeMap::from([(slot.slot_id, vector.clone())]),
            scalars: BTreeMap::new(),
            metadata: BTreeMap::new(),
            anchors: Vec::new(),
            provenance: LedgerRef {
                seq: vault.latest_seq().saturating_add(1),
                hash: [0; 32],
            },
            flags: CxFlags::default(),
        })
        .unwrap();

    let recorded = RecordedSlot {
        cx_id,
        slot_id: slot.slot_id,
        lens_id: slot.lens_id,
        weights_sha256: materialized
            .registry
            .frozen_contract(slot.lens_id)
            .unwrap()
            .weights_sha256(),
        input_hash: *blake3::hash(&input.bytes).as_bytes(),
        corpus_shard_hash: None,
        forge_seed: 0x1364_D15C,
        input: None,
    };
    let candidates = dense_candidates(&vector);
    let fusion = FusionWeights {
        mode: FusionMode::SingleLens,
        k: 3,
        candidates,
        weights: Vec::new(),
        single_slot: Some(slot.slot_id),
    };
    let original_hits = rerun_fusion(
        &[RemeasuredSlot {
            cx_id,
            slot_id: slot.slot_id,
            lens_id: slot.lens_id,
            input_hash: recorded.input_hash,
            forge_seed: recorded.forge_seed,
            vector,
        }],
        &fusion,
    )
    .unwrap();
    let measure_ref = append_measure(&vault, &recorded);
    let answer_id = b"issue1364-answer".to_vec();
    append_answer(&vault, &answer_id, measure_ref.seq, &fusion, &original_hits);
    vault.flush().unwrap();
    let before_rows = decoded_entries(&path).len();
    SeededVault {
        root,
        path,
        resolved,
        answer_id,
        before_rows,
    }
}

fn retained_input(vault_path: &std::path::Path, bytes: &[u8]) -> Input {
    let name = format!("{}.bin", hex_bytes(blake3::hash(bytes).as_bytes()));
    let input_dir = vault_path.join("inputs");
    fs::create_dir_all(&input_dir).unwrap();
    fs::write(input_dir.join(&name), bytes).unwrap();
    Input::new(Modality::Code, bytes.to_vec())
        .with_pointer(format!("{RETAINED_INPUT_PREFIX}{name}"))
}

fn dense_candidates(vector: &SlotVector) -> Vec<CxId> {
    let dim = match vector {
        SlotVector::Dense { data, .. } => data.len(),
        _ => panic!("fixture uses dense vector"),
    };
    (1..=dim)
        .map(|value| CxId::from_bytes([value as u8; 16]))
        .collect()
}

fn append_measure(vault: &AsterVault, slot: &RecordedSlot) -> LedgerRef {
    vault
        .append_ledger_entry(
            EntryKind::Measure,
            SubjectId::Cx(slot.cx_id),
            serde_json::to_vec(&json!({
                "cx_id": slot.cx_id.to_string(),
                "slot_id": slot.slot_id.get(),
                "lens_id": slot.lens_id.to_string(),
                "weights_sha256": hex_bytes(&slot.weights_sha256),
                "input_hash": hex_bytes(&slot.input_hash),
                "forge_seed": slot.forge_seed,
            }))
            .unwrap(),
            ActorId::Service("issue1364-test".to_string()),
        )
        .unwrap()
}

fn append_answer(
    vault: &AsterVault,
    answer_id: &[u8],
    measure_seq: u64,
    fusion: &FusionWeights,
    original_hits: &[calyx_ledger::HitRef],
) -> LedgerRef {
    vault
        .append_ledger_entry(
            EntryKind::Answer,
            SubjectId::Query(answer_id.to_vec()),
            serde_json::to_vec(&json!({
                "measure_refs": [measure_seq],
                "fusion_weights": fusion,
                "original_hits": original_hits,
            }))
            .unwrap(),
            ActorId::Service("issue1364-test".to_string()),
        )
        .unwrap()
}

fn decoded_entries(path: &std::path::Path) -> Vec<calyx_ledger::LedgerEntry> {
    AsterLedgerCfStore::open(path)
        .unwrap()
        .scan()
        .unwrap()
        .into_iter()
        .map(|row| decode(&row.bytes).unwrap())
        .collect()
}

fn reproduce_payloads(entries: &[calyx_ledger::LedgerEntry], answer_id: &[u8]) -> Vec<Value> {
    entries
        .iter()
        .filter(|entry| {
            entry.kind == EntryKind::Admin
                && matches!(&entry.subject, SubjectId::Query(id) if id == answer_id)
        })
        .filter_map(|entry| serde_json::from_slice(&entry.payload).ok())
        .filter(|payload: &Value| payload["type"] == REPRODUCE_PAYLOAD_TAG)
        .collect()
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-cli-provenance-{name}-{}-{}",
        std::process::id(),
        crate::cmd::vault::now_ms()
    ))
}

fn manual_fsv_root() -> PathBuf {
    let base = calyx_fsv::fsv_root("CALYX_FSV_ROOT").unwrap_or_else(|| temp_root("manual-record"));
    base.join(format!(
        "issue1364-reproduce-record-cli-{}",
        crate::cmd::vault::now_ms()
    ))
}

fn write_fsv(
    vault: &std::path::Path,
    before_rows: usize,
    after_rows: usize,
    answer_id: &[u8],
    report: &ReproduceOut,
    verify: &VerifyResult,
) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    fs::create_dir_all(&root).unwrap();
    let payload = json!({
        "source_of_truth": "Aster durable Ledger CF read through AsterLedgerCfStore",
        "vault": vault.display().to_string(),
        "answer_id": hex_bytes(answer_id),
        "before_rows": before_rows,
        "after_rows": after_rows,
        "appended_rows": after_rows.saturating_sub(before_rows),
        "bit_parity": report.bit_parity,
        "original_hash": &report.original_hash,
        "reproduced_hash": &report.reproduced_hash,
        "verify_chain": format!("{verify:?}"),
    });
    fs::write(
        root.join("issue1364-reproduce-record-ledger-readback.json"),
        serde_json::to_vec_pretty(&payload).unwrap(),
    )
    .unwrap();
}
